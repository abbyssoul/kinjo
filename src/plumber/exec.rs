//! Running a prepared command, and the dependencies a rule declares.
//!
//! Tokenizing and interpolating command templates lives in [`super::template`];
//! by the time anything here runs, the argument vector is already decided.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use color_eyre::eyre::{Result, eyre};

use super::ActionMode;

/// The final argument vector and the mode deciding how it reaches the operating
/// system. Produced by [`super::CommandAction::prepare`].
///
/// Equality is the rule's *observable* execution: two prepared commands that
/// compare equal run the identical program with the identical arguments in the
/// identical way, so a user has nothing to choose between them. That is what
/// makes them collapsible, and what makes anything else worth asking about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreparedCommand {
    pub argv: Vec<String>,
    pub mode: ActionMode,
}

pub(super) fn fork(command: &PreparedCommand) -> Result<()> {
    let (program, args) = command_parts(command)?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| spawn_error(program, err))?;
    // Reap the child once it exits so it does not linger as a zombie for the
    // lifetime of the TUI. The thread parks in `wait` and goes away with the
    // child; if the TUI exits first, orphans are re-parented and reaped by init.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn exec(command: PreparedCommand) -> Result<()> {
    let (program, args) = command_parts(&command)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // `exec` only returns when the hand-off fails; on success the current
        // process image is replaced and control never comes back.
        let err = Command::new(program).args(args).exec();
        Err(spawn_error(program, err))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new(program)
            .args(args)
            .status()
            .map_err(|err| spawn_error(program, err))?;
        if status.success() {
            Ok(())
        } else {
            Err(eyre!("process exited with status {status}"))
        }
    }
}

fn command_parts(command: &PreparedCommand) -> Result<(&str, &[String])> {
    let Some((program, args)) = command.argv.split_first() else {
        return Err(eyre!("prepared command has an empty argument vector"));
    };
    if program.is_empty() {
        return Err(eyre!("prepared command has an empty program name"));
    }
    Ok((program, args))
}

/// Turn a spawn/exec failure into a user-facing message, special-casing the
/// common "binary not on PATH" case so the report is actionable rather than a
/// bare OS error code.
fn spawn_error(program: &str, err: std::io::Error) -> color_eyre::eyre::Report {
    if err.kind() == std::io::ErrorKind::NotFound {
        eyre!("command `{program}` not found")
    } else {
        eyre!("could not start `{program}`: {err}")
    }
}

/// A validated dependency of a command rule, parsed from a `requirements` entry
/// such as `"xdg-open"` or `"browser, optional"`.
///
/// Parsed once at load time: a rule holds `Requirement`s, never raw strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// The program to look for. Never empty.
    pub command: String,
    /// Whether the action still runs when `command` is absent.
    pub optional: bool,
}

impl Requirement {
    /// Parse one `requirements` entry.
    ///
    /// The grammar, after trimming, is exactly `<program>` or
    /// `<program>, optional`. Everything else is rejected rather than
    /// interpreted generously: under the old lenient parse an entry like
    /// `"browser, optinal"` silently meant *mandatory*, so a typo quietly turned
    /// an optional dependency into one that blocks the action.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        let mut parts = trimmed.split(',');
        let command = parts.next().unwrap_or_default().trim();
        if command.is_empty() {
            return Err(eyre!("requirement `{raw}` has an empty program name"));
        }
        let Some(suffix) = parts.next() else {
            return Ok(Self {
                command: command.to_string(),
                optional: false,
            });
        };
        if parts.next().is_some() {
            return Err(eyre!(
                "requirement `{raw}` has more than one `,`; \
                 write `<program>` or `<program>, optional`"
            ));
        }
        if !suffix.trim().eq_ignore_ascii_case("optional") {
            return Err(eyre!(
                "requirement `{raw}` has an unsupported suffix `{}`; \
                 the only supported suffix is `, optional`",
                suffix.trim()
            ));
        }
        Ok(Self {
            command: command.to_string(),
            optional: true,
        })
    }
}

impl std::fmt::Display for Requirement {
    /// Renders back into the grammar it was parsed from, so the UI can show a
    /// rule's dependencies without keeping the raw text alongside them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.command)?;
        if self.optional {
            f.write_str(", optional")?;
        }
        Ok(())
    }
}

/// Name of the first mandatory requirement that cannot be resolved on `PATH`, if
/// any. Optional requirements are never reported: their absence is expected.
pub(super) fn missing_requirement(requirements: &[Requirement]) -> Option<&str> {
    requirements
        .iter()
        .filter(|requirement| !requirement.optional)
        .find(|requirement| locate(&requirement.command).is_none())
        .map(|requirement| requirement.command.as_str())
}

/// Resolve `program` the way the OS would when spawning it: an explicit path
/// (containing a separator) is checked directly, otherwise each `PATH` entry is
/// tried. Returns the resolved path when it names an executable file.
pub fn locate(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.chars().any(std::path::is_separator) {
        let path = PathBuf::from(program);
        return is_executable(&path).then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .flat_map(|dir| candidates_in(&dir, program))
        .find(|candidate| is_executable(candidate))
}

/// The paths under `dir` that could resolve `program`: the bare name, plus — on
/// Windows — the name with each `PATHEXT` extension appended, mirroring how
/// `CreateProcess` resolves `ping` to `ping.exe`.
#[cfg(windows)]
fn candidates_in(dir: &Path, program: &str) -> Vec<PathBuf> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut candidates = vec![dir.join(program)];
    candidates.extend(
        pathext
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(|ext| dir.join(format!("{program}{ext}"))),
    );
    candidates
}

#[cfg(not(windows))]
fn candidates_in(dir: &Path, program: &str) -> Vec<PathBuf> {
    vec![dir.join(program)]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_a_bare_program_and_the_optional_suffix() {
        assert_eq!(
            Requirement::parse("xdg-open").unwrap(),
            Requirement {
                command: "xdg-open".to_string(),
                optional: false,
            }
        );
        // Surrounding and inner whitespace is trimmed, and the marker is
        // case-insensitive.
        assert_eq!(
            Requirement::parse("  browser ,  Optional ").unwrap(),
            Requirement {
                command: "browser".to_string(),
                optional: true,
            }
        );
    }

    #[test]
    fn parse_rejects_every_other_suffix_and_shape() {
        // A trailing word that is not the marker used to be silently ignored,
        // leaving the requirement mandatory. It is now a configuration error.
        for raw in ["foo, please", "foo, optionally", "foo, optional!", "foo,"] {
            assert!(Requirement::parse(raw).is_err(), "`{raw}` must be rejected");
        }
        assert!(
            Requirement::parse("foo, optional, bar")
                .unwrap_err()
                .to_string()
                .contains("more than one `,`")
        );
        assert!(
            Requirement::parse("foo,,optional")
                .unwrap_err()
                .to_string()
                .contains("more than one `,`")
        );
        assert!(
            Requirement::parse("foo, please")
                .unwrap_err()
                .to_string()
                .contains("unsupported suffix `please`")
        );
    }

    #[test]
    fn parse_rejects_an_empty_program_name() {
        for raw in ["", "   ", ",", ", optional", "  , optional"] {
            assert!(
                Requirement::parse(raw)
                    .unwrap_err()
                    .to_string()
                    .contains("empty program name"),
                "`{raw}` must be rejected"
            );
        }
    }

    #[test]
    fn display_round_trips_the_requirement_grammar() {
        for raw in ["xdg-open", "browser, optional"] {
            assert_eq!(Requirement::parse(raw).unwrap().to_string(), raw);
        }
    }

    /// A command guaranteed to be on `PATH` for the current platform.
    #[cfg(windows)]
    const PRESENT_COMMAND: &str = "cmd.exe";
    #[cfg(not(windows))]
    const PRESENT_COMMAND: &str = "sh";

    fn requirements(raw: &[&str]) -> Vec<Requirement> {
        raw.iter()
            .map(|entry| Requirement::parse(entry).expect("valid requirement"))
            .collect()
    }

    #[test]
    fn locate_resolves_absolute_paths_and_path_lookups() {
        // The running test binary is an executable file at a known absolute path.
        let exe = std::env::current_exe().unwrap();
        assert!(locate(exe.to_str().unwrap()).is_some());

        // A shell interpreter is present on every supported platform.
        assert!(locate(PRESENT_COMMAND).is_some());

        assert!(locate("kinjo-no-such-binary-xyz").is_none());
        assert!(locate("/no/such/absolute/path/xyz").is_none());
        assert!(locate("").is_none());
    }

    /// Windows resolves bare names through `PATHEXT`; a requirement written as
    /// `cmd` (no extension) must still be found.
    #[cfg(windows)]
    #[test]
    fn locate_resolves_bare_names_via_pathext() {
        assert!(locate("cmd").is_some());
    }

    #[test]
    fn missing_requirement_skips_optional_and_present_commands() {
        assert_eq!(missing_requirement(&[]), None);
        assert_eq!(missing_requirement(&requirements(&[PRESENT_COMMAND])), None);
        assert_eq!(
            missing_requirement(&requirements(&["definitely-absent-xyz, optional"])),
            None
        );
        assert_eq!(
            missing_requirement(&requirements(&[PRESENT_COMMAND, "definitely-absent-xyz"])),
            Some("definitely-absent-xyz")
        );
    }

    #[test]
    fn fork_reports_a_missing_binary() {
        let command = PreparedCommand {
            argv: vec!["kinjo-no-such-binary-xyz".to_string()],
            mode: ActionMode::Fork,
        };

        let err = fork(&command).unwrap_err();
        assert!(
            err.to_string()
                .contains("command `kinjo-no-such-binary-xyz` not found")
        );
    }

    #[test]
    fn public_execution_rejects_an_empty_prepared_command_without_panicking() {
        let empty = PreparedCommand {
            argv: Vec::new(),
            mode: ActionMode::Execute,
        };
        assert!(
            exec(empty)
                .unwrap_err()
                .to_string()
                .contains("empty argument vector")
        );

        let empty_program = PreparedCommand {
            argv: vec![String::new()],
            mode: ActionMode::Fork,
        };
        assert!(
            fork(&empty_program)
                .unwrap_err()
                .to_string()
                .contains("empty program name")
        );
    }
}
