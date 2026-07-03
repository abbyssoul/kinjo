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
        bindings.set("browse", "refresh", &["r", "f5"]);
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
        bindings.ensure_quit_reachable()?;
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
        let parsed: BTreeMap<String, BTreeMap<String, Vec<String>>> =
            toml::from_str(&source).map_err(|err| eyre!("{}: {err}", path.display()))?;
        for (mode, commands) in parsed {
            for (command, keys) in commands {
                // Only bindings that exist in the defaults are meaningful; an
                // unknown pair is a typo that would otherwise silently do nothing.
                let binding = (mode.clone(), command);
                if !self.bindings.contains_key(&binding) {
                    return Err(eyre!(
                        "{}: unknown keybinding `{mode}.{}`",
                        path.display(),
                        binding.1
                    ));
                }
                let specs = keys
                    .iter()
                    .map(|key| {
                        KeySpec::parse(key).map_err(|err| {
                            eyre!("{}: `{mode}.{}`: {err}", path.display(), binding.1)
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.bindings.insert(binding, specs);
            }
        }
        Ok(())
    }

    /// Guard against a configuration that unbinds every way out of the app.
    fn ensure_quit_reachable(&self) -> Result<()> {
        let quit_bound = [("common", "quit"), ("browse", "quit")]
            .iter()
            .any(|(mode, command)| {
                self.bindings
                    .get(&(mode.to_string(), command.to_string()))
                    .is_some_and(|keys| !keys.is_empty())
            });
        if quit_bound {
            Ok(())
        } else {
            Err(eyre!(
                "keybindings leave no way to quit: bind `common.quit` or `browse.quit`"
            ))
        }
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
            value => char_or_function_key(value)?,
        };
        Ok(Self { code, ctrl })
    }

    fn matches(&self, event: KeyEvent) -> bool {
        // SHIFT is deliberately ignored: it is already encoded in the char
        // (`?` arrives as Char('?') with SHIFT set). ALT is not part of any
        // spec, so an ALT-modified key must not trigger a plain binding.
        self.code == event.code
            && event.modifiers.contains(KeyModifiers::CONTROL) == self.ctrl
            && !event.modifiers.contains(KeyModifiers::ALT)
    }
}

/// A single-character key (`x`) or a function key (`f1`–`f12`). A bare `f` is
/// the character key, not a function key.
fn char_or_function_key(value: &str) -> Result<KeyCode> {
    if let Some(digits) = value.strip_prefix('f')
        && !digits.is_empty()
        && digits.chars().all(|ch| ch.is_ascii_digit())
    {
        return match digits.parse::<u8>() {
            Ok(number @ 1..=12) => Ok(KeyCode::F(number)),
            _ => Err(eyre!("function key `{value}` is outside f1-f12")),
        };
    }
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Ok(KeyCode::Char(ch)),
        _ => Err(eyre!("unsupported key `{value}`")),
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
    fn refresh_is_bound_to_r_and_f5_by_default() {
        let bindings = KeyBindings::default();
        assert!(bindings.is("browse", "refresh", key(KeyCode::Char('r'))));
        assert!(bindings.is("browse", "refresh", key(KeyCode::F(5))));
    }

    #[test]
    fn function_keys_parse_within_f1_to_f12() {
        assert_eq!(KeySpec::parse("f1").unwrap().code, KeyCode::F(1));
        assert_eq!(KeySpec::parse("F5").unwrap().code, KeyCode::F(5));
        assert_eq!(KeySpec::parse("f12").unwrap().code, KeyCode::F(12));
        assert_eq!(KeySpec::parse("ctrl-f5").unwrap().code, KeyCode::F(5));

        // A bare `f` is the character key, not a truncated function key.
        assert_eq!(KeySpec::parse("f").unwrap().code, KeyCode::Char('f'));

        assert!(KeySpec::parse("f0").is_err());
        assert!(KeySpec::parse("f13").is_err());
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
    fn invalid_files_report_file_and_key_errors() {
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

        // Every parse error is prefixed with the offending file's path; the
        // TOML errors also carry a snippet of the offending line.
        assert!(
            outside_err
                .to_string()
                .contains(&outside_section.display().to_string())
        );
        assert!(unquoted_err.to_string().contains("quit = [q]"));
        assert!(unsupported_err.to_string().contains("unsupported key"));
        assert!(unsupported_err.to_string().contains("`browse.quit`"));

        remove(&outside_section);
        remove(&unquoted);
        remove(&unsupported);
    }

    #[test]
    fn unknown_modes_and_commands_are_rejected() {
        let bad_mode = temp_file(
            "bad-mode",
            r#"
[brwose]
quit = ["x"]
"#,
        );
        let bad_command = temp_file(
            "bad-command",
            r#"
[browse]
qiut = ["x"]
"#,
        );

        let mode_err = KeyBindings::load(std::slice::from_ref(&bad_mode)).unwrap_err();
        let command_err = KeyBindings::load(std::slice::from_ref(&bad_command)).unwrap_err();

        assert!(mode_err.to_string().contains("unknown keybinding"));
        assert!(mode_err.to_string().contains("brwose.quit"));
        assert!(command_err.to_string().contains("browse.qiut"));

        remove(&bad_mode);
        remove(&bad_command);
    }

    #[test]
    fn unbinding_every_quit_key_is_rejected() {
        let path = temp_file(
            "no-quit",
            r#"
[browse]
quit = []

[common]
quit = []
"#,
        );

        let err = KeyBindings::load(std::slice::from_ref(&path)).unwrap_err();

        assert!(err.to_string().contains("no way to quit"));

        remove(&path);
    }

    #[test]
    fn alt_modified_keys_do_not_trigger_plain_bindings() {
        let bindings = KeyBindings::default();

        assert!(bindings.is("browse", "down", key(KeyCode::Char('j'))));
        assert!(!bindings.is(
            "browse",
            "down",
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT)
        ));
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
