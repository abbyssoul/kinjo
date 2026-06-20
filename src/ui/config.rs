use color_eyre::eyre::Result;

use crate::plumber::{self, Matcher, MatcherBuilder};

use super::{
    cli::{Cli, CliCommand},
    keymap::{self, KeyBindings},
};

pub fn load_matcher(cli: &Cli) -> Result<Matcher> {
    let mut builder = MatcherBuilder::new();

    let dirs = matcher_config_dirs(cli);
    plumber::load_from_dirs(&mut builder, &dirs)?;
    Ok(builder.build())
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

    fn test_cli(command: CliCommand, config_dirs: Vec<PathBuf>) -> Cli {
        Cli {
            domain: "local".to_string(),
            config_dirs,
            service_type: None,
            fake_discovery: false,
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
        let extra = PathBuf::from("/tmp/avahi-tui-extra-commands");
        let dirs = matcher_config_dirs(&test_cli(CliCommand::Run, vec![extra.clone()]));

        assert_eq!(
            dirs.first(),
            Some(&PathBuf::from(plumber::SYSTEM_CONFIG_DIR))
        );
        assert_eq!(dirs.last(), Some(&extra));
    }

    #[test]
    fn list_commands_with_explicit_dirs_does_not_expand_defaults() {
        let explicit = vec![
            PathBuf::from("/tmp/avahi-tui-list-a"),
            PathBuf::from("/tmp/avahi-tui-list-b"),
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

        let matcher = load_matcher(&test_cli(
            CliCommand::ListCommands,
            vec![base.clone(), overlay.clone()],
        ))
        .unwrap();

        assert_eq!(matcher.command_count(), 2);
        assert_eq!(matcher.commands()[0].name, "mosh");
        assert_eq!(matcher.commands()[1].name, "ssh");
        assert_eq!(matcher.commands()[1].action.command, "ssh overlay");

        remove(&base);
        remove(&overlay);
    }

    #[test]
    fn load_keybindings_falls_back_to_defaults_when_no_files_exist() {
        let bindings =
            KeyBindings::load(&[PathBuf::from("/tmp/avahi-tui-no-such-keymap")]).unwrap();

        assert!(bindings.is(
            "browse",
            "quit",
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('q'),
                crossterm::event::KeyModifiers::NONE,
            )
        ));
    }
}
