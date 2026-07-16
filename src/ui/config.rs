use color_eyre::eyre::Result;

use crate::plumber::{self, Matcher, MatcherBuilder};

use super::{
    cli::{Cli, CliCommand},
    keymap::{self, KeyBindings},
};

/// Load the command rules for this invocation. For a normal run, malformed
/// files are skipped and returned as warnings so a single bad config cannot
/// prevent the TUI from starting; `list-commands` loads strictly, since it is
/// the config validation tool.
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

    /// The overlay order that reaches the loader is the command-line order of
    /// the `--config-dir` occurrences, regardless of which side of the
    /// `list-commands` name each was written on.
    ///
    /// This proves order rather than membership: both directories define `ssh`,
    /// so only the directory that overlays *last* can supply the surviving
    /// rule. `base` also defines a rule `overlay` does not, so a placement that
    /// dropped either directory would lose a rule instead of merely reordering
    /// one — the count and the winner are checked together. Directories are
    /// real and the argv is parsed by the real CLI, so nothing here can agree
    /// with a broken parser.
    #[test]
    fn list_commands_overlay_order_follows_command_line_order_not_placement() {
        let base = temp_dir("overlay-order-base");
        let overlay = temp_dir("overlay-order-overlay");
        write_command(&base, "ssh.toml", "ssh", "ssh base");
        write_command(&base, "mosh.toml", "mosh", "mosh base");
        write_command(&overlay, "ssh.toml", "ssh", "ssh overlay");

        // Build `kinjo [before...] list-commands [after...]`, so each case below
        // states only where its directories were written.
        let argv_of = |before: &[&std::path::Path], after: &[&std::path::Path]| {
            let flagged = |dirs: &[&std::path::Path]| -> Vec<String> {
                dirs.iter()
                    .flat_map(|dir| ["--config-dir".to_string(), dir.display().to_string()])
                    .collect()
            };

            let mut argv = vec!["kinjo".to_string()];
            argv.extend(flagged(before));
            argv.push("list-commands".to_string());
            argv.extend(flagged(after));
            argv
        };

        let (base, overlay) = (base.as_path(), overlay.as_path());
        // Every case writes `base` before `overlay` on the command line, across
        // all three placements, so `overlay` must win in each. Reversing the
        // command line must reverse the winner — that is what makes these
        // assertions statements about order rather than membership.
        let cases = [
            (argv_of(&[base, overlay], &[]), "ssh overlay"),
            (argv_of(&[], &[base, overlay]), "ssh overlay"),
            (argv_of(&[base], &[overlay]), "ssh overlay"),
            (argv_of(&[overlay, base], &[]), "ssh base"),
            (argv_of(&[], &[overlay, base]), "ssh base"),
            (argv_of(&[overlay], &[base]), "ssh base"),
        ];

        for (argv, expected) in &cases {
            let cli = super::super::cli::parse_from(argv.clone()).unwrap();
            let (matcher, warnings) = load_matcher(&cli).unwrap();

            assert!(warnings.is_empty(), "{argv:?}");
            // Both directories were loaded: `mosh` exists only in `base`, and
            // `ssh` collapses to one rule however many layers define it.
            assert_eq!(matcher.command_count(), 2, "{argv:?}");
            let ssh = matcher
                .commands()
                .iter()
                .find(|command| command.name == "ssh")
                .unwrap_or_else(|| panic!("{argv:?}: no `ssh` rule"));
            assert_eq!(ssh.action.command, *expected, "{argv:?}");
        }

        remove(base);
        remove(overlay);
    }

    /// The placement fix must not have quietly turned a run's extra directories
    /// into a replacement for the defaults: a run still layers defaults first.
    #[test]
    fn run_config_dirs_parsed_from_argv_still_overlay_the_defaults() {
        let extra = temp_dir("run-extra-commands");
        let dirs = matcher_config_dirs(
            &super::super::cli::parse_from(["kinjo", "--config-dir", extra.to_str().unwrap()])
                .unwrap(),
        );

        assert!(dirs.contains(&PathBuf::from(plumber::SYSTEM_CONFIG_DIR)));
        assert_eq!(dirs.last(), Some(&extra));

        remove(&extra);
    }

    /// With no explicit directory, `list-commands` keeps expanding the normal
    /// defaults — the fix changes where the flag may be written, not what its
    /// absence means.
    #[test]
    fn list_commands_without_config_dirs_still_expands_the_defaults() {
        let cli = super::super::cli::parse_from(["kinjo", "list-commands"]).unwrap();

        let dirs = matcher_config_dirs(&cli);

        assert_eq!(dirs, plumber::config_dirs(&[]));
        assert!(dirs.contains(&PathBuf::from(plumber::SYSTEM_CONFIG_DIR)));
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
