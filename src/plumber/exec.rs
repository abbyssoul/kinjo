use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use color_eyre::eyre::{Result, eyre};

use crate::discovery::Entry;

use super::{ActionMode, CommandAction};

#[derive(Debug, Clone)]
pub struct PreparedCommand {
    pub argv: Vec<String>,
    pub mode: ActionMode,
}

pub fn prepare(action: &CommandAction, record: &Entry) -> Result<PreparedCommand> {
    let expanded = interpolate(&action.command, record)?;
    let argv = split_command_line(&expanded)?;
    if argv.is_empty() {
        return Err(eyre!("action command expanded to an empty argv"));
    }
    Ok(PreparedCommand {
        argv,
        mode: action.mode,
    })
}

pub fn fork(command: &PreparedCommand) -> Result<()> {
    Command::new(&command.argv[0])
        .args(&command.argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| spawn_error(&command.argv[0], err))?;
    Ok(())
}

pub fn exec(command: PreparedCommand) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // `exec` only returns when the hand-off fails; on success the current
        // process image is replaced and control never comes back.
        let err = Command::new(&command.argv[0])
            .args(&command.argv[1..])
            .exec();
        Err(spawn_error(&command.argv[0], err))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new(&command.argv[0])
            .args(&command.argv[1..])
            .status()
            .map_err(|err| spawn_error(&command.argv[0], err))?;
        if status.success() {
            Ok(())
        } else {
            Err(eyre!("process exited with status {status}"))
        }
    }
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

/// A declared dependency of a command, parsed from a `requirements` entry such
/// as `"xdg-open"` or `"browser, optional"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub command: String,
    pub optional: bool,
}

/// Parse a single `requirements` entry. The optional `, optional` suffix marks a
/// dependency whose absence should not block the action.
pub fn parse_requirement(raw: &str) -> Requirement {
    let mut parts = raw.splitn(2, ',');
    let command = parts.next().unwrap_or("").trim().to_string();
    let optional = parts
        .next()
        .is_some_and(|rest| rest.trim().eq_ignore_ascii_case("optional"));
    Requirement { command, optional }
}

/// Name of the first mandatory requirement that cannot be resolved on `PATH`,
/// if any. Optional and blank requirements are ignored.
pub fn missing_requirement(requirements: &[String]) -> Option<String> {
    requirements
        .iter()
        .map(|raw| parse_requirement(raw))
        .filter(|req| !req.optional && !req.command.is_empty())
        .find(|req| locate(&req.command).is_none())
        .map(|req| req.command)
}

/// Resolve `program` the way the OS would when spawning it: an explicit path
/// (containing a separator) is checked directly, otherwise each `PATH` entry is
/// tried. Returns the resolved path when it names an executable file.
pub fn locate(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains('/') {
        let path = PathBuf::from(program);
        return is_executable(&path).then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
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

fn interpolate(template: &str, record: &Entry) -> Result<String> {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            output.push(ch);
            continue;
        }
        let mut field = String::new();
        loop {
            let Some(next) = chars.next() else {
                return Err(eyre!("unterminated interpolation in `{template}`"));
            };
            if next == '}' {
                break;
            }
            field.push(next);
        }
        let Some(value) = record.field(&field) else {
            return Err(eyre!(
                "service field `{field}` is unavailable for `{}`",
                record.name
            ));
        };
        output.push_str(&value);
    }
    Ok(output)
}

fn split_command_line(command: &str) -> Result<Vec<String>> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => quote = Some(ch),
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    argv.push(std::mem::take(&mut current));
                }
            }
            (None, c) => current.push(c),
        }
    }

    if let Some(q) = quote {
        return Err(eyre!("unterminated `{q}` quote in command"));
    }
    if !current.is_empty() {
        argv.push(current);
    }
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plumber::ActionMode;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn interpolates_and_splits() {
        let mut record = Entry::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        let action = CommandAction {
            description: None,
            command: "ssh '{hostname}'".to_string(),
            mode: ActionMode::Execute,
        };
        let prepared = prepare(&action, &record).unwrap();
        assert_eq!(prepared.argv, vec!["ssh", "alpha.local"]);
    }

    #[test]
    fn prepares_all_supported_service_fields() {
        let mut record = Entry::new("Kitchen Printer", "_ipp._tcp", "local");
        record.hostname = Some("printer.local".to_string());
        record.addresses = vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 20))];
        record.port = Some(631);
        record
            .txt
            .insert("path".to_string(), "/ipp/print".to_string());
        let action = CommandAction {
            description: None,
            command: "open '{name}' {service_type} {domain} {hostname} {address} {port} {txt.path}"
                .to_string(),
            mode: ActionMode::Fork,
        };

        let prepared = prepare(&action, &record).unwrap();

        assert_eq!(prepared.mode, ActionMode::Fork);
        assert_eq!(
            prepared.argv,
            vec![
                "open",
                "Kitchen Printer",
                "_ipp._tcp",
                "local",
                "printer.local",
                "192.0.2.20",
                "631",
                "/ipp/print",
            ]
        );
    }

    #[test]
    fn splits_quoted_and_escaped_arguments() {
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: r#"printf "two words" one\ arg 'single quoted' "\\""#.to_string(),
            mode: ActionMode::Execute,
        };

        let prepared = prepare(&action, &record).unwrap();

        assert_eq!(
            prepared.argv,
            vec!["printf", "two words", "one arg", "single quoted", "\\"]
        );
    }

    #[test]
    fn missing_interpolation_field_is_an_error() {
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "ssh {hostname}".to_string(),
            mode: ActionMode::Execute,
        };

        let err = prepare(&action, &record).unwrap_err();

        assert!(err.to_string().contains("service field `hostname`"));
        assert!(err.to_string().contains("alpha"));
    }

    #[test]
    fn malformed_templates_and_quotes_are_errors() {
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let unterminated_interpolation = CommandAction {
            description: None,
            command: "echo {name".to_string(),
            mode: ActionMode::Execute,
        };
        let unterminated_quote = CommandAction {
            description: None,
            command: "echo 'alpha".to_string(),
            mode: ActionMode::Execute,
        };

        assert!(
            prepare(&unterminated_interpolation, &record)
                .unwrap_err()
                .to_string()
                .contains("unterminated interpolation")
        );
        assert!(
            prepare(&unterminated_quote, &record)
                .unwrap_err()
                .to_string()
                .contains("unterminated `'` quote")
        );
    }

    #[test]
    fn fork_spawns_a_real_process() {
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "true".to_string(),
            mode: ActionMode::Fork,
        };

        let prepared = prepare(&action, &record).unwrap();
        assert_eq!(prepared.mode, ActionMode::Fork);
        // `true` exits 0 immediately; forking it should succeed without error.
        fork(&prepared).unwrap();
    }

    #[test]
    fn fork_reports_a_missing_binary() {
        let command = PreparedCommand {
            argv: vec!["avahi-tui-no-such-binary-xyz".to_string()],
            mode: ActionMode::Fork,
        };

        let err = fork(&command).unwrap_err();
        assert!(
            err.to_string()
                .contains("command `avahi-tui-no-such-binary-xyz` not found")
        );
    }

    #[test]
    fn interpolates_txt_record_fields() {
        let mut record = Entry::new("nas", "_http._tcp", "local");
        record.hostname = Some("nas.local".to_string());
        record.txt.insert("path".to_string(), "/admin".to_string());
        let action = CommandAction {
            description: None,
            command: "xdg-open http://{hostname}{txt.path}".to_string(),
            mode: ActionMode::Fork,
        };

        let prepared = prepare(&action, &record).unwrap();

        assert_eq!(prepared.argv, vec!["xdg-open", "http://nas.local/admin"]);
    }

    #[test]
    fn missing_txt_field_is_an_error() {
        let record = Entry::new("nas", "_http._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "echo {txt.path}".to_string(),
            mode: ActionMode::Fork,
        };

        let err = prepare(&action, &record).unwrap_err();

        assert!(err.to_string().contains("service field `txt.path`"));
    }

    #[test]
    fn empty_command_after_splitting_is_an_error() {
        let record = Entry::new("alpha", "_ssh._tcp", "local");
        let action = CommandAction {
            description: None,
            command: "   ".to_string(),
            mode: ActionMode::Execute,
        };

        let err = prepare(&action, &record).unwrap_err();

        assert!(err.to_string().contains("empty argv"));
    }

    #[test]
    fn parse_requirement_detects_the_optional_marker() {
        assert_eq!(
            parse_requirement("xdg-open"),
            Requirement {
                command: "xdg-open".to_string(),
                optional: false,
            }
        );
        assert_eq!(
            parse_requirement("  browser ,  Optional "),
            Requirement {
                command: "browser".to_string(),
                optional: true,
            }
        );
        // A trailing word other than `optional` is not treated as the marker.
        assert!(!parse_requirement("foo, please").optional);
    }

    #[test]
    fn locate_resolves_absolute_paths_and_path_lookups() {
        // The running test binary is an executable file at a known absolute path.
        let exe = std::env::current_exe().unwrap();
        assert!(locate(exe.to_str().unwrap()).is_some());

        // `sh` is present on every supported (unix) platform.
        assert!(locate("sh").is_some());

        assert!(locate("avahi-tui-no-such-binary-xyz").is_none());
        assert!(locate("/no/such/absolute/path/xyz").is_none());
        assert!(locate("").is_none());
    }

    #[test]
    fn missing_requirement_skips_optional_and_present_commands() {
        assert_eq!(missing_requirement(&[]), None);
        assert_eq!(missing_requirement(&["sh".to_string()]), None);
        assert_eq!(
            missing_requirement(&["definitely-absent-xyz, optional".to_string()]),
            None
        );
        assert_eq!(
            missing_requirement(&["sh".to_string(), "definitely-absent-xyz".to_string()]),
            Some("definitely-absent-xyz".to_string())
        );
    }
}
