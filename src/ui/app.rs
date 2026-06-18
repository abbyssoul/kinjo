use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
    sync::mpsc,
    time::Duration,
};

use color_eyre::eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::{
    discovery::{DiscoveryEvent, Entry, EntryGroup, EntryId, GroupingMode, group_entries},
    plumber::{self, ActionMode, CommandConfig, MatchResult, PreparedCommand, RuleEngine},
};

use super::{cli::Cli, filter::FilterState, keymap::KeyBindings, render};

/// One row of the "group by command" view: a configured command together with
/// the distinct logical services it matches.
#[derive(Debug, Clone)]
pub struct CommandGroup {
    pub command: CommandConfig,
    pub services: Vec<EntryGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Browse,
    Search,
    TypeFilter,
    ActionPicker,
    InstancePicker,
    ServicePicker,
    Help,
}

pub struct App {
    pub cli: Cli,
    pub matcher: Box<dyn RuleEngine>,
    pub keybindings: KeyBindings,
    pub discovery_rx: mpsc::Receiver<DiscoveryEvent>,
    pub records: BTreeMap<EntryId, Entry>,
    pub filter: FilterState,
    pub visible_groups: Vec<EntryGroup>,
    pub selected: usize,
    pub mode: AppMode,
    pub type_filter_index: usize,
    pub action_matches: Vec<MatchResult>,
    pub action_index: usize,
    pub pending_action: Option<MatchResult>,
    pub instance_index: usize,
    pub status: String,
    pub group_match_counts: Vec<usize>,
    pub ticks: u64,
    /// Rows of the "group by command" view; populated only in that grouping mode.
    pub command_groups: Vec<CommandGroup>,
    /// Cursor within the service picker opened from a command row.
    pub service_picker_index: usize,
    /// Top line shown in the details pane (0 = unscrolled).
    pub details_scroll: usize,
    /// Largest valid `details_scroll`, recomputed by the renderer each frame.
    pub details_max_scroll: Cell<usize>,
    /// Visible height of the details pane, recomputed by the renderer each frame.
    pub details_viewport: Cell<usize>,
}

impl App {
    pub fn new(
        cli: Cli,
        matcher: impl RuleEngine + 'static,
        keybindings: KeyBindings,
        discovery_rx: mpsc::Receiver<DiscoveryEvent>,
    ) -> Self {
        let status = format!(
            "domain: {} | commands: {} | waiting for services",
            cli.domain,
            matcher.command_count()
        );
        Self {
            cli,
            matcher: Box::new(matcher),
            keybindings,
            discovery_rx,
            records: BTreeMap::new(),
            filter: FilterState::default(),
            visible_groups: Vec::new(),
            selected: 0,
            mode: AppMode::Browse,
            type_filter_index: 0,
            action_matches: Vec::new(),
            action_index: 0,
            pending_action: None,
            instance_index: 0,
            status,
            group_match_counts: Vec::new(),
            ticks: 0,
            command_groups: Vec::new(),
            service_picker_index: 0,
            details_scroll: 0,
            details_max_scroll: Cell::new(0),
            details_viewport: Cell::new(0),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<PreparedCommand>> {
        self.recompute_visible();

        loop {
            self.ticks = self.ticks.wrapping_add(1);
            self.drain_discovery();
            terminal.draw(|frame| render::render(frame, self))?;

            if event::poll(Duration::from_millis(120))?
                && let Event::Key(key) = event::read()?
            {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if let Some(command) = self.handle_key(key)? {
                    return Ok(Some(command));
                }
                if matches!(self.mode, AppMode::Browse) && self.status == "quit" {
                    return Ok(None);
                }
            }
        }
    }

    fn drain_discovery(&mut self) {
        let mut changed = false;
        while let Ok(event) = self.discovery_rx.try_recv() {
            match event {
                DiscoveryEvent::Upsert(record) => {
                    if record.has_instance_data() {
                        self.records.remove(&record.pending_id());
                    }
                    self.records.insert(record.id.clone(), record);
                    changed = true;
                }
                DiscoveryEvent::Remove(id) => {
                    let registration_key = id.registration_key();
                    self.records
                        .retain(|record_id, _| record_id.registration_key() != registration_key);
                    changed = true;
                }
                DiscoveryEvent::Status(status) => {
                    self.status = status;
                }
            }
        }
        if changed {
            self.recompute_visible();
        }
    }

    fn recompute_visible(&mut self) {
        let records = self.records.values().cloned().collect::<Vec<_>>();
        self.filter.sync_service_types(&records);
        let filtered = self.filter.apply(&records);

        if self.filter.grouping == GroupingMode::Command {
            self.recompute_command_groups(&filtered);
            return;
        }

        self.command_groups = Vec::new();
        let previous = self
            .visible_groups
            .get(self.selected)
            .map(|group| group.id.clone());
        self.visible_groups = group_entries(&filtered, self.filter.grouping);
        self.group_match_counts = self
            .visible_groups
            .iter()
            .map(|group| self.matcher.matches_group(group).len())
            .collect();
        match find_selection(&self.visible_groups, previous, |group| group.id.clone()) {
            Some(index) => self.selected = index,
            None => self.clamp_selection(),
        }
    }

    /// Build the command-grouped rows: each configured command paired with the
    /// distinct logical services that match at least one of its instances.
    fn recompute_command_groups(&mut self, filtered: &[Entry]) {
        let previous = self
            .command_groups
            .get(self.selected)
            .map(|group| group.command.name.clone());

        let service_groups = group_entries(filtered, GroupingMode::LogicalService);
        let mut command_groups: Vec<CommandGroup> = self
            .matcher
            .commands()
            .iter()
            .map(|command| CommandGroup {
                command: command.clone(),
                services: Vec::new(),
            })
            .collect();
        let index: HashMap<String, usize> = command_groups
            .iter()
            .enumerate()
            .map(|(i, group)| (group.command.name.clone(), i))
            .collect();
        for service_group in &service_groups {
            for result in self.matcher.matches_group(service_group) {
                if let Some(&i) = index.get(&result.command.name) {
                    command_groups[i].services.push(service_group.clone());
                }
            }
        }

        self.command_groups = command_groups;
        self.visible_groups = Vec::new();
        self.group_match_counts = Vec::new();
        match find_selection(&self.command_groups, previous, |group| {
            group.command.name.clone()
        }) {
            Some(index) => self.selected = index,
            None => self.clamp_selection(),
        }
    }

    /// Number of rows in the currently active left-hand list.
    fn active_count(&self) -> usize {
        if self.filter.grouping == GroupingMode::Command {
            self.command_groups.len()
        } else {
            self.visible_groups.len()
        }
    }

    fn clamp_selection(&mut self) {
        let count = self.active_count();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    /// Close any open modal/picker and drop its transient state. Clearing the
    /// already-empty action state on plain closes (search/help) is harmless.
    fn return_to_browse(&mut self) {
        self.mode = AppMode::Browse;
        self.action_matches.clear();
        self.pending_action = None;
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        if self.keybindings.is("common", "quit", key) {
            self.status = "quit".to_string();
            return Ok(None);
        }

        match self.mode {
            AppMode::Browse => self.handle_browse_key(key),
            AppMode::Search => {
                self.handle_search_key(key);
                Ok(None)
            }
            AppMode::TypeFilter => {
                self.handle_type_filter_key(key);
                Ok(None)
            }
            AppMode::ActionPicker => self.handle_action_picker_key(key),
            AppMode::InstancePicker => self.handle_instance_picker_key(key),
            AppMode::ServicePicker => self.handle_service_picker_key(key),
            AppMode::Help => {
                if self.keybindings.is("help", "close", key) {
                    self.return_to_browse();
                }
                Ok(None)
            }
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        match key.code {
            _ if self.keybindings.is("browse", "quit", key) => self.status = "quit".to_string(),
            _ if self.keybindings.is("browse", "down", key) => self.move_selection(1),
            _ if self.keybindings.is("browse", "up", key) => self.move_selection(-1),
            _ if self.keybindings.is("browse", "invoke", key) => return self.invoke_selected(),
            _ if self.keybindings.is("browse", "search", key) => self.mode = AppMode::Search,
            _ if self.keybindings.is("browse", "type_filter", key) => {
                self.type_filter_index = 0;
                self.mode = AppMode::TypeFilter;
            }
            _ if self.keybindings.is("browse", "tab_next", key) => self.cycle_tab(1),
            _ if self.keybindings.is("browse", "tab_prev", key) => self.cycle_tab(-1),
            _ if self.keybindings.is("browse", "same_host", key) => {
                self.toggle_same_host_filter();
            }
            _ if self.keybindings.is("browse", "details_down", key) => self.scroll_details(1),
            _ if self.keybindings.is("browse", "details_up", key) => self.scroll_details(-1),
            _ if self.keybindings.is("browse", "help", key) => self.mode = AppMode::Help,
            KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.filter.text_query.push(ch);
                    self.mode = AppMode::Search;
                    self.recompute_visible();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            _ if self.keybindings.is("search", "close", key) => self.return_to_browse(),
            KeyCode::Backspace => {
                self.filter.text_query.pop();
                self.recompute_visible();
            }
            _ if self.keybindings.is("search", "clear", key) => {
                self.filter.clear_text();
                self.recompute_visible();
            }
            KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.filter.text_query.push(ch);
                    self.recompute_visible();
                }
            }
            _ => {}
        }
    }

    fn handle_type_filter_key(&mut self, key: KeyEvent) {
        let types = self.service_types();
        match key.code {
            _ if self.keybindings.is("type_filter", "close", key) => self.return_to_browse(),
            _ if self.keybindings.is("type_filter", "down", key) => {
                self.type_filter_index = move_index(self.type_filter_index, types.len(), 1);
            }
            _ if self.keybindings.is("type_filter", "up", key) => {
                self.type_filter_index = move_index(self.type_filter_index, types.len(), -1);
            }
            _ if self.keybindings.is("type_filter", "toggle", key) => {
                if let Some(service_type) = types.get(self.type_filter_index) {
                    self.filter.toggle_service_type(service_type);
                    self.recompute_visible();
                }
            }
            _ => {}
        }
    }

    /// Switch the active top-panel tab to the grouping mode at `index` within
    /// [`GroupingMode::TABS`]. Switching views resets the cursor and detail
    /// scroll so the new list starts cleanly from the top.
    fn select_tab(&mut self, index: usize) {
        let Some(&mode) = GroupingMode::TABS.get(index) else {
            return;
        };
        if self.filter.grouping == mode {
            return;
        }
        self.filter.grouping = mode;
        self.selected = 0;
        self.details_scroll = 0;
        self.recompute_visible();
    }

    /// Move to the next/previous tab, wrapping around the ends.
    fn cycle_tab(&mut self, delta: isize) {
        let len = GroupingMode::TABS.len() as isize;
        let current = GroupingMode::TABS
            .iter()
            .position(|mode| *mode == self.filter.grouping)
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.select_tab(next);
    }

    fn handle_action_picker_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        let len = self.action_matches.len();
        match key.code {
            _ if self.keybindings.is("picker", "close", key) => self.return_to_browse(),
            _ if self.keybindings.is("picker", "down", key) => {
                self.action_index = move_index(self.action_index, len, 1);
            }
            _ if self.keybindings.is("picker", "up", key) => {
                self.action_index = move_index(self.action_index, len, -1);
            }
            _ if self.keybindings.is("picker", "select", key) => {
                if let Some(action) = self.action_matches.get(self.action_index).cloned() {
                    return self.choose_action(action);
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_instance_picker_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        let count = self
            .pending_action
            .as_ref()
            .map(|action| action.matching_records.len())
            .unwrap_or(0);
        match key.code {
            _ if self.keybindings.is("picker", "close", key) => self.return_to_browse(),
            _ if self.keybindings.is("picker", "down", key) => {
                self.instance_index = move_index(self.instance_index, count, 1);
            }
            _ if self.keybindings.is("picker", "up", key) => {
                self.instance_index = move_index(self.instance_index, count, -1);
            }
            _ if self.keybindings.is("picker", "select", key) => {
                let Some(action) = self.pending_action.clone() else {
                    self.return_to_browse();
                    return Ok(None);
                };
                let Some(record) = action.matching_records.get(self.instance_index) else {
                    self.return_to_browse();
                    return Ok(None);
                };
                return self.execute_action(&action, record);
            }
            _ => {}
        }
        Ok(None)
    }

    fn move_selection(&mut self, delta: isize) {
        self.selected = move_index(self.selected, self.active_count(), delta);
        // A different row is now in focus — start its details from the top.
        self.details_scroll = 0;
    }

    /// Scroll the details pane by half its visible height (vim/tig `u`/`d`).
    /// `direction` is +1 to scroll down, -1 to scroll up.
    fn scroll_details(&mut self, direction: isize) {
        let step = (self.details_viewport.get() / 2).max(1) as isize;
        let max = self.details_max_scroll.get() as isize;
        self.details_scroll =
            (self.details_scroll as isize + direction * step).clamp(0, max) as usize;
    }

    fn invoke_selected(&mut self) -> Result<Option<PreparedCommand>> {
        if self.filter.grouping == GroupingMode::Command {
            return self.invoke_command();
        }
        let Some(group) = self.visible_groups.get(self.selected) else {
            self.status = "no service selected".to_string();
            return Ok(None);
        };
        let matches = self.matcher.matches_group(group);
        match matches.len() {
            0 => {
                self.status = format!("no configured actions match `{}`", group.label);
                Ok(None)
            }
            1 => self.choose_action(matches.into_iter().next().unwrap()),
            _ => {
                self.action_matches = matches;
                self.action_index = 0;
                self.mode = AppMode::ActionPicker;
                Ok(None)
            }
        }
    }

    fn invoke_command(&mut self) -> Result<Option<PreparedCommand>> {
        let Some(group) = self.command_groups.get(self.selected) else {
            self.status = "no command selected".to_string();
            return Ok(None);
        };
        match group.services.len() {
            0 => {
                self.status = format!("no services match command `{}`", group.command.name);
                Ok(None)
            }
            1 => {
                let command = group.command.clone();
                let service = group.services[0].clone();
                self.run_command_on(&command, &service)
            }
            _ => {
                self.service_picker_index = 0;
                self.mode = AppMode::ServicePicker;
                Ok(None)
            }
        }
    }

    /// Run `command` against a chosen logical service, reusing the regular
    /// action flow (which handles instance disambiguation and execution).
    fn run_command_on(
        &mut self,
        command: &CommandConfig,
        service: &EntryGroup,
    ) -> Result<Option<PreparedCommand>> {
        let Some(result) = self
            .matcher
            .matches_group(service)
            .into_iter()
            .find(|result| result.command.name == command.name)
        else {
            self.status = format!("`{}` no longer matches `{}`", command.name, service.label);
            self.return_to_browse();
            return Ok(None);
        };
        self.choose_action(result)
    }

    fn handle_service_picker_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        let count = self
            .command_groups
            .get(self.selected)
            .map(|group| group.services.len())
            .unwrap_or(0);
        match key.code {
            _ if self.keybindings.is("picker", "close", key) => self.return_to_browse(),
            _ if self.keybindings.is("picker", "down", key) => {
                self.service_picker_index = move_index(self.service_picker_index, count, 1);
            }
            _ if self.keybindings.is("picker", "up", key) => {
                self.service_picker_index = move_index(self.service_picker_index, count, -1);
            }
            _ if self.keybindings.is("picker", "select", key) => {
                let Some(group) = self.command_groups.get(self.selected) else {
                    self.return_to_browse();
                    return Ok(None);
                };
                let Some(service) = group.services.get(self.service_picker_index).cloned() else {
                    self.return_to_browse();
                    return Ok(None);
                };
                let command = group.command.clone();
                return self.run_command_on(&command, &service);
            }
            _ => {}
        }
        Ok(None)
    }

    fn choose_action(&mut self, action: MatchResult) -> Result<Option<PreparedCommand>> {
        if action.needs_instance && action.matching_records.len() > 1 {
            self.pending_action = Some(action);
            self.instance_index = 0;
            self.mode = AppMode::InstancePicker;
            return Ok(None);
        }

        let Some(record) = action.matching_records.first() else {
            self.status = "selected action has no matching services".to_string();
            self.return_to_browse();
            return Ok(None);
        };
        self.execute_action(&action, record)
    }

    fn execute_action(
        &mut self,
        action: &MatchResult,
        record: &Entry,
    ) -> Result<Option<PreparedCommand>> {
        let name = &action.command.name;

        if let Some(missing) = plumber::exec::missing_requirement(&action.command.requirements) {
            return self.fail(format!(
                "`{name}` needs `{missing}`, which is not installed"
            ));
        }

        let prepared = match plumber::exec::prepare(&action.command.action, record) {
            Ok(prepared) => prepared,
            Err(err) => return self.fail(format!("cannot run `{name}`: {err}")),
        };

        match prepared.mode {
            ActionMode::Fork => match plumber::exec::fork(&prepared) {
                Ok(()) => {
                    self.status = format!("launched `{name}`");
                    self.return_to_browse();
                    Ok(None)
                }
                Err(err) => self.fail(format!("cannot run `{name}`: {err}")),
            },
            // The execute hand-off happens after the TUI is torn down, so the
            // caller takes ownership of the prepared command from here.
            ActionMode::Execute => Ok(Some(prepared)),
        }
    }

    /// Surface a user-facing failure on the status line and return to browsing.
    /// Action failures are expected (bad config, missing tools) and must never
    /// propagate out of the event loop and tear down the terminal.
    fn fail(&mut self, message: String) -> Result<Option<PreparedCommand>> {
        self.status = message;
        self.return_to_browse();
        Ok(None)
    }

    fn toggle_same_host_filter(&mut self) {
        if self.filter.grouping == GroupingMode::Command && self.filter.host_filter.is_none() {
            self.status = "same-host filter is unavailable in command view".to_string();
            return;
        }
        if self.filter.host_filter.is_some() {
            self.filter.clear_host_filter();
            self.status = "host filter cleared".to_string();
            self.recompute_visible();
            return;
        }
        match self
            .visible_groups
            .get(self.selected)
            .and_then(|group| group.hostname.clone())
        {
            Some(host) => {
                self.status = format!("filtering by host `{host}`");
                self.filter.set_host_filter(host);
                self.recompute_visible();
            }
            None => self.status = "selected service has no resolved host yet".to_string(),
        }
    }

    pub fn service_types(&self) -> Vec<String> {
        FilterState::discovered_types(&self.records.values().cloned().collect::<Vec<_>>())
    }
}

/// Move a list cursor by `delta`, clamped to `[0, len-1]` (or 0 when empty).
fn move_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).clamp(0, len as isize - 1) as usize
}

/// Index of the item whose key equals `previous` — used to keep the cursor on
/// the same logical row when a list is rebuilt.
fn find_selection<T, K: PartialEq>(
    items: &[T],
    previous: Option<K>,
    key: impl Fn(&T) -> K,
) -> Option<usize> {
    let previous = previous?;
    items.iter().position(|item| key(item) == previous)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::plumber::Matcher;
    use crate::ui::keymap::KeyBindings;

    fn test_cli() -> Cli {
        Cli {
            domain: "local".to_string(),
            config_dirs: Vec::new(),
            service_type: None,
            fake_discovery: true,
            command: crate::ui::cli::CliCommand::Run,
        }
    }

    #[test]
    fn resolved_service_replaces_pending_record() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(test_cli(), Matcher::default(), KeyBindings::default(), rx);

        let pending = Entry::new("workstation", "_ssh._tcp", "local").with_instance_id();
        let mut resolved = Entry::new("workstation", "_ssh._tcp", "local");
        resolved.hostname = Some("workstation.local".to_string());
        resolved.address = Some("192.168.1.20".parse().unwrap());
        resolved.port = Some(22);
        let resolved = resolved.with_instance_id();

        tx.send(DiscoveryEvent::Upsert(pending)).unwrap();
        tx.send(DiscoveryEvent::Upsert(resolved)).unwrap();

        app.drain_discovery();

        assert_eq!(app.records.len(), 1);
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances.len(), 1);
        assert_eq!(
            app.visible_groups[0].hostname.as_deref(),
            Some("workstation.local")
        );
    }

    #[test]
    fn command_grouping_counts_distinct_matching_services() {
        use crate::plumber::MatcherBuilder;

        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "ssh",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();

        let (tx, rx) = mpsc::channel();
        let mut app = App::new(test_cli(), matcher, KeyBindings::default(), rx);

        let mut alpha = Entry::new("alpha", "_ssh._tcp", "local");
        alpha.hostname = Some("alpha.local".to_string());
        alpha.address = Some("192.168.1.10".parse().unwrap());
        alpha.port = Some(22);
        let mut beta = Entry::new("beta", "_ssh._tcp", "local");
        beta.hostname = Some("beta.local".to_string());
        beta.address = Some("192.168.1.11".parse().unwrap());
        beta.port = Some(22);
        let web = Entry::new("web", "_http._tcp", "local");

        tx.send(DiscoveryEvent::Upsert(alpha.with_instance_id()))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(beta.with_instance_id()))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(web.with_instance_id()))
            .unwrap();
        app.drain_discovery();

        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();

        assert_eq!(app.command_groups.len(), 1);
        assert_eq!(app.command_groups[0].command.name, "ssh");
        // alpha + beta are distinct logical services; the http service is excluded.
        assert_eq!(app.command_groups[0].services.len(), 2);
    }

    #[test]
    fn multiple_resolved_addresses_remain_instances() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(test_cli(), Matcher::default(), KeyBindings::default(), rx);

        let mut first = Entry::new("workstation", "_ssh._tcp", "local");
        first.hostname = Some("workstation.local".to_string());
        first.address = Some("192.168.1.20".parse().unwrap());
        first.port = Some(22);

        let mut second = first.clone();
        second.address = Some("192.168.1.21".parse().unwrap());

        tx.send(DiscoveryEvent::Upsert(first.with_instance_id()))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(second.with_instance_id()))
            .unwrap();

        app.drain_discovery();

        assert_eq!(app.records.len(), 2);
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances.len(), 2);
    }

    // ── interaction harness ────────────────────────────────────────────────
    use crate::plumber::MatcherBuilder;
    use crossterm::event::KeyModifiers;

    const SSH: &str = r#"
[metadata]
name = "ssh"
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "ssh {hostname}"
mode = "execute"
"#;

    const PING: &str = r#"
[metadata]
name = "ping"
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "true"
mode = "fork"
"#;

    /// Matches every instance by address and echoes it, so picking different
    /// instances yields observably different argv.
    const PING_ADDR: &str = r#"
[metadata]
name = "ping-addr"
[match.address]
regex = "^10[.]"
[action]
command = "echo {address}"
mode = "execute"
"#;

    fn matcher_from(sources: &[&str]) -> Matcher {
        let mut builder = MatcherBuilder::new();
        builder.start_layer();
        for (index, source) in sources.iter().enumerate() {
            builder.add_str(&format!("test-{index}"), source).unwrap();
        }
        builder.build()
    }

    fn app_with(matcher: Matcher, records: Vec<Entry>) -> App {
        let (_tx, rx) = mpsc::channel();
        let mut app = App::new(test_cli(), matcher, KeyBindings::default(), rx);
        for record in records {
            let record = record.with_instance_id();
            app.records.insert(record.id.clone(), record);
        }
        app.recompute_visible();
        app
    }

    fn ssh(name: &str, addr: &str) -> Entry {
        let mut record = Entry::new(name, "_ssh._tcp", "local");
        record.hostname = Some(format!("{name}.local"));
        record.address = Some(addr.parse().unwrap());
        record.port = Some(22);
        record
    }

    fn http(name: &str) -> Entry {
        let mut record = Entry::new(name, "_http._tcp", "local");
        record.hostname = Some(format!("{name}.local"));
        record.address = Some("192.168.1.50".parse().unwrap());
        record.port = Some(80);
        record
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn send(app: &mut App, code: KeyCode) -> Option<PreparedCommand> {
        app.handle_key(key(code)).unwrap()
    }

    #[test]
    fn navigation_moves_and_clamps_selection_and_resets_scroll() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        assert_eq!(app.visible_groups.len(), 2);
        app.details_scroll = 4;

        send(&mut app, KeyCode::Down);
        assert_eq!(app.selected, 1);
        assert_eq!(
            app.details_scroll, 0,
            "moving rows resets the detail scroll"
        );

        send(&mut app, KeyCode::Down);
        assert_eq!(app.selected, 1, "down clamps at the last row");

        send(&mut app, KeyCode::Up);
        send(&mut app, KeyCode::Up);
        assert_eq!(app.selected, 0, "up clamps at the first row");
    }

    #[test]
    fn typing_in_browse_enters_search_and_filters() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("zulu", "10.0.0.2")],
        );

        send(&mut app, KeyCode::Char('z'));

        assert_eq!(app.mode, AppMode::Search);
        assert_eq!(app.filter.text_query, "z");
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].label, "zulu");
    }

    #[test]
    fn search_backspace_clear_and_close() {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        send(&mut app, KeyCode::Char('z'));
        send(&mut app, KeyCode::Char('u'));
        assert_eq!(app.filter.text_query, "zu");

        send(&mut app, KeyCode::Backspace);
        assert_eq!(app.filter.text_query, "z");

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.filter.text_query, "", "ctrl-u clears the query");

        send(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, AppMode::Browse, "enter closes search");
    }

    #[test]
    fn type_filter_toggle_hides_a_service_type() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), http("web")],
        );
        assert_eq!(app.visible_groups.len(), 2);

        send(&mut app, KeyCode::Char('t'));
        assert_eq!(app.mode, AppMode::TypeFilter);
        // Discovered types are sorted: _http._tcp is first.
        send(&mut app, KeyCode::Char(' '));

        assert!(
            app.visible_groups
                .iter()
                .all(|g| g.service_type == "_ssh._tcp")
        );
        assert_eq!(app.visible_groups.len(), 1);
    }

    #[test]
    fn tab_keys_cycle_active_view_and_wrap() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        assert_eq!(app.filter.grouping, GroupingMode::LogicalService);

        // TABS = [LogicalService, Host, ServiceType, Command]; two forward steps
        // land on the service-type view.
        send(&mut app, KeyCode::Tab);
        assert_eq!(app.filter.grouping, GroupingMode::Host);
        send(&mut app, KeyCode::Tab);
        assert_eq!(app.filter.grouping, GroupingMode::ServiceType);
        assert_eq!(app.mode, AppMode::Browse);

        // Stepping back past the first tab wraps to the last (command) tab.
        send(&mut app, KeyCode::BackTab);
        send(&mut app, KeyCode::BackTab);
        send(&mut app, KeyCode::BackTab);
        assert_eq!(app.filter.grouping, GroupingMode::Command);
    }

    #[test]
    fn switching_tabs_resets_selection_and_scroll() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        send(&mut app, KeyCode::Down);
        app.details_scroll = 3;
        assert_eq!(app.selected, 1);

        send(&mut app, KeyCode::Tab);
        assert_eq!(app.selected, 0, "switching views resets the cursor");
        assert_eq!(
            app.details_scroll, 0,
            "switching views resets detail scroll"
        );
    }

    #[test]
    fn invoke_single_matching_action_returns_prepared_execute_command() {
        let mut app = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")]);

        let command = send(&mut app, KeyCode::Enter).expect("execute action returns a command");

        assert_eq!(command.argv, vec!["ssh", "alpha.local"]);
    }

    #[test]
    fn invoke_with_multiple_actions_opens_picker_then_runs_selection() {
        let mut app = app_with(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")]);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::ActionPicker);
        assert_eq!(app.action_matches.len(), 2);

        // action_index 0 is `ssh` (insertion order); selecting it runs that action.
        let command = send(&mut app, KeyCode::Enter).expect("picked action runs");
        assert_eq!(command.argv, vec!["ssh", "alpha.local"]);
    }

    #[test]
    fn invoke_without_a_matching_command_reports_status() {
        let mut app = app_with(matcher_from(&[SSH]), vec![http("web")]);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert!(app.status.contains("no configured actions match"));
    }

    #[test]
    fn fork_action_launches_and_returns_to_browse() {
        let mut app = app_with(matcher_from(&[PING]), vec![ssh("alpha", "10.0.0.1")]);

        assert!(
            send(&mut app, KeyCode::Enter).is_none(),
            "fork does not exec"
        );
        assert_eq!(app.mode, AppMode::Browse);
        assert!(app.status.contains("launched `ping`"));
    }

    #[test]
    fn instance_picker_disambiguates_then_executes_chosen_instance() {
        // Two instances of one logical service differing only by address.
        let mut app = app_with(
            matcher_from(&[PING_ADDR]),
            vec![ssh("alpha", "10.0.0.1"), ssh("alpha", "10.0.0.2")],
        );
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances.len(), 2);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::InstancePicker);

        // Instances sort by ascending address: index 1 is 10.0.0.2.
        send(&mut app, KeyCode::Down);
        let command = send(&mut app, KeyCode::Enter).expect("instance chosen");
        assert_eq!(command.argv, vec!["echo", "10.0.0.2"]);
    }

    #[test]
    fn same_host_filter_toggles_on_and_off() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        // Groups sort by label, so the cursor starts on `alpha`.
        send(&mut app, KeyCode::Char('s'));
        assert_eq!(app.filter.host_filter.as_deref(), Some("alpha.local"));
        assert_eq!(app.visible_groups.len(), 1);

        send(&mut app, KeyCode::Char('s'));
        assert!(app.filter.host_filter.is_none());
        assert_eq!(app.visible_groups.len(), 2);
    }

    #[test]
    fn remove_event_drops_records_sharing_a_registration_key() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(test_cli(), Matcher::default(), KeyBindings::default(), rx);

        tx.send(DiscoveryEvent::Upsert(
            ssh("alpha", "10.0.0.1").with_instance_id(),
        ))
        .unwrap();
        app.drain_discovery();
        assert_eq!(app.records.len(), 1);

        let removal = Entry::new("alpha", "_ssh._tcp", "local")
            .with_instance_id()
            .id;
        tx.send(DiscoveryEvent::Remove(removal)).unwrap();
        app.drain_discovery();

        assert!(app.records.is_empty());
        assert!(app.visible_groups.is_empty());
    }

    #[test]
    fn command_view_runs_single_service_and_picks_among_many() {
        let mut single = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")]);
        single.filter.grouping = GroupingMode::Command;
        single.recompute_visible();
        assert_eq!(single.command_groups.len(), 1);
        assert_eq!(single.command_groups[0].services.len(), 1);

        let command = send(&mut single, KeyCode::Enter).expect("single service runs");
        assert_eq!(command.argv, vec!["ssh", "alpha.local"]);

        let mut many = app_with(
            matcher_from(&[SSH]),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        many.filter.grouping = GroupingMode::Command;
        many.recompute_visible();
        assert_eq!(many.command_groups[0].services.len(), 2);

        assert!(send(&mut many, KeyCode::Enter).is_none());
        assert_eq!(many.mode, AppMode::ServicePicker);
        // Services sort by label; index 1 is `beta`.
        send(&mut many, KeyCode::Down);
        let command = send(&mut many, KeyCode::Enter).expect("picked service runs");
        assert_eq!(command.argv, vec!["ssh", "beta.local"]);
    }

    #[test]
    fn quit_keys_set_status_to_quit() {
        let mut common = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        assert!(
            common
                .handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
                .unwrap()
                .is_none()
        );
        assert_eq!(common.status, "quit");

        let mut browse = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        send(&mut browse, KeyCode::Char('q'));
        assert_eq!(browse.status, "quit");
    }

    #[test]
    fn help_modal_opens_and_closes() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);

        send(&mut app, KeyCode::Char('?'));
        assert_eq!(app.mode, AppMode::Help);

        send(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, AppMode::Browse);
    }

    #[test]
    fn scroll_details_steps_by_half_viewport_and_clamps() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        app.details_viewport.set(10);
        app.details_max_scroll.set(5);

        send(&mut app, KeyCode::Char('d'));
        assert_eq!(app.details_scroll, 5, "half of 10 clamps to the max of 5");

        send(&mut app, KeyCode::Char('d'));
        assert_eq!(app.details_scroll, 5, "cannot scroll past the maximum");

        send(&mut app, KeyCode::Char('u'));
        assert_eq!(app.details_scroll, 0, "scrolling up returns to the top");
    }

    #[test]
    fn move_index_clamps_and_handles_empty_lists() {
        assert_eq!(move_index(0, 0, 1), 0);
        assert_eq!(move_index(0, 3, -1), 0);
        assert_eq!(move_index(2, 3, 1), 2);
        assert_eq!(move_index(1, 3, 1), 2);
    }

    // ── command-execution error handling ───────────────────────────────────
    const NEEDS_TOOL: &str = r#"
[metadata]
name = "needs-tool"
requirements = ["avahi-tui-absent-tool-xyz"]
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "echo hi"
mode = "execute"
"#;

    const BAD_TEMPLATE: &str = r#"
[metadata]
name = "bad-template"
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "echo {nonexistent_field}"
mode = "execute"
"#;

    const FORK_MISSING_BINARY: &str = r#"
[metadata]
name = "ghost"
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "avahi-tui-absent-binary-xyz --flag"
mode = "fork"
"#;

    const OPTIONAL_REQ: &str = r#"
[metadata]
name = "with-optional"
requirements = ["avahi-tui-absent-tool-xyz, optional"]
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "echo hi"
mode = "execute"
"#;

    #[test]
    fn unsatisfied_requirement_reports_status_without_executing() {
        let mut app = app_with(matcher_from(&[NEEDS_TOOL]), vec![ssh("alpha", "10.0.0.1")]);

        assert!(
            send(&mut app, KeyCode::Enter).is_none(),
            "a missing requirement must not execute"
        );
        assert!(app.status.contains("avahi-tui-absent-tool-xyz"));
        assert_eq!(app.mode, AppMode::Browse);
    }

    #[test]
    fn optional_requirement_does_not_block_execution() {
        let mut app = app_with(
            matcher_from(&[OPTIONAL_REQ]),
            vec![ssh("alpha", "10.0.0.1")],
        );

        let command = send(&mut app, KeyCode::Enter).expect("optional requirement is skipped");
        assert_eq!(command.argv, vec!["echo", "hi"]);
    }

    #[test]
    fn bad_template_reports_status_instead_of_crashing_the_loop() {
        let mut app = app_with(
            matcher_from(&[BAD_TEMPLATE]),
            vec![ssh("alpha", "10.0.0.1")],
        );

        // `send` unwraps the handler Result, so reaching the assert proves the
        // error never propagated out of the event loop.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert!(app.status.contains("cannot run `bad-template`"));
        assert_eq!(app.mode, AppMode::Browse);
    }

    #[test]
    fn fork_failure_reports_status_and_stays_in_browse() {
        let mut app = app_with(
            matcher_from(&[FORK_MISSING_BINARY]),
            vec![ssh("alpha", "10.0.0.1")],
        );

        assert!(send(&mut app, KeyCode::Enter).is_none());
        // The message names both the action and the missing binary.
        assert!(app.status.contains("cannot run `ghost`"));
        assert!(
            app.status
                .contains("command `avahi-tui-absent-binary-xyz` not found")
        );
        assert_eq!(app.mode, AppMode::Browse);
    }

    #[test]
    fn failed_action_closes_an_open_picker() {
        // Two actions match, opening the picker; selecting the broken one must
        // both report the error and drop the picker, not leave it dangling.
        let mut app = app_with(
            matcher_from(&[SSH, BAD_TEMPLATE]),
            vec![ssh("alpha", "10.0.0.1")],
        );

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::ActionPicker);
        assert_eq!(app.action_matches.len(), 2);

        // action_index 1 is the bad-template command (insertion order).
        send(&mut app, KeyCode::Down);
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::Browse);
        assert!(app.action_matches.is_empty());
        assert!(app.status.contains("cannot run `bad-template`"));
    }
}
