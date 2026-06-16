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
    cli::Cli,
    discovery::DiscoveryEvent,
    filter::FilterState,
    keymap::KeyBindings,
    plumber::{ActionMode, CommandConfig, MatchResult, Matcher},
    process::{self, PreparedCommand},
    service::{self, GroupingMode, ServiceGroup, ServiceId, ServiceRecord},
    ui,
};

/// One row of the "group by command" view: a configured command together with
/// the distinct logical services it matches.
#[derive(Debug, Clone)]
pub struct CommandGroup {
    pub command: CommandConfig,
    pub services: Vec<ServiceGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Browse,
    Search,
    TypeFilter,
    Grouping,
    ActionPicker,
    InstancePicker,
    ServicePicker,
    Help,
}

pub struct App {
    pub cli: Cli,
    pub matcher: Matcher,
    pub keybindings: KeyBindings,
    pub discovery_rx: mpsc::Receiver<DiscoveryEvent>,
    pub records: BTreeMap<ServiceId, ServiceRecord>,
    pub filter: FilterState,
    pub visible_groups: Vec<ServiceGroup>,
    pub selected: usize,
    pub mode: AppMode,
    pub type_filter_index: usize,
    pub grouping_index: usize,
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
        matcher: Matcher,
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
            matcher,
            keybindings,
            discovery_rx,
            records: BTreeMap::new(),
            filter: FilterState::default(),
            visible_groups: Vec::new(),
            selected: 0,
            mode: AppMode::Browse,
            type_filter_index: 0,
            grouping_index: 0,
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
            terminal.draw(|frame| ui::render(frame, self))?;

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
        let previous_selection = self
            .visible_groups
            .get(self.selected)
            .map(|group| group.id.clone());
        self.visible_groups = service::group_records(&filtered, self.filter.grouping);
        self.group_match_counts = self
            .visible_groups
            .iter()
            .map(|group| self.matcher.matches_group(group).len())
            .collect();
        if let Some(previous_selection) = previous_selection
            && let Some(index) = self
                .visible_groups
                .iter()
                .position(|group| group.id == previous_selection)
        {
            self.selected = index;
            return;
        }
        self.clamp_selection();
    }

    /// Build the command-grouped rows: each configured command paired with the
    /// distinct logical services that match at least one of its instances.
    fn recompute_command_groups(&mut self, filtered: &[ServiceRecord]) {
        let previous = self
            .command_groups
            .get(self.selected)
            .map(|group| group.command.name.clone());

        let service_groups = service::group_records(filtered, GroupingMode::LogicalService);
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
        if let Some(previous) = previous
            && let Some(index) = self
                .command_groups
                .iter()
                .position(|group| group.command.name == previous)
        {
            self.selected = index;
            return;
        }
        self.clamp_selection();
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
            AppMode::Grouping => {
                self.handle_grouping_key(key);
                Ok(None)
            }
            AppMode::ActionPicker => self.handle_action_picker_key(key),
            AppMode::InstancePicker => self.handle_instance_picker_key(key),
            AppMode::ServicePicker => self.handle_service_picker_key(key),
            AppMode::Help => {
                if self.keybindings.is("help", "close", key) {
                    self.mode = AppMode::Browse;
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
            _ if self.keybindings.is("browse", "grouping", key) => {
                self.grouping_index = GroupingMode::ALL
                    .iter()
                    .position(|mode| *mode == self.filter.grouping)
                    .unwrap_or(0);
                self.mode = AppMode::Grouping;
            }
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
            _ if self.keybindings.is("search", "close", key) => self.mode = AppMode::Browse,
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
            _ if self.keybindings.is("type_filter", "close", key) => self.mode = AppMode::Browse,
            _ if self.keybindings.is("type_filter", "down", key) => {
                if !types.is_empty() {
                    self.type_filter_index = (self.type_filter_index + 1).min(types.len() - 1);
                }
            }
            _ if self.keybindings.is("type_filter", "up", key) => {
                self.type_filter_index = self.type_filter_index.saturating_sub(1);
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

    fn handle_grouping_key(&mut self, key: KeyEvent) {
        match key.code {
            _ if self.keybindings.is("grouping", "close", key) => self.mode = AppMode::Browse,
            _ if self.keybindings.is("grouping", "down", key) => {
                self.grouping_index = (self.grouping_index + 1).min(GroupingMode::ALL.len() - 1);
            }
            _ if self.keybindings.is("grouping", "up", key) => {
                self.grouping_index = self.grouping_index.saturating_sub(1);
            }
            _ if self.keybindings.is("grouping", "select", key) => {
                self.filter.grouping = GroupingMode::ALL[self.grouping_index];
                self.mode = AppMode::Browse;
                self.recompute_visible();
            }
            _ => {}
        }
    }

    fn handle_action_picker_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        match key.code {
            _ if self.keybindings.is("picker", "close", key) => {
                self.mode = AppMode::Browse;
                self.action_matches.clear();
            }
            _ if self.keybindings.is("picker", "down", key) => {
                if !self.action_matches.is_empty() {
                    self.action_index = (self.action_index + 1).min(self.action_matches.len() - 1);
                }
            }
            _ if self.keybindings.is("picker", "up", key) => {
                self.action_index = self.action_index.saturating_sub(1);
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
            _ if self.keybindings.is("picker", "close", key) => {
                self.mode = AppMode::Browse;
                self.pending_action = None;
            }
            _ if self.keybindings.is("picker", "down", key) => {
                if count > 0 {
                    self.instance_index = (self.instance_index + 1).min(count - 1);
                }
            }
            _ if self.keybindings.is("picker", "up", key) => {
                self.instance_index = self.instance_index.saturating_sub(1);
            }
            _ if self.keybindings.is("picker", "select", key) => {
                let Some(action) = self.pending_action.clone() else {
                    self.mode = AppMode::Browse;
                    return Ok(None);
                };
                let Some(record) = action.matching_records.get(self.instance_index) else {
                    self.mode = AppMode::Browse;
                    return Ok(None);
                };
                return self.execute_action(&action, record);
            }
            _ => {}
        }
        Ok(None)
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.active_count();
        if count == 0 {
            self.selected = 0;
            return;
        }
        let selected = self.selected as isize + delta;
        self.selected = selected.clamp(0, count as isize - 1) as usize;
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
        service: &ServiceGroup,
    ) -> Result<Option<PreparedCommand>> {
        let Some(result) = self
            .matcher
            .matches_group(service)
            .into_iter()
            .find(|result| result.command.name == command.name)
        else {
            self.status = format!(
                "`{}` no longer matches `{}`",
                command.name, service.label
            );
            self.mode = AppMode::Browse;
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
            _ if self.keybindings.is("picker", "close", key) => {
                self.mode = AppMode::Browse;
            }
            _ if self.keybindings.is("picker", "down", key) => {
                if count > 0 {
                    self.service_picker_index = (self.service_picker_index + 1).min(count - 1);
                }
            }
            _ if self.keybindings.is("picker", "up", key) => {
                self.service_picker_index = self.service_picker_index.saturating_sub(1);
            }
            _ if self.keybindings.is("picker", "select", key) => {
                let Some(group) = self.command_groups.get(self.selected) else {
                    self.mode = AppMode::Browse;
                    return Ok(None);
                };
                let Some(service) = group.services.get(self.service_picker_index).cloned() else {
                    self.mode = AppMode::Browse;
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
            self.mode = AppMode::Browse;
            return Ok(None);
        };
        self.execute_action(&action, record)
    }

    fn execute_action(
        &mut self,
        action: &MatchResult,
        record: &ServiceRecord,
    ) -> Result<Option<PreparedCommand>> {
        let prepared = process::prepare(&action.command.action, record)?;
        match prepared.mode {
            ActionMode::Fork => {
                process::fork(&prepared)?;
                self.status = format!("launched `{}`", action.command.name);
                self.mode = AppMode::Browse;
                self.action_matches.clear();
                self.pending_action = None;
                Ok(None)
            }
            ActionMode::Execute => Ok(Some(prepared)),
        }
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::keymap::KeyBindings;

    fn test_cli() -> Cli {
        Cli {
            domain: "local".to_string(),
            config_dirs: Vec::new(),
            service_type: None,
            fake_discovery: true,
            command: crate::cli::CliCommand::Run,
        }
    }

    #[test]
    fn resolved_service_replaces_pending_record() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(test_cli(), Matcher::default(), KeyBindings::default(), rx);

        let pending = ServiceRecord::new("workstation", "_ssh._tcp", "local").with_instance_id();
        let mut resolved = ServiceRecord::new("workstation", "_ssh._tcp", "local");
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

        let mut alpha = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        alpha.hostname = Some("alpha.local".to_string());
        alpha.address = Some("192.168.1.10".parse().unwrap());
        alpha.port = Some(22);
        let mut beta = ServiceRecord::new("beta", "_ssh._tcp", "local");
        beta.hostname = Some("beta.local".to_string());
        beta.address = Some("192.168.1.11".parse().unwrap());
        beta.port = Some(22);
        let web = ServiceRecord::new("web", "_http._tcp", "local");

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

        let mut first = ServiceRecord::new("workstation", "_ssh._tcp", "local");
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
}
