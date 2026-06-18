use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, eyre};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec {
    code: KeyCode,
    ctrl: bool,
}

#[derive(Debug, Clone)]
pub struct KeyBindings {
    bindings: BTreeMap<(String, String), Vec<KeySpec>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings = Self {
            bindings: BTreeMap::new(),
        };
        bindings.set("browse", "quit", &["q"]);
        bindings.set("browse", "up", &["up", "k"]);
        bindings.set("browse", "down", &["down", "j"]);
        bindings.set("browse", "invoke", &["enter"]);
        bindings.set("browse", "search", &["/"]);
        bindings.set("browse", "type_filter", &["t"]);
        bindings.set("browse", "tab_next", &["tab", "right"]);
        bindings.set("browse", "tab_prev", &["backtab", "left"]);
        bindings.set("browse", "same_host", &["s"]);
        bindings.set("browse", "details_down", &["d", "pagedown", "ctrl-d"]);
        bindings.set("browse", "details_up", &["u", "pageup", "ctrl-u"]);
        bindings.set("browse", "help", &["?"]);
        bindings.set("search", "close", &["esc", "enter"]);
        bindings.set("search", "clear", &["ctrl-u"]);
        bindings.set("type_filter", "close", &["esc", "t"]);
        bindings.set("type_filter", "up", &["up", "k"]);
        bindings.set("type_filter", "down", &["down", "j"]);
        bindings.set("type_filter", "toggle", &["space", "enter"]);
        bindings.set("picker", "close", &["esc"]);
        bindings.set("picker", "up", &["up", "k"]);
        bindings.set("picker", "down", &["down", "j"]);
        bindings.set("picker", "select", &["enter"]);
        bindings.set("help", "close", &["esc", "?", "q"]);
        bindings.set("common", "quit", &["ctrl-c"]);
        bindings
    }
}

impl KeyBindings {
    pub fn load(paths: &[PathBuf]) -> Result<Self> {
        let mut bindings = Self::default();
        for path in paths {
            if path.is_file() {
                bindings.apply_file(path)?;
            }
        }
        Ok(bindings)
    }

    pub fn is(&self, mode: &str, command: &str, key: KeyEvent) -> bool {
        self.bindings
            .get(&(mode.to_string(), command.to_string()))
            .is_some_and(|keys| keys.iter().any(|spec| spec.matches(key)))
    }

    fn set(&mut self, mode: &str, command: &str, keys: &[&str]) {
        let specs = keys
            .iter()
            .map(|key| KeySpec::parse(key).expect("default keybindings must be valid"))
            .collect();
        self.bindings
            .insert((mode.to_string(), command.to_string()), specs);
    }

    fn apply_file(&mut self, path: &Path) -> Result<()> {
        let source = fs::read_to_string(path)?;
        let mut mode: Option<String> = None;
        for (index, line) in source.lines().enumerate() {
            let line_no = index + 1;
            let line = strip_comment(line).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                mode = Some(line[1..line.len() - 1].trim().to_string());
                continue;
            }
            let Some((command, value)) = line.split_once('=') else {
                return Err(eyre!(
                    "{}:{line_no}: expected command = [keys]",
                    path.display()
                ));
            };
            let Some(mode) = &mode else {
                return Err(eyre!(
                    "{}:{line_no}: keybinding outside a section",
                    path.display()
                ));
            };
            let keys = parse_key_array(path, line_no, value.trim())?;
            self.bindings
                .insert((mode.clone(), command.trim().to_string()), keys);
        }
        Ok(())
    }
}

impl KeySpec {
    fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        let (ctrl, key) = normalized
            .strip_prefix("ctrl-")
            .map(|key| (true, key))
            .unwrap_or((false, normalized.as_str()));
        let code = match key {
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "enter" => KeyCode::Enter,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            "esc" | "escape" => KeyCode::Esc,
            "space" => KeyCode::Char(' '),
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "backspace" => KeyCode::Backspace,
            value if value.chars().count() == 1 => KeyCode::Char(value.chars().next().unwrap()),
            _ => return Err(eyre!("unsupported key `{value}`")),
        };
        Ok(Self { code, ctrl })
    }

    fn matches(&self, event: KeyEvent) -> bool {
        self.code == event.code && event.modifiers.contains(KeyModifiers::CONTROL) == self.ctrl
    }
}

pub fn default_config_paths() -> Vec<PathBuf> {
    config_paths(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Resolve the keybinding file search path from the relevant environment
/// variables. Split out from [`default_config_paths`] so the precedence rules
/// can be unit tested without mutating process-global environment state.
fn config_paths(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    if let Some(xdg) = xdg_config_home {
        vec![
            PathBuf::from(xdg)
                .join("avahi-tui")
                .join("keybindings.toml"),
        ]
    } else if let Some(home) = home {
        vec![
            PathBuf::from(home)
                .join(".config")
                .join("avahi-tui")
                .join("keybindings.toml"),
        ]
    } else {
        Vec::new()
    }
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map(|(line, _)| line).unwrap_or(line)
}

fn parse_key_array(path: &Path, line_no: usize, value: &str) -> Result<Vec<KeySpec>> {
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(eyre!("{}:{line_no}: expected key array", path.display()));
    }
    let inner = value[1..value.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|item| {
            let item = item.trim();
            if !item.starts_with('"') || !item.ends_with('"') {
                return Err(eyre!(
                    "{}:{line_no}: keys must be quoted strings",
                    path.display()
                ));
            }
            KeySpec::parse(&item[1..item.len() - 1])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{remove, temp_file};
    use std::ffi::OsString;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    #[test]
    fn defaults_include_vim_navigation() {
        let bindings = KeyBindings::default();
        assert!(bindings.is("browse", "down", key(KeyCode::Char('j'))));
    }

    #[test]
    fn load_replaces_default_command_bindings_from_file() {
        let path = temp_file(
            "override",
            r#"
[browse]
quit = ["x", "ctrl-x"]
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert!(bindings.is("browse", "quit", key(KeyCode::Char('x'))));
        assert!(bindings.is("browse", "quit", ctrl('x')));
        assert!(!bindings.is("browse", "quit", key(KeyCode::Char('q'))));

        remove(&path);
    }

    #[test]
    fn later_files_override_earlier_files() {
        let first = temp_file(
            "first",
            r#"
[picker]
select = ["space"]
"#,
        );
        let second = temp_file(
            "second",
            r#"
[picker]
select = ["tab"]
"#,
        );

        let bindings = KeyBindings::load(&[first.clone(), second.clone()]).unwrap();

        assert!(bindings.is("picker", "select", key(KeyCode::Tab)));
        assert!(!bindings.is("picker", "select", key(KeyCode::Char(' '))));

        remove(&first);
        remove(&second);
    }

    #[test]
    fn key_aliases_and_comments_are_parsed() {
        let path = temp_file(
            "aliases",
            r#"
# full-line comment
[help]
close = ["escape", "backspace"] # trailing comment
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert!(bindings.is("help", "close", key(KeyCode::Esc)));
        assert!(bindings.is("help", "close", key(KeyCode::Backspace)));

        remove(&path);
    }

    #[test]
    fn ctrl_binding_does_not_match_plain_key() {
        let bindings = KeyBindings::default();

        assert!(bindings.is("common", "quit", ctrl('c')));
        assert!(!bindings.is("common", "quit", key(KeyCode::Char('c'))));
    }

    #[test]
    fn invalid_files_report_line_or_key_errors() {
        let outside_section = temp_file("outside-section", r#"quit = ["q"]"#);
        let unquoted = temp_file(
            "unquoted",
            r#"
[browse]
quit = [q]
"#,
        );
        let unsupported = temp_file(
            "unsupported",
            r#"
[browse]
quit = ["meta-q"]
"#,
        );

        let outside_err = KeyBindings::load(std::slice::from_ref(&outside_section)).unwrap_err();
        let unquoted_err = KeyBindings::load(std::slice::from_ref(&unquoted)).unwrap_err();
        let unsupported_err = KeyBindings::load(std::slice::from_ref(&unsupported)).unwrap_err();

        assert!(outside_err.to_string().contains("outside a section"));
        assert!(
            unquoted_err
                .to_string()
                .contains("keys must be quoted strings")
        );
        assert!(unsupported_err.to_string().contains("unsupported key"));

        remove(&outside_section);
        remove(&unquoted);
        remove(&unsupported);
    }

    #[test]
    fn empty_key_array_unbinds_a_command() {
        let path = temp_file(
            "empty-array",
            r#"
[browse]
quit = []
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        // The default `q` binding is replaced by an empty list, so nothing fires.
        assert!(!bindings.is("browse", "quit", key(KeyCode::Char('q'))));

        remove(&path);
    }

    #[test]
    fn config_paths_prefer_xdg_config_home_over_home() {
        let paths = config_paths(
            Some(OsString::from("/xdg")),
            Some(OsString::from("/home/user")),
        );

        assert_eq!(
            paths,
            vec![PathBuf::from("/xdg/avahi-tui/keybindings.toml")]
        );
    }

    #[test]
    fn config_paths_fall_back_to_dot_config_under_home() {
        let paths = config_paths(None, Some(OsString::from("/home/user")));

        assert_eq!(
            paths,
            vec![PathBuf::from(
                "/home/user/.config/avahi-tui/keybindings.toml"
            )]
        );
    }

    #[test]
    fn config_paths_are_empty_without_any_home_variables() {
        assert!(config_paths(None, None).is_empty());
    }

    #[test]
    fn unknown_mode_or_command_never_matches() {
        let bindings = KeyBindings::default();

        assert!(!bindings.is("no-such-mode", "quit", key(KeyCode::Char('q'))));
        assert!(!bindings.is("browse", "no-such-command", key(KeyCode::Char('q'))));
    }
}
