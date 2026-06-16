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
        bindings.set("browse", "grouping", &["g"]);
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
        bindings.set("grouping", "close", &["esc", "g"]);
        bindings.set("grouping", "up", &["up", "k"]);
        bindings.set("grouping", "down", &["down", "j"]);
        bindings.set("grouping", "select", &["enter"]);
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
    let mut paths = Vec::new();
    if let Some(home) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.push(
            PathBuf::from(home)
                .join("avahi-tui")
                .join("keybindings.toml"),
        );
    } else if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("avahi-tui")
                .join("keybindings.toml"),
        );
    }
    paths
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

    #[test]
    fn defaults_include_vim_navigation() {
        let bindings = KeyBindings::default();
        assert!(bindings.is(
            "browse",
            "down",
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
        ));
    }
}
