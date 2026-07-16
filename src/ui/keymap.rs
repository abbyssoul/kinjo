use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, eyre};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A UI mode that dispatches keys. [`Mode::Common`] is not a mode the UI is
/// ever "in": its actions stay active inside every dispatching mode, which is
/// why a common binding can collide with a mode-specific one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    Common,
    Browse,
    Search,
    TypeFilter,
    Picker,
    Help,
}

impl Mode {
    /// The modes that resolve keys. Every one of them also sees
    /// [`Mode::Common`], so this is the set collision validation walks.
    pub const DISPATCH: [Mode; 5] = [
        Mode::Browse,
        Mode::Search,
        Mode::TypeFilter,
        Mode::Picker,
        Mode::Help,
    ];

    /// The `[section]` name this mode carries in a keybindings file.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Common => "common",
            Mode::Browse => "browse",
            Mode::Search => "search",
            Mode::TypeFilter => "type_filter",
            Mode::Picker => "picker",
            Mode::Help => "help",
        }
    }
}

/// One bindable UI action. Each action belongs to exactly one [`Mode`] and has
/// one command name, so the typed value and the `mode.command` spelling in a
/// keybindings file are two views of the same thing.
///
/// Declaration order is the resolution and reporting order: [`Action::Quit`]
/// first, so a common binding is named consistently in collision errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// `common.quit`: active in every mode.
    Quit,
    BrowseQuit,
    MoveUp,
    MoveDown,
    Invoke,
    OpenSearch,
    OpenTypeFilter,
    TabNext,
    TabPrev,
    SameHost,
    Refresh,
    DetailsDown,
    DetailsUp,
    OpenHelp,
    SearchClose,
    SearchClear,
    TypeFilterClose,
    TypeFilterUp,
    TypeFilterDown,
    TypeFilterToggle,
    PickerClose,
    PickerUp,
    PickerDown,
    PickerSelect,
    HelpClose,
    HelpUp,
    HelpDown,
}

impl Action {
    pub const ALL: [Action; 27] = [
        Action::Quit,
        Action::BrowseQuit,
        Action::MoveUp,
        Action::MoveDown,
        Action::Invoke,
        Action::OpenSearch,
        Action::OpenTypeFilter,
        Action::TabNext,
        Action::TabPrev,
        Action::SameHost,
        Action::Refresh,
        Action::DetailsDown,
        Action::DetailsUp,
        Action::OpenHelp,
        Action::SearchClose,
        Action::SearchClear,
        Action::TypeFilterClose,
        Action::TypeFilterUp,
        Action::TypeFilterDown,
        Action::TypeFilterToggle,
        Action::PickerClose,
        Action::PickerUp,
        Action::PickerDown,
        Action::PickerSelect,
        Action::HelpClose,
        Action::HelpUp,
        Action::HelpDown,
    ];

    /// The mode this action is bound in, and its command name within it.
    fn spelling(self) -> (Mode, &'static str) {
        match self {
            Action::Quit => (Mode::Common, "quit"),
            Action::BrowseQuit => (Mode::Browse, "quit"),
            Action::MoveUp => (Mode::Browse, "up"),
            Action::MoveDown => (Mode::Browse, "down"),
            Action::Invoke => (Mode::Browse, "invoke"),
            Action::OpenSearch => (Mode::Browse, "search"),
            Action::OpenTypeFilter => (Mode::Browse, "type_filter"),
            Action::TabNext => (Mode::Browse, "tab_next"),
            Action::TabPrev => (Mode::Browse, "tab_prev"),
            Action::SameHost => (Mode::Browse, "same_host"),
            Action::Refresh => (Mode::Browse, "refresh"),
            Action::DetailsDown => (Mode::Browse, "details_down"),
            Action::DetailsUp => (Mode::Browse, "details_up"),
            Action::OpenHelp => (Mode::Browse, "help"),
            Action::SearchClose => (Mode::Search, "close"),
            Action::SearchClear => (Mode::Search, "clear"),
            Action::TypeFilterClose => (Mode::TypeFilter, "close"),
            Action::TypeFilterUp => (Mode::TypeFilter, "up"),
            Action::TypeFilterDown => (Mode::TypeFilter, "down"),
            Action::TypeFilterToggle => (Mode::TypeFilter, "toggle"),
            Action::PickerClose => (Mode::Picker, "close"),
            Action::PickerUp => (Mode::Picker, "up"),
            Action::PickerDown => (Mode::Picker, "down"),
            Action::PickerSelect => (Mode::Picker, "select"),
            Action::HelpClose => (Mode::Help, "close"),
            Action::HelpUp => (Mode::Help, "up"),
            Action::HelpDown => (Mode::Help, "down"),
        }
    }

    pub fn mode(self) -> Mode {
        self.spelling().0
    }

    pub fn command(self) -> &'static str {
        self.spelling().1
    }

    /// How this action is written in a keybindings file, e.g. `browse.quit`.
    pub fn qualified_name(self) -> String {
        format!("{}.{}", self.mode().name(), self.command())
    }

    /// The action a `mode.command` pair names, if any.
    fn parse(mode: &str, command: &str) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|action| action.mode().name() == mode && action.command() == command)
    }

    /// Whether this action is resolved while the UI is in `mode`: its own
    /// actions plus the always-active common ones.
    fn active_in(self, mode: Mode) -> bool {
        let own = self.mode();
        own == mode || own == Mode::Common
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec {
    code: KeyCode,
    ctrl: bool,
}

#[derive(Debug, Clone)]
pub struct KeyBindings {
    bindings: BTreeMap<Action, Vec<KeySpec>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings = Self {
            bindings: BTreeMap::new(),
        };
        bindings.set(Action::BrowseQuit, &["q"]);
        bindings.set(Action::MoveUp, &["up", "k"]);
        bindings.set(Action::MoveDown, &["down", "j"]);
        bindings.set(Action::Invoke, &["enter"]);
        bindings.set(Action::OpenSearch, &["/"]);
        bindings.set(Action::OpenTypeFilter, &["t"]);
        bindings.set(Action::TabNext, &["tab", "right"]);
        bindings.set(Action::TabPrev, &["backtab", "left"]);
        bindings.set(Action::SameHost, &["s"]);
        bindings.set(Action::Refresh, &["r", "f5"]);
        bindings.set(Action::DetailsDown, &["d", "pagedown", "ctrl-d"]);
        bindings.set(Action::DetailsUp, &["u", "pageup", "ctrl-u"]);
        bindings.set(Action::OpenHelp, &["?"]);
        bindings.set(Action::SearchClose, &["esc", "enter"]);
        bindings.set(Action::SearchClear, &["ctrl-u"]);
        bindings.set(Action::TypeFilterClose, &["esc", "t"]);
        bindings.set(Action::TypeFilterUp, &["up", "k"]);
        bindings.set(Action::TypeFilterDown, &["down", "j"]);
        bindings.set(Action::TypeFilterToggle, &["space", "enter"]);
        bindings.set(Action::PickerClose, &["esc"]);
        bindings.set(Action::PickerUp, &["up", "k"]);
        bindings.set(Action::PickerDown, &["down", "j"]);
        bindings.set(Action::PickerSelect, &["enter"]);
        bindings.set(Action::HelpClose, &["esc", "?", "q"]);
        bindings.set(Action::HelpUp, &["up", "k", "pageup"]);
        bindings.set(Action::HelpDown, &["down", "j", "pagedown"]);
        bindings.set(Action::Quit, &["ctrl-c"]);
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
        bindings.ensure_no_collisions()?;
        bindings.ensure_quit_reachable()?;
        Ok(bindings)
    }

    /// The single action `key` triggers while the UI is in `mode`, if any.
    ///
    /// At most one action can match: [`KeyBindings::load`] rejects any
    /// configuration where two actions active in one mode share a key, so
    /// callers never depend on the order these are checked in.
    pub fn resolve(&self, mode: Mode, key: KeyEvent) -> Option<Action> {
        Action::ALL
            .into_iter()
            .filter(|action| action.active_in(mode))
            .find(|action| self.keys(*action).iter().any(|spec| spec.matches(key)))
    }

    /// The compact label for `action`: its first bound key only. `None` when
    /// the action is unbound, so a caller can drop the hint entirely.
    pub fn compact(&self, action: Action) -> Option<String> {
        self.keys(action).first().map(KeySpec::label)
    }

    /// Compact labels for several actions that share one hint (such as the
    /// down/up pair behind "move"), joined with `/`. Unbound actions are
    /// skipped; `None` when none of them is bound.
    pub fn compact_group(&self, actions: &[Action]) -> Option<String> {
        Self::join(
            actions.iter().filter_map(|action| self.compact(*action)),
            "/",
        )
    }

    /// The complete label for `action`: every key bound to it. Used where
    /// there is room to be exhaustive rather than compact.
    pub fn describe(&self, action: Action) -> Option<String> {
        Self::join(self.keys(action).iter().map(KeySpec::label), " / ")
    }

    /// Complete labels for several actions covered by one help row.
    pub fn describe_group(&self, actions: &[Action]) -> Option<String> {
        Self::join(
            actions
                .iter()
                .flat_map(|action| self.keys(*action).iter().map(KeySpec::label)),
            " / ",
        )
    }

    fn join(labels: impl Iterator<Item = String>, separator: &str) -> Option<String> {
        let joined = labels.collect::<Vec<_>>().join(separator);
        (!joined.is_empty()).then_some(joined)
    }

    fn keys(&self, action: Action) -> &[KeySpec] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    fn set(&mut self, action: Action, keys: &[&str]) {
        let specs = keys
            .iter()
            .map(|key| KeySpec::parse(key).expect("default keybindings must be valid"))
            .collect();
        self.bindings.insert(action, dedupe(specs));
    }

    fn apply_file(&mut self, path: &Path) -> Result<()> {
        let source = fs::read_to_string(path)?;
        let parsed: BTreeMap<String, BTreeMap<String, Vec<String>>> =
            toml::from_str(&source).map_err(|err| eyre!("{}: {err}", path.display()))?;
        for (mode, commands) in parsed {
            for (command, keys) in commands {
                // Only bindings that name a real action are meaningful; an
                // unknown pair is a typo that would otherwise silently do nothing.
                let Some(action) = Action::parse(&mode, &command) else {
                    return Err(eyre!(
                        "{}: unknown keybinding `{mode}.{command}`",
                        path.display(),
                    ));
                };
                let specs = keys
                    .iter()
                    .map(|key| {
                        KeySpec::parse(key).map_err(|err| {
                            eyre!("{}: `{}`: {err}", path.display(), action.qualified_name())
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.bindings.insert(action, dedupe(specs));
            }
        }
        Ok(())
    }

    /// Reject a configuration in which one key would trigger two actions in the
    /// same mode. Without this, dispatch order would silently decide which of
    /// them wins and make the other unreachable. Common bindings are checked
    /// against every mode because they stay active inside all of them.
    fn ensure_no_collisions(&self) -> Result<()> {
        for mode in Mode::DISPATCH {
            let mut claimed: Vec<(&KeySpec, Action)> = Vec::new();
            for action in Action::ALL.into_iter().filter(|a| a.active_in(mode)) {
                for spec in self.keys(action) {
                    if let Some((_, owner)) = claimed.iter().find(|(seen, _)| *seen == spec) {
                        return Err(eyre!(
                            "keybinding conflict in `{}` mode: `{}` is bound to both `{}` and `{}`",
                            mode.name(),
                            spec.label(),
                            owner.qualified_name(),
                            action.qualified_name(),
                        ));
                    }
                    claimed.push((spec, action));
                }
            }
        }
        Ok(())
    }

    /// Guard against a configuration that unbinds every way out of the app.
    fn ensure_quit_reachable(&self) -> Result<()> {
        let quit_bound = [Action::Quit, Action::BrowseQuit]
            .into_iter()
            .any(|action| !self.keys(action).is_empty());
        if quit_bound {
            Ok(())
        } else {
            Err(eyre!(
                "keybindings leave no way to quit: bind `common.quit` or `browse.quit`"
            ))
        }
    }
}

/// Drop repeated keys within one action: they bind the same thing, so they are
/// not a conflict, but they would double up in a hint.
fn dedupe(specs: Vec<KeySpec>) -> Vec<KeySpec> {
    let mut unique: Vec<KeySpec> = Vec::with_capacity(specs.len());
    for spec in specs {
        if !unique.contains(&spec) {
            unique.push(spec);
        }
    }
    unique
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

    /// How this key is shown to the user. Display lives here so that every
    /// hint spells a key the same way the resolver understands it.
    fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Enter => "⏎".to_string(),
            KeyCode::PageUp => "pgup".to_string(),
            KeyCode::PageDown => "pgdn".to_string(),
            KeyCode::Esc => "esc".to_string(),
            KeyCode::Tab => "⇥".to_string(),
            KeyCode::BackTab => "⇧⇥".to_string(),
            KeyCode::Backspace => "⌫".to_string(),
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::F(number) => format!("F{number}"),
            other => format!("{other:?}"),
        };
        if self.ctrl { format!("^{base}") } else { base }
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
        vec![PathBuf::from(xdg).join("kinjo").join("keybindings.toml")]
    } else if let Some(home) = home {
        vec![
            PathBuf::from(home)
                .join(".config")
                .join("kinjo")
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

        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('j'))),
            Some(Action::MoveDown)
        );
    }

    #[test]
    fn refresh_is_bound_to_r_and_f5_by_default() {
        let bindings = KeyBindings::default();

        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('r'))),
            Some(Action::Refresh)
        );
        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::F(5))),
            Some(Action::Refresh)
        );
    }

    /// The defaults are the configuration every user starts from, so they must
    /// satisfy the same rule a custom file is held to.
    #[test]
    fn default_bindings_are_free_of_collisions() {
        KeyBindings::default().ensure_no_collisions().unwrap();
        KeyBindings::default().ensure_quit_reachable().unwrap();
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

        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('x'))),
            Some(Action::BrowseQuit)
        );
        assert_eq!(
            bindings.resolve(Mode::Browse, ctrl('x')),
            Some(Action::BrowseQuit)
        );
        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('q'))),
            None
        );

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

        assert_eq!(
            bindings.resolve(Mode::Picker, key(KeyCode::Tab)),
            Some(Action::PickerSelect)
        );
        assert_eq!(
            bindings.resolve(Mode::Picker, key(KeyCode::Char(' '))),
            None
        );

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

        assert_eq!(
            bindings.resolve(Mode::Help, key(KeyCode::Esc)),
            Some(Action::HelpClose)
        );
        assert_eq!(
            bindings.resolve(Mode::Help, key(KeyCode::Backspace)),
            Some(Action::HelpClose)
        );

        remove(&path);
    }

    #[test]
    fn ctrl_binding_does_not_match_plain_key() {
        let bindings = KeyBindings::default();

        assert_eq!(
            bindings.resolve(Mode::Browse, ctrl('c')),
            Some(Action::Quit)
        );
        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('c'))),
            None
        );
    }

    /// The common quit stays live inside every modal mode, not just browse.
    #[test]
    fn common_bindings_resolve_inside_every_mode() {
        let bindings = KeyBindings::default();

        for mode in Mode::DISPATCH {
            assert_eq!(bindings.resolve(mode, ctrl('c')), Some(Action::Quit));
        }
    }

    #[test]
    fn a_key_resolves_only_within_its_own_mode() {
        let bindings = KeyBindings::default();

        // `/` opens search while browsing and means nothing in a picker.
        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('/'))),
            Some(Action::OpenSearch)
        );
        assert_eq!(
            bindings.resolve(Mode::Picker, key(KeyCode::Char('/'))),
            None
        );
    }

    /// The same key in two different modes is not a conflict: only one of the
    /// modes is ever dispatching.
    #[test]
    fn the_same_key_may_bind_different_actions_in_different_modes() {
        let path = temp_file(
            "cross-mode",
            r#"
[browse]
same_host = ["z"]

[picker]
select = ["z"]
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('z'))),
            Some(Action::SameHost)
        );
        assert_eq!(
            bindings.resolve(Mode::Picker, key(KeyCode::Char('z'))),
            Some(Action::PickerSelect)
        );

        remove(&path);
    }

    #[test]
    fn two_colliding_actions_in_one_mode_are_rejected() {
        let path = temp_file(
            "browse-collision",
            r#"
[browse]
same_host = ["z"]
refresh = ["z"]
"#,
        );

        let err = KeyBindings::load(std::slice::from_ref(&path)).unwrap_err();

        let message = err.to_string();
        // The error has to name the key and both sides of the conflict; that is
        // the whole information needed to fix the file.
        assert!(message.contains("keybinding conflict"), "{message}");
        assert!(message.contains("`z`"), "{message}");
        assert!(message.contains("browse.same_host"), "{message}");
        assert!(message.contains("browse.refresh"), "{message}");

        remove(&path);
    }

    /// A common binding is live inside every mode, so it conflicts with a
    /// mode's own binding even though the two live in different sections.
    #[test]
    fn a_common_binding_colliding_with_a_modal_action_is_rejected() {
        let path = temp_file(
            "common-collision",
            r#"
[common]
quit = ["space"]
"#,
        );

        let err = KeyBindings::load(std::slice::from_ref(&path)).unwrap_err();

        let message = err.to_string();
        assert!(message.contains("keybinding conflict"), "{message}");
        assert!(message.contains("type_filter"), "{message}");
        assert!(message.contains("common.quit"), "{message}");
        assert!(message.contains("type_filter.toggle"), "{message}");

        remove(&path);
    }

    /// Rebinding a colliding default away resolves the conflict: validation
    /// judges the effective configuration, not the file in isolation.
    #[test]
    fn moving_the_conflicting_default_away_makes_a_common_binding_valid() {
        let path = temp_file(
            "common-resolved",
            r#"
[common]
quit = ["space"]

[type_filter]
toggle = ["enter"]
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert_eq!(
            bindings.resolve(Mode::TypeFilter, key(KeyCode::Char(' '))),
            Some(Action::Quit)
        );

        remove(&path);
    }

    /// One action listing a key twice binds the same thing twice; it is not an
    /// ambiguity and must not be reported as one, nor doubled in a hint.
    #[test]
    fn a_repeated_key_within_one_action_is_not_a_collision() {
        let path = temp_file(
            "repeat",
            r#"
[browse]
refresh = ["z", "z"]
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert_eq!(bindings.describe(Action::Refresh).unwrap(), "z");

        remove(&path);
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

    /// Unbinding one of the two quit actions is fine: the other still gets the
    /// user out.
    #[test]
    fn unbinding_one_quit_action_is_allowed_while_the_other_survives() {
        let path = temp_file(
            "one-quit",
            r#"
[browse]
quit = []
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('q'))),
            None
        );
        assert_eq!(
            bindings.resolve(Mode::Browse, ctrl('c')),
            Some(Action::Quit)
        );

        remove(&path);
    }

    #[test]
    fn alt_modified_keys_do_not_trigger_plain_bindings() {
        let bindings = KeyBindings::default();

        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('j'))),
            Some(Action::MoveDown)
        );
        assert_eq!(
            bindings.resolve(
                Mode::Browse,
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT)
            ),
            None
        );
    }

    #[test]
    fn empty_key_array_unbinds_a_command() {
        let path = temp_file(
            "empty-array",
            r#"
[browse]
same_host = []
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        // The default `s` binding is replaced by an empty list, so nothing fires
        // and no hint can be produced for it.
        assert_eq!(
            bindings.resolve(Mode::Browse, key(KeyCode::Char('s'))),
            None
        );
        assert_eq!(bindings.compact(Action::SameHost), None);
        assert_eq!(bindings.describe(Action::SameHost), None);

        remove(&path);
    }

    /// An unbound action must vanish from a shared hint rather than leave a
    /// dangling separator.
    #[test]
    fn an_unbound_action_drops_out_of_a_grouped_hint() {
        let path = temp_file(
            "half-unbound",
            r#"
[browse]
up = []
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert_eq!(
            bindings.compact_group(&[Action::MoveDown, Action::MoveUp]),
            Some("↓".to_string())
        );

        remove(&path);
    }

    #[test]
    fn default_bindings_format_for_display() {
        let bindings = KeyBindings::default();

        // Compact takes the first key only; describe keeps every alias.
        assert_eq!(bindings.compact(Action::MoveDown), Some("↓".to_string()));
        assert_eq!(
            bindings.describe(Action::MoveDown),
            Some("↓ / j".to_string())
        );
        assert_eq!(bindings.compact(Action::Invoke), Some("⏎".to_string()));
        assert_eq!(bindings.compact(Action::TabNext), Some("⇥".to_string()));
        assert_eq!(
            bindings.describe(Action::TabPrev),
            Some("⇧⇥ / ←".to_string())
        );
        assert_eq!(bindings.compact(Action::OpenSearch), Some("/".to_string()));
        assert_eq!(
            bindings.compact(Action::SearchClose),
            Some("esc".to_string())
        );

        // Function and control keys keep their conventional spellings.
        assert_eq!(
            bindings.describe(Action::Refresh),
            Some("r / F5".to_string())
        );
        assert_eq!(
            bindings.describe(Action::DetailsDown),
            Some("d / pgdn / ^d".to_string())
        );
        assert_eq!(
            bindings.compact(Action::SearchClear),
            Some("^u".to_string())
        );
        assert_eq!(bindings.compact(Action::Quit), Some("^c".to_string()));

        assert_eq!(
            bindings.compact_group(&[Action::MoveDown, Action::MoveUp]),
            Some("↓/↑".to_string())
        );
        assert_eq!(
            bindings.describe_group(&[Action::BrowseQuit, Action::Quit]),
            Some("q / ^c".to_string())
        );
    }

    #[test]
    fn custom_bindings_format_for_display() {
        let path = temp_file(
            "custom-format",
            r#"
[browse]
down = ["ctrl-n", "f9"]
up = ["ctrl-p"]
invoke = ["space"]
help = ["f1"]
"#,
        );

        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        assert_eq!(bindings.compact(Action::MoveDown), Some("^n".to_string()));
        assert_eq!(
            bindings.describe(Action::MoveDown),
            Some("^n / F9".to_string())
        );
        assert_eq!(bindings.compact(Action::Invoke), Some("space".to_string()));
        assert_eq!(bindings.compact(Action::OpenHelp), Some("F1".to_string()));
        assert_eq!(
            bindings.compact_group(&[Action::MoveDown, Action::MoveUp]),
            Some("^n/^p".to_string())
        );

        remove(&path);
    }

    /// Every action must be spelled the way a keybindings file spells it, and
    /// no two actions may claim the same spelling.
    #[test]
    fn every_action_has_a_unique_round_tripping_spelling() {
        let mut names: Vec<String> = Vec::new();
        for action in Action::ALL {
            assert_eq!(
                Action::parse(action.mode().name(), action.command()),
                Some(action)
            );
            names.push(action.qualified_name());
        }
        names.sort();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique);
    }

    /// Every action must have a default binding, otherwise a file could never
    /// name it and the defaults would not describe the whole interface.
    #[test]
    fn every_action_appears_in_the_defaults() {
        let bindings = KeyBindings::default();

        for action in Action::ALL {
            assert!(
                bindings.bindings.contains_key(&action),
                "{} has no default entry",
                action.qualified_name()
            );
        }
    }

    /// Every example in `docs/keybindings.md` must survive validation. A
    /// documented example that the loader rejects is a broken promise.
    #[test]
    fn the_documented_examples_are_accepted() {
        let examples = [
            // Arrow keys only, Vim navigation disabled.
            r#"
[browse]
up = ["up"]
down = ["down"]
details_up = ["pageup"]
details_down = ["pagedown"]
"#,
            // Emacs-style navigation for every list.
            r#"
[browse]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]

[type_filter]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]

[picker]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]
"#,
            // Search moved off `/`; `f` must not collide with `f5`.
            r#"
[browse]
search = ["f"]
"#,
            // `x` closes every modal.
            r#"
[type_filter]
close = ["esc", "x"]

[picker]
close = ["esc", "x"]

[help]
close = ["esc", "x"]
"#,
            // The documented conflict resolution for a common binding.
            r#"
[common]
quit = ["space"]

[type_filter]
toggle = ["enter"]
"#,
        ];

        for (index, example) in examples.iter().enumerate() {
            let path = temp_file(&format!("documented-{index}"), example);
            let loaded = KeyBindings::load(std::slice::from_ref(&path));
            remove(&path);
            assert!(
                loaded.is_ok(),
                "documented example {index} was rejected: {}",
                loaded.unwrap_err()
            );
        }
    }

    #[test]
    fn config_paths_prefer_xdg_config_home_over_home() {
        let paths = config_paths(
            Some(OsString::from("/xdg")),
            Some(OsString::from("/home/user")),
        );

        assert_eq!(paths, vec![PathBuf::from("/xdg/kinjo/keybindings.toml")]);
    }

    #[test]
    fn config_paths_fall_back_to_dot_config_under_home() {
        let paths = config_paths(None, Some(OsString::from("/home/user")));

        assert_eq!(
            paths,
            vec![PathBuf::from("/home/user/.config/kinjo/keybindings.toml")]
        );
    }

    #[test]
    fn config_paths_are_empty_without_any_home_variables() {
        assert!(config_paths(None, None).is_empty());
    }
}
