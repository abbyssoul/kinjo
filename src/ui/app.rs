use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use color_eyre::eyre::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::{
    DefaultTerminal,
    layout::{Position, Rect},
};

use crate::{
    discovery::{
        BrowseMode, DiscoveryEvent, DiscoverySession, Entry, EntryGroup, EntryId, GroupingMode,
        RowHost, SessionPoll, SessionState, browse_groups, browse_row_count,
    },
    plumber::{ActionOutcome, CommandConfig, MatchResult, PreparedCommand, RuleEngine},
};

use super::{
    cli::Cli,
    filter::FilterState,
    keymap::{Action, KeyBindings, Mode as KeyMode},
    render,
};

/// Loads a fresh rule set for a config reload (SIGHUP). Injected by the
/// composition root so the app stays decoupled from config-file I/O.
pub type ConfigLoader = Box<dyn Fn(&Cli) -> Result<(Box<dyn RuleEngine>, Vec<String>)>>;

/// Starts a replacement discovery session for a service-list refresh.
///
/// Takes nothing: the composition root builds it around the same validated
/// discovery options the startup session used. A refresh therefore repeats that
/// browse exactly, and the app never handles — or could re-derive — unvalidated
/// discovery inputs.
pub type DiscoveryFactory = Box<dyn Fn() -> DiscoverySession>;

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

impl AppMode {
    /// The keybinding mode this UI mode resolves keys in. The three pickers
    /// differ only in what they list, so they share one set of bindings.
    pub fn key_mode(self) -> KeyMode {
        match self {
            AppMode::Browse => KeyMode::Browse,
            AppMode::Search => KeyMode::Search,
            AppMode::TypeFilter => KeyMode::TypeFilter,
            AppMode::ActionPicker | AppMode::InstancePicker | AppMode::ServicePicker => {
                KeyMode::Picker
            }
            AppMode::Help => KeyMode::Help,
        }
    }
}

pub struct App {
    pub cli: Cli,
    pub matcher: Box<dyn RuleEngine>,
    pub keybindings: KeyBindings,
    /// The running discovery session: its events, its state, and its shutdown.
    /// One value, so the receiver and the adapter behind it cannot drift apart;
    /// dropping or replacing it stops the producer.
    pub session: DiscoverySession,
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
    /// Set by the quit keybindings; the event loop exits when it is true.
    pub should_quit: bool,
    /// Set from the SIGHUP handler; the event loop polls it and reloads the
    /// command configs when it flips to true.
    pub reload_requested: Arc<AtomicBool>,
    /// Reloads command configs on request; reload is unavailable when unset.
    pub config_loader: Option<ConfigLoader>,
    /// Starts a replacement discovery session; refresh is unavailable when unset.
    pub discovery_factory: Option<DiscoveryFactory>,
    /// Per-group action matches, parallel to `visible_groups`. Computed once
    /// per recompute so rendering and invocation share one result instead of
    /// re-running the matcher (regexes included) every frame.
    pub group_matches: Vec<Vec<MatchResult>>,
    /// Sorted service types across all discovered records, recomputed with
    /// the visible groups (rendering reads it several times per frame).
    pub service_types: Vec<String>,
    /// How many rows each top-panel tab lists, in [`GroupingMode::TABS`] order.
    /// Recomputed with the visible rows from the same filtered records, so a
    /// tab's count always matches the list it would show.
    pub tab_counts: [usize; GroupingMode::TABS.len()],
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
    /// Screen rectangle of the left list panel, recorded by the renderer each
    /// frame so mouse events can be hit-tested against it.
    pub list_area: Cell<Rect>,
    /// Screen rectangle of the details pane, recorded by the renderer each frame.
    pub details_area: Cell<Rect>,
}

impl App {
    pub fn new(
        cli: Cli,
        matcher: impl RuleEngine + 'static,
        keybindings: KeyBindings,
        session: DiscoverySession,
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
            session,
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
            should_quit: false,
            reload_requested: Arc::new(AtomicBool::new(false)),
            config_loader: None,
            discovery_factory: None,
            group_matches: Vec::new(),
            service_types: Vec::new(),
            tab_counts: [0; GroupingMode::TABS.len()],
            ticks: 0,
            command_groups: Vec::new(),
            service_picker_index: 0,
            details_scroll: 0,
            details_max_scroll: Cell::new(0),
            details_viewport: Cell::new(0),
            list_area: Cell::new(Rect::default()),
            details_area: Cell::new(Rect::default()),
        }
    }

    /// Attach a factory for replacement discovery sessions, enabling the
    /// refresh command. The running session itself comes from [`App::new`].
    pub fn with_discovery_factory(mut self, factory: DiscoveryFactory) -> Self {
        self.discovery_factory = Some(factory);
        self
    }

    /// Attach a command-config loader, enabling reload-on-SIGHUP.
    pub fn with_config_loader(mut self, loader: ConfigLoader) -> Self {
        self.config_loader = Some(loader);
        self
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<PreparedCommand>> {
        let _mouse_capture = MouseCaptureGuard::enable();
        self.recompute_visible();

        loop {
            self.ticks = self.ticks.wrapping_add(1);
            self.poll_reload();
            self.drain_discovery();
            terminal.draw(|frame| render::render(frame, self))?;

            if event::poll(Duration::from_millis(120))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Release {
                            continue;
                        }
                        if let Some(command) = self.handle_key(key)? {
                            return Ok(Some(command));
                        }
                        if self.should_quit {
                            return Ok(None);
                        }
                    }
                    Event::Mouse(mouse) => self.handle_mouse(mouse),
                    _ => {}
                }
            }
        }
    }

    fn drain_discovery(&mut self) {
        let mut changed = false;
        loop {
            let event = match self.session.poll() {
                SessionPoll::Event(event) => event,
                SessionPoll::Idle => break,
                // The producer is gone. Reported once, so this reacts to the
                // ending rather than re-applying it on every tick.
                SessionPoll::Ended(state) => {
                    changed |= self.apply_session_end(&state);
                    break;
                }
            };
            match event {
                DiscoveryEvent::Upsert(record) => {
                    let id = record.id();
                    // A real occurrence supersedes the registration's
                    // unresolved placeholder, if one is still listed.
                    if !id.is_pending() {
                        self.records
                            .remove(&EntryId::pending(record.registration()));
                    }
                    self.records.insert(id, record);
                    changed = true;
                }
                DiscoveryEvent::Remove(id) => {
                    // Exactly the named occurrence: siblings of the same
                    // registration on other interfaces stay live.
                    changed |= self.records.remove(&id).is_some();
                }
                DiscoveryEvent::RemoveRegistration(registration) => {
                    let before = self.records.len();
                    self.records
                        .retain(|id, _| *id.registration() != registration);
                    changed |= self.records.len() != before;
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

    /// React to discovery ending, once.
    ///
    /// A real adapter's records are only worth showing while a live browse is
    /// confirming them: mDNS is edge-triggered, so once the producer is gone
    /// nothing will ever retract a service that has since died. Keeping the
    /// list up while labelling it current would invite the user to launch a
    /// command at a host that may no longer be there, so a failure clears it.
    ///
    /// A finite fake stream completing is the opposite case: it is the normal,
    /// expected ending, it never claimed to be watching the network, and its
    /// samples stay exactly as valid as they were. Returns whether the visible
    /// list needs recomputing.
    fn apply_session_end(&mut self, state: &SessionState) -> bool {
        match state {
            // Cannot be reached: the session only reports an ending once it has
            // left `Listening`.
            SessionState::Listening => false,
            SessionState::Complete => {
                self.status = format!(
                    "sample discovery complete | {} record(s)",
                    self.records.len()
                );
                false
            }
            SessionState::Failed(failure) => {
                // Pickers were computed from records that are about to go.
                self.close_pickers();
                let had_records = !self.records.is_empty();
                self.records.clear();
                // Set last: the cause must be what the user is left looking at.
                self.status = failure.message();
                had_records
            }
        }
    }

    fn recompute_visible(&mut self) {
        let records = self.records.values().cloned().collect::<Vec<_>>();
        self.service_types = FilterState::discovered_types(&records);
        self.filter.sync_service_types(&records);
        let filtered = self.filter.apply(&records);
        self.tab_counts = self.count_tabs(&filtered);

        // The command tab groups configured rules rather than projecting the
        // discovered entries, so it has its own row builder.
        let Some(browse) = self.filter.grouping.browse_mode() else {
            self.recompute_command_groups(&filtered);
            return;
        };

        self.command_groups = Vec::new();
        let previous = self
            .visible_groups
            .get(self.selected)
            .map(|group| group.id().clone());
        self.visible_groups = browse_groups(&filtered, browse);
        self.group_matches = self
            .visible_groups
            .iter()
            .map(|group| self.matcher.matches_group(group))
            .collect();
        // Structured row identity, not the list position: the cursor stays on
        // the row the user chose even when rows appear, vanish, or re-sort.
        match find_selection(&self.visible_groups, previous, |group| group.id().clone()) {
            Some(index) => self.selected = index,
            None => self.clamp_selection(),
        }
    }

    /// How many rows each tab would list, in [`GroupingMode::TABS`] order: the
    /// browse tabs count the rows of their projection of `filtered`, and the
    /// command tab counts the configured rules it lists.
    fn count_tabs(&self, filtered: &[Entry]) -> [usize; GroupingMode::TABS.len()] {
        GroupingMode::TABS.map(|mode| match mode.browse_mode() {
            Some(browse) => browse_row_count(filtered, browse),
            None => self.matcher.command_count(),
        })
    }

    /// Build the command-grouped rows: each configured command paired with the
    /// distinct logical services that match at least one of its instances.
    fn recompute_command_groups(&mut self, filtered: &[Entry]) {
        let previous = self
            .command_groups
            .get(self.selected)
            .map(|group| group.command.name.clone());

        let service_groups = browse_groups(filtered, BrowseMode::LogicalService);
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
        self.group_matches = Vec::new();
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

    /// Resolve the key to at most one action for the active mode, then act on
    /// it. Resolution is the keymap's job, so no handler re-checks keys and no
    /// binding can be shadowed by the order the handlers are written in.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<PreparedCommand>> {
        let action = self.keybindings.resolve(self.mode.key_mode(), key);
        if action == Some(Action::Quit) {
            self.should_quit = true;
            return Ok(None);
        }

        match self.mode {
            AppMode::Browse => self.handle_browse_key(action, key),
            AppMode::Search => {
                self.handle_search_key(action, key);
                Ok(None)
            }
            AppMode::TypeFilter => {
                self.handle_type_filter_key(action);
                Ok(None)
            }
            AppMode::ActionPicker => self.handle_action_picker_key(action),
            AppMode::InstancePicker => self.handle_instance_picker_key(action),
            AppMode::ServicePicker => self.handle_service_picker_key(action),
            AppMode::Help => {
                if action == Some(Action::HelpClose) {
                    self.return_to_browse();
                }
                Ok(None)
            }
        }
    }

    fn handle_browse_key(
        &mut self,
        action: Option<Action>,
        key: KeyEvent,
    ) -> Result<Option<PreparedCommand>> {
        match action {
            Some(Action::BrowseQuit) => self.should_quit = true,
            Some(Action::MoveDown) => self.move_selection(1),
            Some(Action::MoveUp) => self.move_selection(-1),
            Some(Action::Invoke) => return self.invoke_selected(),
            Some(Action::OpenSearch) => self.mode = AppMode::Search,
            Some(Action::OpenTypeFilter) => {
                self.type_filter_index = 0;
                self.mode = AppMode::TypeFilter;
            }
            Some(Action::TabNext) => self.cycle_tab(1),
            Some(Action::TabPrev) => self.cycle_tab(-1),
            Some(Action::SameHost) => self.toggle_same_host_filter(),
            Some(Action::Refresh) => self.refresh_services(),
            Some(Action::DetailsDown) => self.scroll_details(1),
            Some(Action::DetailsUp) => self.scroll_details(-1),
            Some(Action::OpenHelp) => self.mode = AppMode::Help,
            // Typing a character with nothing bound to it starts a search with
            // it, so the query never loses the keystroke that opened it.
            None => {
                if let Some(ch) = typed_char(key) {
                    self.filter.text_query.push(ch);
                    self.mode = AppMode::Search;
                    self.recompute_visible();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Search editing is append-only: the bound actions come first, and any key
    /// that is not bound falls through to the editor itself.
    fn handle_search_key(&mut self, action: Option<Action>, key: KeyEvent) {
        match action {
            // Leaving search keeps the query: it is the active filter, and
            // `clear` is the only thing that removes it.
            Some(Action::SearchClose) => self.return_to_browse(),
            Some(Action::SearchClear) => {
                self.filter.clear_text();
                self.recompute_visible();
            }
            None => match key.code {
                KeyCode::Backspace | KeyCode::Delete => {
                    self.filter.text_query.pop();
                    self.recompute_visible();
                }
                _ => {
                    if let Some(ch) = typed_char(key) {
                        self.filter.text_query.push(ch);
                        self.recompute_visible();
                    }
                }
            },
            _ => {}
        }
    }

    fn handle_type_filter_key(&mut self, action: Option<Action>) {
        let count = self.service_types.len();
        match action {
            Some(Action::TypeFilterClose) => self.return_to_browse(),
            Some(Action::TypeFilterDown) => {
                self.type_filter_index = move_index(self.type_filter_index, count, 1);
            }
            Some(Action::TypeFilterUp) => {
                self.type_filter_index = move_index(self.type_filter_index, count, -1);
            }
            Some(Action::TypeFilterToggle) => {
                if let Some(service_type) = self.service_types.get(self.type_filter_index).cloned()
                {
                    self.filter.toggle_service_type(&service_type);
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

    fn handle_action_picker_key(
        &mut self,
        action: Option<Action>,
    ) -> Result<Option<PreparedCommand>> {
        let len = self.action_matches.len();
        match action {
            Some(Action::PickerClose) => self.return_to_browse(),
            Some(Action::PickerDown) => {
                self.action_index = move_index(self.action_index, len, 1);
            }
            Some(Action::PickerUp) => {
                self.action_index = move_index(self.action_index, len, -1);
            }
            Some(Action::PickerSelect) => {
                if let Some(chosen) = self.action_matches.get(self.action_index).cloned() {
                    return self.choose_action(chosen);
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_instance_picker_key(
        &mut self,
        action: Option<Action>,
    ) -> Result<Option<PreparedCommand>> {
        let count = self
            .pending_action
            .as_ref()
            .map(|action| action.matching_records.len())
            .unwrap_or(0);
        match action {
            Some(Action::PickerClose) => self.return_to_browse(),
            Some(Action::PickerDown) => {
                self.instance_index = move_index(self.instance_index, count, 1);
            }
            Some(Action::PickerUp) => {
                self.instance_index = move_index(self.instance_index, count, -1);
            }
            Some(Action::PickerSelect) => {
                let Some(pending) = self.pending_action.clone() else {
                    self.return_to_browse();
                    return Ok(None);
                };
                let Some(record) = pending.matching_records.get(self.instance_index) else {
                    self.return_to_browse();
                    return Ok(None);
                };
                return self.execute_action(&pending, record);
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
        self.scroll_details_by(direction * step);
    }

    /// Scroll the details pane by `delta` lines, clamped to the content bounds.
    fn scroll_details_by(&mut self, delta: isize) {
        let max = self.details_max_scroll.get() as isize;
        self.details_scroll = (self.details_scroll as isize + delta).clamp(0, max) as usize;
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Modal pickers and help stay keyboard-driven; the mouse only drives
        // the browse layer (which remains visible while searching).
        if !matches!(self.mode, AppMode::Browse | AppMode::Search) {
            return;
        }
        let position = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.mouse_scroll(position, 1),
            MouseEventKind::ScrollUp => self.mouse_scroll(position, -1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = self.list_row_at(position) {
                    self.selected = index;
                    // A different row is in focus — start its details from the top.
                    self.details_scroll = 0;
                }
            }
            _ => {}
        }
    }

    /// A wheel event over the details pane scrolls its content; over the list
    /// it moves the selection (the list window follows the selected row).
    fn mouse_scroll(&mut self, position: Position, direction: isize) {
        if self.details_area.get().contains(position) {
            self.scroll_details_by(direction);
        } else if self.list_area.get().contains(position) {
            self.move_selection(direction);
        }
    }

    /// The list index under `position`, when it falls on a rendered row of the
    /// left panel — accounting for the panel border and the scroll window the
    /// renderer derives from the current selection.
    fn list_row_at(&self, position: Position) -> Option<usize> {
        let area = self.list_area.get();
        if !area.contains(position) {
            return None;
        }
        // First content row sits below the top border; the bottom border and
        // anything past the last row are not selectable.
        let row = (position.y.checked_sub(area.y + 1))? as usize;
        let inner_h = area.height.saturating_sub(2) as usize;
        if row >= inner_h {
            return None;
        }
        let total = self.active_count();
        let index = render::scroll_offset(self.selected, total, inner_h) + row;
        (index < total).then_some(index)
    }

    fn invoke_selected(&mut self) -> Result<Option<PreparedCommand>> {
        if self.filter.grouping == GroupingMode::Command {
            return self.invoke_command();
        }
        let Some(group) = self.visible_groups.get(self.selected) else {
            self.status = "no service selected".to_string();
            return Ok(None);
        };
        let matches = self
            .group_matches
            .get(self.selected)
            .cloned()
            .unwrap_or_default();
        match matches.len() {
            0 => {
                self.status = format!("no configured actions match `{}`", group.label());
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
            self.status = format!("`{}` no longer matches `{}`", command.name, service.label());
            self.return_to_browse();
            return Ok(None);
        };
        self.choose_action(result)
    }

    fn handle_service_picker_key(
        &mut self,
        action: Option<Action>,
    ) -> Result<Option<PreparedCommand>> {
        let count = self
            .command_groups
            .get(self.selected)
            .map(|group| group.services.len())
            .unwrap_or(0);
        match action {
            Some(Action::PickerClose) => self.return_to_browse(),
            Some(Action::PickerDown) => {
                self.service_picker_index = move_index(self.service_picker_index, count, 1);
            }
            Some(Action::PickerUp) => {
                self.service_picker_index = move_index(self.service_picker_index, count, -1);
            }
            Some(Action::PickerSelect) => {
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

        // The rule owns what running it means — checking its requirements,
        // building the argv, and honouring its mode. The UI only decides what a
        // person sees afterwards.
        match action.command.run(record) {
            Ok(ActionOutcome::Forked) => {
                self.status = format!("launched `{name}`");
                self.return_to_browse();
                Ok(None)
            }
            // The execute hand-off happens after the TUI is torn down, so the
            // caller takes ownership of the prepared command from here.
            Ok(ActionOutcome::Handoff(prepared)) => Ok(Some(prepared)),
            Err(err) => self.fail(format!("cannot run `{name}`: {err}")),
        }
    }

    /// Perform the config reload when one was requested (SIGHUP).
    fn poll_reload(&mut self) {
        if self.reload_requested.swap(false, Ordering::Relaxed) {
            self.reload_config();
        }
    }

    /// Reload command configs through the injected loader, keeping the current
    /// rule set when the reload fails. A reload failure must not tear down the
    /// TUI — it is reported on the status line like action failures.
    fn reload_config(&mut self) {
        let Some(loader) = &self.config_loader else {
            self.status = "config reload is not available".to_string();
            return;
        };
        match loader(&self.cli) {
            Ok((matcher, warnings)) => {
                self.matcher = matcher;
                self.close_pickers();
                self.recompute_visible();
                self.status = match warnings.len() {
                    0 => format!("reloaded {} command(s)", self.matcher.command_count()),
                    skipped => format!(
                        "reloaded {} command(s); skipped {skipped} config file(s)",
                        self.matcher.command_count()
                    ),
                };
            }
            Err(err) => self.status = format!("config reload failed: {err}"),
        }
    }

    /// Restart discovery from scratch: the list empties and repopulates as the
    /// fresh browse reports services, exactly like app startup. Filters and
    /// the active view are kept; they describe what the user wants to see,
    /// not what has been seen.
    fn refresh_services(&mut self) {
        let Some(factory) = &self.discovery_factory else {
            self.status = "refresh is not available".to_string();
            return;
        };
        // Stop the old producer (cancel + join) before starting the
        // replacement, so two browsers never feed the link at once.
        self.session.shutdown();
        let replacement = factory();

        // Only now that a replacement exists do the old session and its records
        // go. The old session's receiver is dropped with it, so its events can
        // never arrive on the new list — old and new cannot mix.
        self.session = replacement;
        self.records.clear();
        self.close_pickers();
        self.selected = 0;
        self.details_scroll = 0;
        self.recompute_visible();
        self.status = "refreshing: service discovery restarted".to_string();
    }

    /// Close any open picker: its entries were computed from the pre-reload
    /// rule set or the pre-refresh service list. Search and help stay open —
    /// they do not cache matcher or record data.
    fn close_pickers(&mut self) {
        if matches!(
            self.mode,
            AppMode::ActionPicker | AppMode::InstancePicker | AppMode::ServicePicker
        ) {
            self.return_to_browse();
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

    /// Narrow the list to the selected row's host, or clear an active narrowing.
    ///
    /// Only a projection whose rows have one invariant hostname can offer this:
    /// the logical-service and host views. The service-type view's rows span
    /// several hosts and the command view lists rules rather than discovered
    /// entries, so both report the filter unavailable instead of silently
    /// filtering by some child's host.
    fn toggle_same_host_filter(&mut self) {
        if self.filter.host_filter.is_some() {
            self.filter.clear_host_filter();
            self.status = "host filter cleared".to_string();
            self.recompute_visible();
            return;
        }
        // A `&'static str`, so the messages below borrow nothing from `self`.
        let view = self.filter.grouping.label();
        let unavailable =
            |reason| format!("same-host filter is unavailable in the {view} view: {reason}");
        if self.filter.grouping.browse_mode().is_none() {
            self.status = unavailable("it lists commands, not discovered services");
            return;
        }
        let Some(group) = self.visible_groups.get(self.selected) else {
            self.status = "no row selected".to_string();
            return;
        };
        match group.facts().host() {
            RowHost::Resolved(host) => {
                let host = host.to_string();
                self.status = format!("filtering by host `{host}`");
                self.filter.set_host_filter(host);
                self.recompute_visible();
            }
            RowHost::Unresolved => {
                self.status = "selected row has no resolved host yet".to_string();
            }
            RowHost::Varies => {
                self.status = unavailable("its rows span several hosts");
            }
        }
    }
}

/// Enables terminal mouse reporting for its lifetime. Dropping it — including
/// during unwinding — restores the terminal's native mouse handling (text
/// selection, scrollback) before the rest of the terminal state is torn down.
struct MouseCaptureGuard;

impl MouseCaptureGuard {
    fn enable() -> Self {
        let _ = crossterm::execute!(std::io::stdout(), event::EnableMouseCapture);
        Self
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), event::DisableMouseCapture);
    }
}

/// Move a list cursor by `delta`, clamped to `[0, len-1]` (or 0 when empty).
fn move_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).clamp(0, len as isize - 1) as usize
}

/// The character an unbound key event types into the search query, if any.
///
/// A modified key is a shortcut, not text: crossterm reports Ctrl-X as
/// `Char('x')` with CONTROL set, so an unbound control chord would otherwise
/// silently type its letter. SHIFT is not excluded — it is already folded into
/// the character itself.
fn typed_char(key: KeyEvent) -> Option<char> {
    let modified = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
        KeyCode::Char(ch) if !modified && !ch.is_control() => Some(ch),
        _ => None,
    }
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
    use std::{net::IpAddr, num::NonZeroU32, sync::mpsc};

    use super::*;
    use crate::ui::keymap::KeyBindings;
    use crate::{
        discovery::{OccurrenceId, Registration, RowServiceType, UNRESOLVED_HOST_LABEL},
        plumber::Matcher,
    };

    fn test_cli() -> Cli {
        Cli {
            domain: "local".to_string(),
            config_dirs: Vec::new(),
            service_type: None,
            fake_discovery: true,
            backend: crate::discovery::DiscoveryBackend::default(),
            command: crate::ui::cli::CliCommand::Run,
        }
    }

    #[test]
    fn resolved_service_replaces_pending_record() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        let pending = Entry::new("workstation", "_ssh._tcp", "local");
        let mut resolved = Entry::new("workstation", "_ssh._tcp", "local");
        resolved.hostname = Some("workstation.local".to_string());
        resolved.addresses = vec!["192.168.1.20".parse().unwrap()];
        resolved.port = Some(22);

        tx.send(DiscoveryEvent::Upsert(pending)).unwrap();
        tx.send(DiscoveryEvent::Upsert(resolved)).unwrap();

        app.drain_discovery();

        assert_eq!(app.records.len(), 1);
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances().len(), 1);
        assert_eq!(
            app.visible_groups[0].facts().host(),
            RowHost::Resolved("workstation.local")
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
        let mut app = App::new(
            test_cli(),
            matcher,
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        let mut alpha = Entry::new("alpha", "_ssh._tcp", "local");
        alpha.hostname = Some("alpha.local".to_string());
        alpha.addresses = vec!["192.168.1.10".parse().unwrap()];
        alpha.port = Some(22);
        let mut beta = Entry::new("beta", "_ssh._tcp", "local");
        beta.hostname = Some("beta.local".to_string());
        beta.addresses = vec!["192.168.1.11".parse().unwrap()];
        beta.port = Some(22);
        let web = Entry::new("web", "_http._tcp", "local");

        tx.send(DiscoveryEvent::Upsert(alpha)).unwrap();
        tx.send(DiscoveryEvent::Upsert(beta)).unwrap();
        tx.send(DiscoveryEvent::Upsert(web)).unwrap();
        app.drain_discovery();

        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();

        assert_eq!(app.command_groups.len(), 1);
        assert_eq!(app.command_groups[0].command.name, "ssh");
        // alpha + beta are distinct logical services; the http service is excluded.
        assert_eq!(app.command_groups[0].services.len(), 2);
    }

    #[test]
    fn multiple_resolved_addresses_collapse_onto_one_service() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        let mut service = Entry::new("workstation", "_ssh._tcp", "local");
        service.hostname = Some("workstation.local".to_string());
        service.addresses = vec![
            "192.168.1.20".parse().unwrap(),
            "192.168.1.21".parse().unwrap(),
        ];
        service.port = Some(22);

        tx.send(DiscoveryEvent::Upsert(service)).unwrap();

        app.drain_discovery();

        // One logical service that carries both of its addresses.
        assert_eq!(app.records.len(), 1);
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances().len(), 1);
        assert_eq!(app.visible_groups[0].instances()[0].addresses.len(), 2);
    }

    // ── interaction harness ────────────────────────────────────────────────
    use crate::plumber::MatcherBuilder;
    use crate::test_support::{remove, temp_file};
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
        // Inert: these tests are about what the app does with records it
        // already has, not about the session producing or ending them.
        let mut app = App::new(
            test_cli(),
            matcher,
            KeyBindings::default(),
            DiscoverySession::inert(),
        );
        for record in records {
            app.records.insert(record.id(), record);
        }
        app.recompute_visible();
        app
    }

    fn ssh(name: &str, addr: &str) -> Entry {
        let mut record = Entry::new(name, "_ssh._tcp", "local");
        record.hostname = Some(format!("{name}.local"));
        record.addresses = vec![addr.parse().unwrap()];
        record.port = Some(22);
        record
    }

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    /// An app with `count` distinct services and the pane rectangles a render
    /// pass would have recorded: a bordered list panel at y = 2..=8 whose five
    /// content rows sit at y = 3..=7, and the details pane to its right.
    fn mouse_app(count: usize) -> App {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let records = names[..count]
            .iter()
            .map(|name| ssh(name, "10.0.0.1"))
            .collect();
        let app = app_with(Matcher::default(), records);
        app.list_area.set(Rect::new(0, 2, 60, 7));
        app.details_area.set(Rect::new(60, 2, 40, 7));
        app
    }

    #[test]
    fn wheel_over_list_moves_the_selection() {
        let mut app = mouse_app(8);
        assert_eq!(app.selected, 0);

        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 5, 4));
        assert_eq!(app.selected, 1);

        app.handle_mouse(mouse_event(MouseEventKind::ScrollUp, 5, 4));
        app.handle_mouse(mouse_event(MouseEventKind::ScrollUp, 5, 4));
        assert_eq!(app.selected, 0, "selection clamps at the top");

        // A wheel event outside both panes does nothing.
        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 5, 0));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn wheel_over_details_scrolls_content_line_by_line() {
        let mut app = mouse_app(8);
        app.details_max_scroll.set(3);

        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        assert_eq!(app.details_scroll, 2);

        // Clamped to the content bounds on both ends.
        for _ in 0..5 {
            app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        }
        assert_eq!(app.details_scroll, 3);
        for _ in 0..9 {
            app.handle_mouse(mouse_event(MouseEventKind::ScrollUp, 70, 4));
        }
        assert_eq!(app.details_scroll, 0);

        // Scrolling the details never moves the list selection.
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn click_selects_the_row_under_the_cursor() {
        let mut app = mouse_app(8);

        // Clicking the fourth visible content row (y = 6) selects index 3.
        app.handle_mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 6));
        assert_eq!(app.selected, 3);

        // With the selection at the end the window shows indices 3..=7, so
        // the first visible row (y = 3) is index 3.
        app.selected = 7;
        app.handle_mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 3));
        assert_eq!(app.selected, 3);
    }

    #[test]
    fn clicks_on_borders_and_empty_rows_are_ignored() {
        let mut app = mouse_app(8);
        app.selected = 2;

        // Top border, bottom border, and the details pane.
        for (x, y) in [(5, 2), (5, 8), (70, 4)] {
            app.handle_mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), x, y));
            assert_eq!(app.selected, 2, "click at ({x},{y}) must not select");
        }

        // A row below the last record is not selectable.
        let mut small = mouse_app(2);
        small.selected = 1;
        small.handle_mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 6));
        assert_eq!(small.selected, 1);
    }

    #[test]
    fn mouse_is_ignored_in_modal_modes() {
        let mut app = mouse_app(8);
        app.mode = AppMode::Help;

        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 5, 4));
        app.handle_mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 6));

        assert_eq!(app.selected, 0);
    }

    /// An SSH service reachable at several addresses (load-balanced).
    fn ssh_multi(name: &str, addrs: &[&str]) -> Entry {
        let mut record = ssh(name, addrs[0]);
        record.addresses = addrs.iter().map(|a| a.parse().unwrap()).collect();
        record
    }

    fn http(name: &str) -> Entry {
        let mut record = Entry::new(name, "_http._tcp", "local");
        record.hostname = Some(format!("{name}.local"));
        record.addresses = vec!["192.168.1.50".parse().unwrap()];
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
        assert_eq!(app.visible_groups[0].label(), "zulu");
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

    /// Search is append-only, so Delete has nothing after a cursor to remove;
    /// it removes the last character exactly like Backspace, which is what the
    /// keybindings documentation promises.
    #[test]
    fn delete_removes_the_last_search_character_like_backspace() {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        send(&mut app, KeyCode::Char('z'));
        send(&mut app, KeyCode::Char('u'));
        assert_eq!(app.filter.text_query, "zu");

        send(&mut app, KeyCode::Delete);
        assert_eq!(app.filter.text_query, "z");

        send(&mut app, KeyCode::Delete);
        assert_eq!(app.filter.text_query, "");

        // Deleting past the start is a no-op rather than an error.
        send(&mut app, KeyCode::Delete);
        assert_eq!(app.filter.text_query, "");
        assert_eq!(app.mode, AppMode::Search, "editing stays open");
    }

    /// Deleting re-filters the list; a stale row set would misreport the query.
    #[test]
    fn deleting_a_character_recomputes_the_visible_rows() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("zulu", "10.0.0.2")],
        );

        send(&mut app, KeyCode::Char('z'));
        assert_eq!(app.visible_groups.len(), 1);

        send(&mut app, KeyCode::Delete);
        assert_eq!(app.filter.text_query, "");
        assert_eq!(app.visible_groups.len(), 2, "both rows are visible again");
    }

    /// Escape and Enter leave editing but the query is the active filter: it
    /// survives, and only the clear action removes it.
    #[test]
    fn escape_and_enter_close_search_but_keep_the_query() {
        for close in [KeyCode::Esc, KeyCode::Enter] {
            let mut app = app_with(
                Matcher::default(),
                vec![ssh("alpha", "10.0.0.1"), ssh("zulu", "10.0.0.2")],
            );
            send(&mut app, KeyCode::Char('z'));
            assert_eq!(app.mode, AppMode::Search);

            send(&mut app, close);

            assert_eq!(app.mode, AppMode::Browse, "{close:?} leaves search");
            assert_eq!(
                app.filter.text_query, "z",
                "{close:?} must keep the active query"
            );
            assert_eq!(
                app.visible_groups.len(),
                1,
                "{close:?} keeps the list filtered"
            );
        }
    }

    /// The configured clear action is the only full clear, and it leaves the
    /// user in search so they can immediately type a new query.
    #[test]
    fn clear_empties_the_query_and_restores_every_row() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("zulu", "10.0.0.2")],
        );
        send(&mut app, KeyCode::Char('z'));
        assert_eq!(app.visible_groups.len(), 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .unwrap();

        assert_eq!(app.filter.text_query, "");
        assert_eq!(app.mode, AppMode::Search, "clearing stays in search");
        assert_eq!(app.visible_groups.len(), 2);
    }

    /// A rebound clear must work and the default `ctrl-u` must stop working,
    /// otherwise the keymap is not really in charge of the search editor.
    #[test]
    fn a_rebound_clear_replaces_the_default_clear_key() {
        let path = temp_file(
            "search-clear",
            r#"
[search]
clear = ["ctrl-k"]
"#,
        );
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        app.keybindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();
        send(&mut app, KeyCode::Char('z'));

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.filter.text_query, "z", "ctrl-u no longer clears");

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.filter.text_query, "");

        remove(&path);
    }

    /// An unbound control chord is a shortcut that did nothing, not text: it
    /// must not silently type its letter into the query.
    #[test]
    fn an_unbound_control_chord_does_not_type_into_the_search_query() {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        send(&mut app, KeyCode::Char('z'));

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
            .unwrap();

        assert_eq!(app.filter.text_query, "z");
    }

    /// Shift is folded into the character, so capitals must still type.
    #[test]
    fn shifted_characters_type_into_the_search_query() {
        let mut app = app_with(Matcher::default(), vec![ssh("Zulu", "10.0.0.2")]);

        app.handle_key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT))
            .unwrap();

        assert_eq!(app.mode, AppMode::Search);
        assert_eq!(app.filter.text_query, "Z");
    }

    /// Rebinding one action must not disturb the others in its mode: the whole
    /// point of resolving a key to a single action.
    #[test]
    fn rebound_browse_keys_dispatch_and_defaults_stop_working() {
        let path = temp_file(
            "browse-rebind",
            r#"
[browse]
down = ["n"]
up = ["p"]
help = ["f1"]
"#,
        );
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        app.keybindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        send(&mut app, KeyCode::Char('n'));
        assert_eq!(app.selected, 1, "the rebound down key moves the cursor");

        send(&mut app, KeyCode::Char('p'));
        assert_eq!(app.selected, 0);

        // `j` is no longer navigation, so it falls through to typing a search.
        send(&mut app, KeyCode::Char('j'));
        assert_eq!(app.mode, AppMode::Search);
        assert_eq!(app.filter.text_query, "j");
        send(&mut app, KeyCode::Esc);

        send(&mut app, KeyCode::F(1));
        assert_eq!(app.mode, AppMode::Help, "the rebound help key opens help");

        remove(&path);
    }

    /// Unbinding an action leaves its key inert rather than falling back to a
    /// default that the configuration deliberately removed.
    #[test]
    fn an_unbound_browse_action_does_nothing() {
        let path = temp_file(
            "unbind-same-host",
            r#"
[browse]
same_host = []
"#,
        );
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        app.keybindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        send(&mut app, KeyCode::Char('s'));

        // `s` is unbound in browse, so it types instead of filtering by host.
        assert_eq!(app.filter.host_filter, None);
        assert_eq!(app.mode, AppMode::Search);

        remove(&path);
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
                .all(|g| g.facts().service_type() == RowServiceType::Invariant("_ssh._tcp"))
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
    fn instance_picker_disambiguates_then_executes_chosen_address() {
        // One logical service reachable at two addresses; an address-specific
        // command expands them into per-address candidates to pick between.
        let mut app = app_with(
            matcher_from(&[PING_ADDR]),
            vec![ssh_multi("alpha", &["10.0.0.1", "10.0.0.2"])],
        );
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances().len(), 1);
        assert_eq!(app.visible_groups[0].instances()[0].addresses.len(), 2);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::InstancePicker);

        // Candidates follow address order: index 1 is 10.0.0.2.
        send(&mut app, KeyCode::Down);
        let command = send(&mut app, KeyCode::Enter).expect("instance chosen");
        assert_eq!(command.argv, vec!["echo", "10.0.0.2"]);
    }

    // ── mode-aware aggregate views ─────────────────────────────────────────

    /// A service on `host` with its own type and port.
    fn service_on(name: &str, service_type: &str, host: &str, port: u16) -> Entry {
        let mut record = Entry::new(name, service_type, "local");
        record.hostname = Some(host.to_string());
        record.addresses = vec!["10.0.0.1".parse().unwrap()];
        record.port = Some(port);
        record
    }

    /// The tab count for `mode`, by its position in the tab bar.
    fn tab_count(app: &App, mode: GroupingMode) -> usize {
        let index = GroupingMode::TABS
            .iter()
            .position(|tab| *tab == mode)
            .expect("a tab for the mode");
        app.tab_counts[index]
    }

    #[test]
    fn tab_counts_follow_their_exact_definitions() {
        let app = app_with(
            matcher_from(&[SSH, PING]),
            vec![
                // Two services on one host, one on another, one unresolved.
                service_on("shell", "_ssh._tcp", "nas.local", 22),
                service_on("site", "_http._tcp", "nas.local", 80),
                service_on("shell", "_ssh._tcp", "pi.local", 22),
                Entry::new("ghost", "_ipp._tcp", "local"),
            ],
        );

        // Logical-service rows, not the occurrences behind them.
        assert_eq!(tab_count(&app, GroupingMode::LogicalService), 4);
        // Resolved host rows plus the single unresolved row.
        assert_eq!(tab_count(&app, GroupingMode::Host), 3);
        // Distinct service types.
        assert_eq!(tab_count(&app, GroupingMode::ServiceType), 3);
        // Configured rules, whatever was discovered.
        assert_eq!(tab_count(&app, GroupingMode::Command), 2);
    }

    #[test]
    fn every_tab_count_matches_the_rows_that_tab_lists() {
        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![
                service_on("shell", "_ssh._tcp", "nas.local", 22),
                service_on("site", "_http._tcp", "nas.local", 80),
                service_on("shell", "_ssh._tcp", "pi.local", 22),
                Entry::new("ghost", "_ipp._tcp", "local"),
            ],
        );

        for mode in GroupingMode::TABS {
            app.filter.grouping = mode;
            app.recompute_visible();
            let rows = if mode == GroupingMode::Command {
                app.command_groups.len()
            } else {
                app.visible_groups.len()
            };
            assert_eq!(tab_count(&app, mode), rows, "{mode:?} count vs its rows");
        }
    }

    #[test]
    fn a_host_row_offers_its_services_without_borrowing_one_childs_metadata() {
        // One host offering SSH and HTTP on different ports.
        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![
                service_on("shell", "_ssh._tcp", "nas.local", 22),
                service_on("site", "_http._tcp", "nas.local", 80),
            ],
        );
        app.filter.grouping = GroupingMode::Host;
        app.recompute_visible();

        assert_eq!(app.visible_groups.len(), 1);
        let host = &app.visible_groups[0];
        assert_eq!(host.label(), "nas.local");
        // The row states the host; the differing types stay on the children.
        assert_eq!(host.facts().host(), RowHost::Resolved("nas.local"));
        assert_eq!(host.facts().service_type(), RowServiceType::Varies);
        assert_eq!(host.logical_service_count(), 2);

        // Invoking the aggregate runs the command against the concrete child
        // that matches it, not against the row.
        let command = send(&mut app, KeyCode::Enter).expect("the ssh child runs");
        assert_eq!(command.argv, vec!["ssh", "nas.local"]);
    }

    #[test]
    fn a_service_type_row_targets_the_concrete_child_the_user_picks() {
        // One type offered by two hosts with different addresses and ports.
        let mut alpha = service_on("alpha", "_ssh._tcp", "alpha.local", 22);
        alpha.addresses = vec!["10.0.0.1".parse().unwrap()];
        let mut beta = service_on("beta", "_ssh._tcp", "beta.local", 2222);
        beta.addresses = vec!["10.0.0.2".parse().unwrap()];

        let mut app = app_with(matcher_from(&[PING_ADDR]), vec![alpha, beta]);
        app.filter.grouping = GroupingMode::ServiceType;
        app.recompute_visible();

        assert_eq!(app.visible_groups.len(), 1);
        let by_type = &app.visible_groups[0];
        assert_eq!(by_type.label(), "_ssh._tcp");
        // No host is type-wide, so the row names none.
        assert_eq!(by_type.facts().host(), RowHost::Varies);
        assert_eq!(by_type.resolved_host_count(), 2);

        // The rule needs a concrete address, so the aggregate offers up its
        // children rather than answering for them.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::InstancePicker);
        send(&mut app, KeyCode::Down);
        let command = send(&mut app, KeyCode::Enter).expect("the chosen child runs");
        assert_eq!(command.argv, vec!["echo", "10.0.0.2"]);
    }

    #[test]
    fn a_command_row_runs_against_a_concrete_service() {
        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![
                service_on("alpha", "_ssh._tcp", "alpha.local", 22),
                service_on("beta", "_ssh._tcp", "beta.local", 22),
            ],
        );
        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();

        assert_eq!(app.command_groups[0].services.len(), 2);
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::ServicePicker);
        let command = send(&mut app, KeyCode::Enter).expect("the picked service runs");
        assert_eq!(command.argv, vec!["ssh", "alpha.local"]);
    }

    #[test]
    fn same_host_filter_is_offered_only_by_invariant_host_projections() {
        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![
                service_on("shell", "_ssh._tcp", "nas.local", 22),
                service_on("site", "_http._tcp", "pi.local", 80),
            ],
        );

        // A host row has one hostname by construction: the filter applies.
        app.filter.grouping = GroupingMode::Host;
        app.recompute_visible();
        send(&mut app, KeyCode::Char('s'));
        assert_eq!(app.filter.host_filter.as_deref(), Some("nas.local"));
        send(&mut app, KeyCode::Char('s'));
        assert!(app.filter.host_filter.is_none());

        // A service-type row spans hosts: the filter must say so rather than
        // silently filter by whichever child happens to sort first.
        app.filter.grouping = GroupingMode::ServiceType;
        app.recompute_visible();
        send(&mut app, KeyCode::Char('s'));
        assert!(app.filter.host_filter.is_none(), "no host was filtered by");
        assert!(app.status.contains("unavailable"), "status: {}", app.status);
        assert!(app.status.contains("span several hosts"));

        // The command view lists rules, not discovered services.
        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();
        send(&mut app, KeyCode::Char('s'));
        assert!(app.filter.host_filter.is_none());
        assert!(app.status.contains("unavailable"));
    }

    #[test]
    fn an_active_host_filter_can_be_cleared_from_any_view() {
        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![service_on("shell", "_ssh._tcp", "nas.local", 22)],
        );
        send(&mut app, KeyCode::Char('s'));
        assert_eq!(app.filter.host_filter.as_deref(), Some("nas.local"));

        // Clearing describes the filter, not the row under the cursor, so the
        // views that cannot set one can still lift it.
        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();
        send(&mut app, KeyCode::Char('s'));

        assert!(app.filter.host_filter.is_none());
        assert!(app.status.contains("host filter cleared"));
    }

    #[test]
    fn an_unresolved_host_row_never_collides_with_the_sentinel_hostname() {
        let mut impostor = Entry::new("impostor", "_ssh._tcp", "local");
        impostor.hostname = Some(UNRESOLVED_HOST_LABEL.to_string());
        impostor.port = Some(22);

        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![impostor, Entry::new("ghost", "_ipp._tcp", "local")],
        );
        app.filter.grouping = GroupingMode::Host;
        app.recompute_visible();

        // Two rows reading alike: the impostor's resolved host, and the row of
        // registrations that have resolved none.
        assert_eq!(app.visible_groups.len(), 2);
        assert!(
            app.visible_groups
                .iter()
                .all(|group| group.label() == UNRESOLVED_HOST_LABEL)
        );

        // The impostor is a real host, so it can be filtered by.
        send(&mut app, KeyCode::Char('s'));
        assert_eq!(
            app.filter.host_filter.as_deref(),
            Some(UNRESOLVED_HOST_LABEL)
        );
        assert_eq!(app.visible_groups.len(), 1);
        send(&mut app, KeyCode::Char('s'));

        // The unresolved row is not a host and has nothing to filter by.
        app.selected = 1;
        send(&mut app, KeyCode::Char('s'));
        assert!(app.filter.host_filter.is_none());
        assert!(
            app.status.contains("no resolved host"),
            "status: {}",
            app.status
        );
    }

    #[test]
    fn selection_survives_recomputation_by_structured_row_identity() {
        let mut app = app_with(
            matcher_from(&[SSH]),
            vec![
                service_on("shell", "_ssh._tcp", "nas.local", 22),
                service_on("shell", "_ssh._tcp", "pi.local", 22),
            ],
        );
        // Two rows labelled `shell`; the cursor sits on the second.
        send(&mut app, KeyCode::Down);
        let chosen = app.visible_groups[app.selected].id().clone();

        // A new row sorts in ahead of the selection.
        let earlier = service_on("alpha", "_ssh._tcp", "alpha.local", 22);
        app.records.insert(earlier.id(), earlier);
        app.recompute_visible();

        assert_eq!(app.selected, 2, "the cursor followed its row");
        assert_eq!(*app.visible_groups[app.selected].id(), chosen);
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

    /// Two occurrences of one registration — the same service announced on two
    /// interfaces — identical but for the occurrence name and address.
    fn on_interface(name: &str, addr: &str, index: u32) -> Entry {
        let mut record = ssh(name, addr);
        record.addresses = vec![addr.parse().unwrap()];
        record.with_occurrence(Some(OccurrenceId(NonZeroU32::new(index).unwrap())))
    }

    #[test]
    fn occurrences_of_one_registration_coexist_with_their_own_addresses() {
        let mut app = app_with(
            Matcher::default(),
            vec![
                on_interface("alpha", "10.0.0.1", 1),
                on_interface("alpha", "10.0.0.2", 2),
            ],
        );
        app.recompute_visible();

        // Same name/type/domain/host/port: only the interface and address
        // differ, so neither occurrence may overwrite the other.
        assert_eq!(app.records.len(), 2);
        let addresses: Vec<_> = app
            .records
            .values()
            .flat_map(|record| record.addresses.clone())
            .collect();
        assert!(addresses.contains(&"10.0.0.1".parse().unwrap()));
        assert!(addresses.contains(&"10.0.0.2".parse().unwrap()));

        // They still read as one logical service.
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].instances().len(), 2);
    }

    #[test]
    fn removing_one_occurrence_preserves_its_sibling() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        tx.send(DiscoveryEvent::Upsert(on_interface("alpha", "10.0.0.1", 1)))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(on_interface("alpha", "10.0.0.2", 2)))
            .unwrap();
        app.drain_discovery();
        assert_eq!(app.records.len(), 2);

        tx.send(DiscoveryEvent::Remove(
            on_interface("alpha", "10.0.0.1", 1).id(),
        ))
        .unwrap();
        app.drain_discovery();

        // The logical service survives with the live occurrence's address.
        assert_eq!(app.records.len(), 1);
        assert_eq!(app.visible_groups.len(), 1);
        let survivor = app.records.values().next().expect("surviving occurrence");
        assert_eq!(
            survivor.addresses,
            vec!["10.0.0.2".parse::<IpAddr>().unwrap()]
        );
    }

    #[test]
    fn registration_removal_clears_every_occurrence() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        tx.send(DiscoveryEvent::Upsert(on_interface("alpha", "10.0.0.1", 1)))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(on_interface("alpha", "10.0.0.2", 2)))
            .unwrap();
        // An occurrence with no adapter name at all, as a zeroconf upsert has.
        tx.send(DiscoveryEvent::Upsert(ssh("alpha", "10.0.0.3")))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(ssh("beta", "10.0.0.4")))
            .unwrap();
        app.drain_discovery();
        // Three coexisting occurrences of `alpha`, plus `beta`.
        assert_eq!(app.records.len(), 4);

        // The fallback an adapter uses when it cannot name what it lost.
        tx.send(DiscoveryEvent::RemoveRegistration(Registration::new(
            "alpha",
            "_ssh._tcp",
            "local",
        )))
        .unwrap();
        app.drain_discovery();

        assert_eq!(app.records.len(), 1);
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].label(), "beta");
    }

    #[test]
    fn removing_an_unknown_occurrence_leaves_the_registration_alone() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        tx.send(DiscoveryEvent::Upsert(on_interface("alpha", "10.0.0.1", 1)))
            .unwrap();
        app.drain_discovery();

        // A removal naming an occurrence that was never listed must not be
        // widened into "remove the registration".
        tx.send(DiscoveryEvent::Remove(
            on_interface("alpha", "10.0.0.9", 7).id(),
        ))
        .unwrap();
        app.drain_discovery();

        assert_eq!(app.records.len(), 1);
    }

    #[test]
    fn upserting_an_occurrence_replaces_it_across_endpoint_and_txt_changes() {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );

        tx.send(DiscoveryEvent::Upsert(on_interface("alpha", "10.0.0.1", 1)))
            .unwrap();

        // The same occurrence re-resolved: new address, port, and TXT data.
        let mut moved = on_interface("alpha", "10.0.0.9", 1);
        moved.port = Some(2222);
        moved.txt.insert("path".to_string(), "/admin".to_string());
        tx.send(DiscoveryEvent::Upsert(moved)).unwrap();
        app.drain_discovery();

        // An adapter-named occurrence keeps its identity when its endpoint
        // moves, so this replaced the record instead of forking a duplicate.
        assert_eq!(app.records.len(), 1);
        let record = app.records.values().next().expect("record");
        assert_eq!(
            record.addresses,
            vec!["10.0.0.9".parse::<IpAddr>().unwrap()]
        );
        assert_eq!(record.port, Some(2222));
        assert_eq!(record.txt.get("path").map(String::as_str), Some("/admin"));
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

    // ── refresh & config reload ─────────────────────────────────────────────
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use color_eyre::eyre::eyre;

    /// A factory whose spawned sessions' senders are captured, so a test can
    /// feed events through whichever session a refresh started — and prove that
    /// events sent to a superseded one never arrive.
    fn channel_factory() -> (
        DiscoveryFactory,
        Arc<Mutex<Vec<mpsc::Sender<DiscoveryEvent>>>>,
    ) {
        let spawned: Arc<Mutex<Vec<mpsc::Sender<DiscoveryEvent>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let factory = {
            let spawned = spawned.clone();
            Box::new(move || {
                let (tx, rx) = mpsc::channel();
                spawned.lock().unwrap().push(tx);
                DiscoverySession::detached(rx)
            })
        };
        (factory, spawned)
    }

    #[test]
    fn refresh_restarts_discovery_and_repopulates_like_startup() {
        let (factory, spawned) = channel_factory();
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        )
        .with_discovery_factory(factory);
        assert_eq!(app.visible_groups.len(), 2);

        send(&mut app, KeyCode::Char('r'));

        // The list is empty and a new session runs.
        assert!(app.records.is_empty());
        assert!(app.visible_groups.is_empty());
        assert!(app.status.contains("refresh"));
        assert_eq!(spawned.lock().unwrap().len(), 1);

        // Events from the replacement session repopulate the list.
        spawned.lock().unwrap()[0]
            .send(DiscoveryEvent::Upsert(ssh("gamma", "10.0.0.3")))
            .unwrap();
        app.drain_discovery();
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].label(), "gamma");
    }

    #[test]
    fn refresh_resets_cursor_and_scroll_but_keeps_filters() {
        let (factory, _spawned) = channel_factory();
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        )
        .with_discovery_factory(factory);
        send(&mut app, KeyCode::Down);
        app.details_scroll = 3;
        app.filter.text_query = "bet".to_string();

        app.refresh_services();

        assert_eq!(app.selected, 0);
        assert_eq!(app.details_scroll, 0);
        assert_eq!(
            app.filter.text_query, "bet",
            "refresh restarts discovery; it does not discard what the user asked to see"
        );
    }

    #[test]
    fn refresh_without_discovery_control_reports_status_and_keeps_records() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);

        send(&mut app, KeyCode::Char('r'));

        assert!(app.status.contains("refresh is not available"));
        assert_eq!(app.records.len(), 1, "records must not be lost");
    }

    // ── discovery session lifecycle ─────────────────────────────────────────

    /// An app whose session is driven by the returned sender. Dropping the
    /// sender is a producer going away, exactly as a real adapter's does.
    fn app_with_session(
        matcher: Matcher,
        records: Vec<Entry>,
    ) -> (App, mpsc::Sender<DiscoveryEvent>) {
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            matcher,
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        );
        for record in records {
            app.records.insert(record.id(), record);
        }
        app.recompute_visible();
        (app, tx)
    }

    /// The core of this task: a producer going away must not read as "no new
    /// events". mDNS is edge-triggered, so a dead browse can never retract a
    /// service that has since gone; keeping the list would invite the user to
    /// launch a command at a host that may not be there.
    #[test]
    fn a_real_disconnect_clears_records_and_reports_a_persistent_failure() {
        let (mut app, tx) = app_with_session(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        assert_eq!(app.visible_groups.len(), 1);

        drop(tx);
        app.drain_discovery();

        assert!(app.records.is_empty(), "unverifiable records must not stay");
        assert!(app.visible_groups.is_empty());
        assert!(matches!(app.session.state(), SessionState::Failed(_)));
        assert!(
            !app.session.state().is_listening(),
            "the app must stop implying it is listening"
        );
        assert!(app.status.contains("discovery stopped"));
    }

    /// The same verdict with nothing discovered yet: the empty list must be
    /// explained as a failure rather than left looking like a quiet network.
    #[test]
    fn a_real_disconnect_with_no_records_still_reports_a_failure() {
        let (mut app, tx) = app_with_session(Matcher::default(), Vec::new());

        drop(tx);
        app.drain_discovery();

        assert!(matches!(app.session.state(), SessionState::Failed(_)));
        assert!(app.status.contains("discovery stopped"));
    }

    /// A startup error's cause must survive: it is carried by the failure, not
    /// left on a status line for the next event to erase.
    #[test]
    fn a_startup_failure_keeps_its_cause_text_across_later_drains() {
        let (mut app, tx) = app_with_session(Matcher::default(), Vec::new());
        tx.send(DiscoveryEvent::Status(
            "mDNS discovery unavailable (no such device); try --fake-discovery for sample records, or refresh to retry"
                .to_string(),
        ))
        .unwrap();
        drop(tx);

        app.drain_discovery();
        let reported = app.status.clone();

        // Ticking on does not decay the verdict back into silence.
        for _ in 0..5 {
            app.drain_discovery();
        }
        assert_eq!(app.status, reported, "the failure must be persistent");
        assert!(app.status.contains("discovery stopped"));
        assert!(matches!(app.session.state(), SessionState::Failed(_)));
    }

    /// A picker's entries were computed from records the failure just
    /// invalidated; leaving it open would let the user act on them.
    #[test]
    fn a_real_disconnect_closes_a_derived_picker() {
        let (mut app, tx) =
            app_with_session(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")]);
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::ActionPicker);

        drop(tx);
        app.drain_discovery();

        assert_eq!(app.mode, AppMode::Browse);
        assert!(app.records.is_empty());
    }

    /// Explicit fake discovery: running out of samples is the normal ending of
    /// a finite stream, so the samples stay and nothing is reported as broken.
    #[test]
    fn finite_fake_completion_keeps_its_samples_and_reports_completion() {
        let mut cli = test_cli();
        cli.fake_discovery = true;
        // One sample record keeps the stream short.
        cli.service_type = Some("_ssh._tcp".to_string());
        let session = crate::discovery::start(
            &cli.discovery_options()
                .expect("valid test discovery options"),
        );
        let mut app = App::new(cli, Matcher::default(), KeyBindings::default(), session);

        // Drain until the stream ends, as the event loop would.
        while app.session.state().is_listening() {
            app.drain_discovery();
            std::thread::yield_now();
        }

        assert_eq!(*app.session.state(), SessionState::Complete);
        assert_eq!(
            app.records.len(),
            1,
            "a finished sample stream keeps its records"
        );
        assert!(app.status.contains("complete"));
        assert!(
            !app.status.contains("failed") && !app.status.contains("stopped"),
            "finishing a finite stream is not a failure: {}",
            app.status
        );
    }

    /// Refresh is the recovery action: it must work *from* a failed session.
    #[test]
    fn refresh_recovers_from_a_failed_session() {
        let (factory, spawned) = channel_factory();
        let (tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        )
        .with_discovery_factory(factory);

        drop(tx);
        app.drain_discovery();
        assert!(matches!(app.session.state(), SessionState::Failed(_)));

        send(&mut app, KeyCode::Char('r'));

        assert!(
            app.session.state().is_listening(),
            "refresh must recover a failed session"
        );
        spawned.lock().unwrap()[0]
            .send(DiscoveryEvent::Upsert(ssh("gamma", "10.0.0.3")))
            .unwrap();
        app.drain_discovery();
        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(app.visible_groups[0].label(), "gamma");
    }

    /// The session owns its receiver, so a replaced session's events cannot
    /// reach the new list — old and new can never mix.
    #[test]
    fn events_from_a_replaced_session_never_reach_the_new_list() {
        let (factory, spawned) = channel_factory();
        let (old_tx, rx) = mpsc::channel();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::detached(rx),
        )
        .with_discovery_factory(factory);

        send(&mut app, KeyCode::Char('r'));

        // The superseded producer tries to keep feeding the list. It cannot:
        // the old session was dropped and took its receiver with it, so the
        // send has nowhere to land. The guarantee is structural, not a rule
        // the drain loop has to remember.
        assert!(
            old_tx
                .send(DiscoveryEvent::Upsert(ssh("stale", "10.0.0.9")))
                .is_err(),
            "a replaced session's receiver must be gone with it"
        );
        spawned.lock().unwrap()[0]
            .send(DiscoveryEvent::Upsert(ssh("fresh", "10.0.0.1")))
            .unwrap();
        app.drain_discovery();

        assert_eq!(app.visible_groups.len(), 1);
        assert_eq!(
            app.visible_groups[0].label(),
            "fresh",
            "only the current session's events may populate the list"
        );
    }

    /// A refresh whose new session then fails must not leave the pre-refresh
    /// records on screen labelled as current.
    #[test]
    fn a_refresh_that_fails_does_not_retain_the_old_records() {
        let (factory, spawned) = channel_factory();
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")])
            .with_discovery_factory(factory);
        assert_eq!(app.visible_groups.len(), 1);

        send(&mut app, KeyCode::Char('r'));
        // The replacement session's producer dies immediately.
        spawned.lock().unwrap().clear();
        app.drain_discovery();

        assert!(app.records.is_empty());
        assert!(app.visible_groups.is_empty());
        assert!(matches!(app.session.state(), SessionState::Failed(_)));
    }

    #[test]
    fn requested_reload_swaps_commands_and_recomputes_matches() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(Box::new(|_cli| {
                Ok((
                    Box::new(matcher_from(&[SSH])) as Box<dyn RuleEngine>,
                    Vec::new(),
                ))
            }));
        assert_eq!(app.matcher.command_count(), 0);
        assert_eq!(app.group_matches[0].len(), 0);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.matcher.command_count(), 1);
        assert_eq!(
            app.group_matches[0].len(),
            1,
            "matches are recomputed against the reloaded rules"
        );
        assert!(app.status.contains("reloaded 1 command"));
        assert!(
            !app.reload_requested.load(Ordering::Relaxed),
            "the request is consumed"
        );
    }

    #[test]
    fn poll_without_a_reload_request_does_nothing() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader({
                let calls = calls.clone();
                Box::new(move |_cli| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok((
                        Box::new(Matcher::default()) as Box<dyn RuleEngine>,
                        Vec::new(),
                    ))
                })
            });
        let status = app.status.clone();

        app.poll_reload();

        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(app.status, status);
    }

    #[test]
    fn failed_reload_keeps_the_current_commands() {
        let mut app = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(Box::new(|_cli| Err(eyre!("boom"))));

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.matcher.command_count(), 1, "old rules stay in force");
        assert!(app.status.contains("config reload failed"));
        assert!(app.status.contains("boom"));
    }

    #[test]
    fn reload_reports_skipped_config_files() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(Box::new(|_cli| {
                Ok((
                    Box::new(matcher_from(&[SSH])) as Box<dyn RuleEngine>,
                    vec!["bad.toml: not toml".to_string()],
                ))
            }));

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert!(app.status.contains("skipped 1 config file"));
    }

    #[test]
    fn reload_closes_a_picker_built_from_the_old_rules() {
        let mut app = app_with(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(Box::new(|_cli| {
                Ok((
                    Box::new(Matcher::default()) as Box<dyn RuleEngine>,
                    Vec::new(),
                ))
            }));
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::ActionPicker);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.mode, AppMode::Browse);
        assert!(app.action_matches.is_empty());
    }

    #[test]
    fn reload_without_a_loader_reports_status() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert!(app.status.contains("config reload is not available"));
    }

    #[test]
    fn quit_keys_request_quit() {
        let mut common = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        assert!(
            common
                .handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
                .unwrap()
                .is_none()
        );
        assert!(common.should_quit);

        let mut browse = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        send(&mut browse, KeyCode::Char('q'));
        assert!(browse.should_quit);
    }

    #[test]
    fn ctrl_c_quits_immediately_from_a_modal() {
        // Regression: the quit request used to be a `status == "quit"` sentinel
        // honored only in Browse mode, so Ctrl-C inside a modal did nothing
        // visible but poisoned the status, quitting on the next unrelated key.
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        send(&mut app, KeyCode::Char('?'));
        assert_eq!(app.mode, AppMode::Help);

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();

        assert!(app.should_quit, "ctrl-c must quit while a modal is open");
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
requirements = ["kinjo-absent-tool-xyz"]
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "echo hi"
mode = "execute"
"#;

    const FORK_MISSING_BINARY: &str = r#"
[metadata]
name = "ghost"
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "kinjo-absent-binary-xyz --flag"
mode = "fork"
"#;

    const OPTIONAL_REQ: &str = r#"
[metadata]
name = "with-optional"
requirements = ["kinjo-absent-tool-xyz, optional"]
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
        assert!(app.status.contains("kinjo-absent-tool-xyz"));
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

    // A template naming an unknown field can no longer reach the app at all:
    // loading rejects it, so there is nothing here to fail at invocation time.
    // `plumber::template` covers that rejection.

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
                .contains("command `kinjo-absent-binary-xyz` not found")
        );
        assert_eq!(app.mode, AppMode::Browse);
    }

    #[test]
    fn failed_action_closes_an_open_picker() {
        // Two actions match, opening the picker; selecting the broken one must
        // both report the error and drop the picker, not leave it dangling.
        let mut app = app_with(
            matcher_from(&[SSH, FORK_MISSING_BINARY]),
            vec![ssh("alpha", "10.0.0.1")],
        );

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::ActionPicker);
        assert_eq!(app.action_matches.len(), 2);

        // action_index 1 is the missing-binary command (insertion order).
        send(&mut app, KeyCode::Down);
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode, AppMode::Browse);
        assert!(app.action_matches.is_empty());
        assert!(app.status.contains("cannot run `ghost`"));
    }
}
