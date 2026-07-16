use color_eyre::eyre::Result;

use crate::plumber::{self, Matcher, MatcherBuilder};

use super::{
    cli::{Cli, CliCommand},
    keymap::{self, KeyBindings},
};

/// Load the command rules to *start* this invocation with. For a normal run,
/// malformed files are skipped and returned as warnings so a single bad config
/// cannot prevent the TUI from starting; `list-commands` loads strictly, since
/// it is the config validation tool.
///
/// Startup is lenient because the alternative is no Kinjo at all: a stale file
/// in a shared directory nobody on this machine can edit must not be able to
/// keep the app from running. See [`reload_matcher`] for why a *reload* cannot
/// afford the same generosity.
pub fn load_matcher(cli: &Cli) -> Result<(Matcher, Vec<String>)> {
    let mut builder = MatcherBuilder::new();

    let dirs = matcher_config_dirs(cli);
    let warnings = if cli.command == CliCommand::ListCommands {
        plumber::load_from_dirs(&mut builder, &dirs)?;
        Vec::new()
    } else {
        plumber::load_from_dirs_lenient(&mut builder, &dirs)
    };
    Ok((builder.build(), warnings))
}

/// Load a *replacement* rule set for a live reload (SIGHUP), transactionally:
/// either the whole configured overlay is valid and a complete [`Matcher`] is
/// returned, or nothing is — and the diagnostics say why, one per invalid file
/// or unreadable directory, each naming its source.
///
/// Reload is all-or-nothing where startup is lenient, because the two are not
/// the same situation. At startup, skipping a bad file is the difference
/// between a partly working Kinjo and none. At reload there is already a
/// working rule set in force, so skipping a bad file instead trades a *complete*
/// rule set for a partial one — a command silently disappearing mid-session
/// because of a half-saved edit, which is worse than the edit simply not taking
/// effect yet.
///
/// The overlay is compiled through the same validator either policy uses, and
/// the candidate matcher is built here, before the caller can install it, so a
/// rejected reload never touches the rules in force.
pub fn reload_matcher(cli: &Cli) -> std::result::Result<Matcher, Vec<String>> {
    reload_matcher_from(&matcher_config_dirs(cli))
}

/// The transactional policy itself, over an explicit overlay. Split out from
/// [`reload_matcher`] so it can be unit tested against real directories without
/// depending on the machine's system and user config directories.
fn reload_matcher_from(dirs: &[std::path::PathBuf]) -> std::result::Result<Matcher, Vec<String>> {
    let mut builder = MatcherBuilder::new();
    let diagnostics = plumber::load_from_dirs_lenient(&mut builder, dirs);
    if diagnostics.is_empty() {
        Ok(builder.build())
    } else {
        // Nothing is built at all: the caller cannot install a partial overlay
        // it was never given.
        Err(diagnostics)
    }
}

fn matcher_config_dirs(cli: &Cli) -> Vec<std::path::PathBuf> {
    if cli.command == CliCommand::ListCommands && !cli.config_dirs.is_empty() {
        cli.config_dirs.clone()
    } else {
        plumber::config_dirs(&cli.config_dirs)
    }
}

pub fn load_keybindings() -> Result<KeyBindings> {
    KeyBindings::load(&keymap::default_config_paths())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;
    use crate::test_support::{remove, temp_dir};
    use crate::ui::keymap::{Action, Mode};

    fn test_cli(command: CliCommand, config_dirs: Vec<PathBuf>) -> Cli {
        Cli {
            domain: "local".to_string(),
            config_dirs,
            service_type: None,
            backend: crate::discovery::DiscoveryBackend::default(),
            command,
        }
    }

    fn write_command(dir: &std::path::Path, file: &str, name: &str, command: &str) {
        fs::write(
            dir.join(file),
            format!(
                r#"
[metadata]
name = "{name}"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "{command}"
mode = "execute"
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn run_command_expands_default_dirs_before_extra_config_dirs() {
        let extra = PathBuf::from("/tmp/kinjo-extra-commands");
        let dirs = matcher_config_dirs(&test_cli(CliCommand::Run, vec![extra.clone()]));

        assert!(dirs.contains(&PathBuf::from(plumber::SYSTEM_CONFIG_DIR)));
        assert_eq!(dirs.last(), Some(&extra));
    }

    #[test]
    fn list_commands_with_explicit_dirs_does_not_expand_defaults() {
        let explicit = vec![
            PathBuf::from("/tmp/kinjo-list-a"),
            PathBuf::from("/tmp/kinjo-list-b"),
        ];
        let dirs = matcher_config_dirs(&test_cli(CliCommand::ListCommands, explicit.clone()));

        assert_eq!(dirs, explicit);
    }

    #[test]
    fn list_commands_with_explicit_dirs_uses_them_as_overlays() {
        let base = temp_dir("list-base");
        let overlay = temp_dir("list-overlay");
        write_command(&base, "ssh.toml", "ssh", "ssh base");
        write_command(&base, "mosh.toml", "mosh", "mosh base");
        write_command(&overlay, "ssh.toml", "ssh", "ssh overlay");

        let (matcher, warnings) = load_matcher(&test_cli(
            CliCommand::ListCommands,
            vec![base.clone(), overlay.clone()],
        ))
        .unwrap();
        assert!(warnings.is_empty());

        assert_eq!(matcher.command_count(), 2);
        assert_eq!(matcher.commands()[0].name, "mosh");
        assert_eq!(matcher.commands()[1].name, "ssh");
        assert_eq!(matcher.commands()[1].action.command, "ssh overlay");

        remove(&base);
        remove(&overlay);
    }

    fn write_invalid_command(dir: &std::path::Path, file: &str) {
        // Valid TOML, impossible rule: the case a plain syntax check misses.
        fs::write(
            dir.join(file),
            r#"
[metadata]
name = "broken"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "echo {nonexistent}"
mode = "fork"
"#,
        )
        .unwrap();
    }

    /// Startup must stay lenient: one bad file in a shared directory cannot be
    /// allowed to stop Kinjo launching, so the good rules still load and the
    /// bad file is reported by name.
    #[test]
    fn startup_load_keeps_valid_commands_and_warns_about_invalid_ones() {
        let dir = temp_dir("startup-lenient");
        write_command(&dir, "ssh.toml", "ssh", "ssh {hostname}");
        write_invalid_command(&dir, "broken.toml");

        let (matcher, warnings) = load_matcher(&test_cli(CliCommand::Run, vec![dir.clone()]))
            .expect("a bad file must not stop startup");

        assert!(matcher.commands().iter().any(|c| c.name == "ssh"));
        assert!(
            warnings.iter().any(|w| w.contains("broken.toml")),
            "{warnings:?}"
        );

        remove(&dir);
    }

    /// The reload counterpart of the test above, and the difference that matters:
    /// the same overlay that startup loads partially is refused outright, because
    /// a reload would be replacing a rule set that already works.
    #[test]
    fn reload_refuses_an_overlay_with_any_invalid_file() {
        let dir = temp_dir("reload-mixed");
        write_command(&dir, "ssh.toml", "ssh", "ssh {hostname}");
        write_invalid_command(&dir, "broken.toml");

        let diagnostics =
            reload_matcher_from(std::slice::from_ref(&dir)).expect_err("no partial swap");

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("broken.toml"), "{diagnostics:?}");
        assert!(
            diagnostics[0].contains("unknown service field"),
            "the message keeps the reason, not just the path: {diagnostics:?}"
        );

        remove(&dir);
    }

    #[test]
    fn reload_accepts_a_fully_valid_overlay_in_precedence_order() {
        let base = temp_dir("reload-base");
        let overlay = temp_dir("reload-overlay");
        write_command(&base, "ssh.toml", "ssh", "ssh base");
        write_command(&overlay, "ssh.toml", "ssh", "ssh overlay");
        write_command(&overlay, "mosh.toml", "mosh", "mosh overlay");

        let matcher = reload_matcher_from(&[base.clone(), overlay.clone()]).expect("all valid");

        assert_eq!(matcher.command_count(), 2);
        assert_eq!(matcher.commands()[0].action.command, "ssh overlay");

        remove(&base);
        remove(&overlay);
    }

    /// Every invalid file is named, not just the first: a reload tells the user
    /// everything they have to fix before it will take effect.
    #[test]
    fn a_rejected_reload_reports_every_invalid_file() {
        let dir = temp_dir("reload-many-invalid");
        write_command(&dir, "good.toml", "good", "true");
        write_invalid_command(&dir, "a-broken.toml");
        fs::write(dir.join("b-broken.toml"), "not toml at all [").unwrap();

        let diagnostics = reload_matcher_from(std::slice::from_ref(&dir)).expect_err("rejected");

        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(diagnostics.iter().any(|d| d.contains("a-broken.toml")));
        assert!(diagnostics.iter().any(|d| d.contains("b-broken.toml")));

        remove(&dir);
    }

    /// A missing directory is not an invalid one. Most of the overlay normally
    /// does not exist, so a reload that treated absence as failure could never
    /// succeed on a default install.
    #[test]
    fn reload_ignores_directories_that_do_not_exist() {
        let dir = temp_dir("reload-present");
        write_command(&dir, "ssh.toml", "ssh", "ssh {hostname}");

        let matcher =
            reload_matcher_from(&[PathBuf::from("/tmp/kinjo-no-such-dir-xyz"), dir.clone()])
                .expect("absent directories are simply empty layers");

        assert_eq!(matcher.command_count(), 1);

        remove(&dir);
    }

    #[test]
    fn load_keybindings_falls_back_to_defaults_when_no_files_exist() {
        let bindings = KeyBindings::load(&[PathBuf::from("/tmp/kinjo-no-such-keymap")]).unwrap();

        assert_eq!(
            bindings.resolve(
                Mode::Browse,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('q'),
                    crossterm::event::KeyModifiers::NONE,
                )
            ),
            Some(Action::BrowseQuit)
        );
    }
}
