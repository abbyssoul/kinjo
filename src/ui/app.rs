use std::{
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
        BrowseMode, DiscoveryEvent, DiscoverySession, Entry, EntryGroup, EntryGroupId, EntryId,
        GroupingMode, MAX_DISCOVERED_OCCURRENCES, RowHost, SessionPoll, SessionState,
        browse_projection,
    },
    plumber::{ActionOutcome, CommandConfig, MatchResult, PreparedCommand, RuleEngine},
};

use super::{
    cli::Cli,
    filter::FilterState,
    keymap::{Action, KeyBindings, Mode as KeyMode},
    layout::{Content, LayoutSnapshot},
    render,
    viewport::Window,
};

/// Preserve time for input and drawing even when discovery remains ready.
const MAX_DISCOVERY_EVENTS_PER_TICK: usize = 256;
/// Apply a queued input burst to one state snapshot without allowing a
/// continuously-ready terminal to postpone drawing forever.
const MAX_INPUT_EVENTS_PER_TICK: usize = 64;

/// What a reload attempt came back with. There is no third case: a reload
/// either produces a rule set complete enough to replace the running one, or it
/// produces the reasons it does not, and the running one stays.
pub enum ReloadOutcome {
    /// The whole configured overlay compiled. Safe to install.
    Loaded(Box<dyn RuleEngine>),
    /// At least one file or directory was invalid, so no rule set was built.
    /// Each diagnostic names its source and what was wrong with it.
    Rejected(Vec<String>),
}

/// Loads a fresh rule set for a config reload (SIGHUP). Injected by the
/// composition root so the app stays decoupled from config-file I/O.
///
/// The loader validates; the app installs or does not. Deciding *outside* the
/// app whether a candidate rule set is complete is what makes the swap
/// transactional: by the time the app sees a [`ReloadOutcome::Loaded`], nothing
/// is left to go wrong half way through.
pub type ConfigLoader = Box<dyn Fn(&Cli) -> ReloadOutcome>;

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

/// One row of a browse projection: a group of discovered entries, and the rules
/// that match it.
///
/// The two travel together because they are decided together — the matches are
/// what the engine said about *this* group, from the same records, in the same
/// recompute — and neither is much use alone. They used to be two vectors kept
/// parallel by index, which is a correspondence a comment can assert and the
/// compiler cannot: rendering hedged with `group_matches.get(i)` against a
/// desync it could not rule out, and a test could set one without the other and
/// still compile. Making the row the unit removes the question.
#[derive(Debug, Clone)]
pub(crate) struct BrowseRow {
    pub(crate) group: EntryGroup,
    /// The rules matching `group`, computed once per recompute so rendering and
    /// invocation share one result instead of re-running the engine (regexes
    /// included) every frame.
    pub(crate) matches: Vec<MatchResult>,
}

/// What an open picker was opened from, as an identity rather than as the data
/// it listed.
///
/// Discovery keeps arriving while a picker is up. A picker that owned a copy of
/// its matches went on offering services that had already been retracted, and
/// confirming it ran a command against a hostname or address that no longer
/// existed. Remembering *what was chosen* lets the matches be rebuilt from
/// current records whenever they change, so a picker can only ever list — and
/// only ever run — what discovery still stands behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAnchor {
    /// A row of the active browse projection.
    Row(EntryGroupId),
    /// A logical service listed under a command row. The command view projects
    /// rules rather than entries, so its rows are not browse rows and cannot be
    /// found among them.
    Service {
        command: String,
        service: EntryGroupId,
    },
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

/// The open interaction and exactly the data meaningful to it. Picker anchors,
/// matches, chosen actions, and indices cannot now exist independently or in a
/// picker mode that does not use them.
#[derive(Debug, Clone)]
enum ModalState {
    Browse,
    Search,
    TypeFilter,
    ActionPicker {
        anchor: PickerAnchor,
        matches: Vec<MatchResult>,
        index: usize,
    },
    InstancePicker {
        anchor: PickerAnchor,
        action: MatchResult,
        index: usize,
    },
    ServicePicker {
        index: usize,
    },
    Help,
}

impl ModalState {
    fn mode(&self) -> AppMode {
        match self {
            Self::Browse => AppMode::Browse,
            Self::Search => AppMode::Search,
            Self::TypeFilter => AppMode::TypeFilter,
            Self::ActionPicker { .. } => AppMode::ActionPicker,
            Self::InstancePicker { .. } => AppMode::InstancePicker,
            Self::ServicePicker { .. } => AppMode::ServicePicker,
            Self::Help => AppMode::Help,
        }
    }
}

/// The running application: the event loop, the state it decides from, and the
/// terminal it draws to.
///
/// # Interface
///
/// Everything below is implementation. A caller builds an `App`, optionally
/// attaches the two capabilities it cannot supply itself, runs it, and collects
/// what outlived the terminal:
///
/// ```text
/// App::new(cli, engine, keybindings, session)
///     .with_discovery_factory(..)   // else refresh is unavailable
///     .with_config_loader(..)       // else reload is unavailable
/// app.reload_trigger()              // wire to a signal, if you have one
/// app.note_skipped_configs(n)       // startup warnings, if any
/// app.run(terminal)                 // -> a command to exec, if the user chose one
/// app.take_reload_diagnostics()     // why the last reload was refused
/// ```
///
/// That list is the whole supported surface, and `kinjo::run` uses no more than
/// it. It is deliberately small enough for the [`RuleEngine`] extension path in
/// `docs/adr/0001-rule-engine-is-a-supported-extension-point.md` to be a real
/// one: substituting an engine means writing this sequence, not reaching into
/// the app's state.
///
/// The fields are `pub(crate)` rather than private only because [`super::render`]
/// projects them into a frame. They are not a caller's business, and a change to
/// any of them is not a change to this crate's public API.
pub struct App {
    pub(crate) cli: Cli,
    matcher: Box<dyn RuleEngine>,
    pub(crate) keybindings: KeyBindings,
    /// The running discovery session: its events, its state, and its shutdown.
    /// One value, so the receiver and the adapter behind it cannot drift apart;
    /// dropping or replacing it stops the producer.
    pub(crate) session: DiscoverySession,
    pub(crate) records: BTreeMap<EntryId, Entry>,
    /// Whether new occurrences are being rejected at the configured resource
    /// ceiling. Kept separately so a later transient status event cannot hide
    /// that the visible list is capped.
    record_limit_reached: bool,
    pub(crate) filter: FilterState,
    /// The rows of the active browse projection, each carrying its own matches.
    /// Empty in the command grouping mode, which projects rules instead and
    /// builds [`Self::command_groups`].
    pub(crate) rows: Vec<BrowseRow>,
    pub(crate) selected: usize,
    modal: ModalState,
    pub(crate) type_filter_index: usize,
    pub(crate) status: String,
    /// Set by the quit keybindings; the event loop exits when it is true.
    should_quit: bool,
    /// Set from the SIGHUP handler; the event loop polls it and reloads the
    /// command configs when it flips to true. Handed out by
    /// [`App::reload_trigger`].
    reload_requested: Arc<AtomicBool>,
    /// Reloads command configs on request; reload is unavailable when unset.
    config_loader: Option<ConfigLoader>,
    /// Why the most recent reload was rejected, in full: one entry per invalid
    /// source, naming it and what was wrong with it.
    ///
    /// The status line is transient — the next tick's message replaces it — but
    /// a rejected reload is something the user has to act on, so the detail is
    /// kept here until it is either superseded or printed on exit. The policy is
    /// latest-only: a reload reports on the configuration as it is *now*, so a
    /// successful reload clears this, and a later failure replaces it rather
    /// than piling up a history of edits already fixed.
    reload_diagnostics: Vec<String>,
    /// Starts a replacement discovery session; refresh is unavailable when unset.
    discovery_factory: Option<DiscoveryFactory>,
    /// How many rows each top-panel tab lists, in [`GroupingMode::TABS`] order.
    /// Recomputed with the visible rows from the same filtered records, so a
    /// tab's count always matches the list it would show.
    pub(crate) tab_counts: [usize; GroupingMode::TABS.len()],
    pub(crate) ticks: u64,
    /// Rows of the "group by command" view; populated only in that grouping mode.
    pub(crate) command_groups: Vec<CommandGroup>,
    /// Top line shown in the help overlay (0 = unscrolled). Help is generated
    /// from the active bindings, so it can be taller than the popup; this is how
    /// far down it the reader has moved. Clamped against the content when it
    /// changes, and again by the renderer's window, so a resize cannot strand it
    /// past the end.
    pub(crate) help_scroll: usize,
    /// Top line shown in the details pane (0 = unscrolled).
    pub(crate) details_scroll: usize,
    /// Where this frame's panels are and what bounds they impose.
    ///
    /// Computed by [`App::update_layout`] from the terminal area and the
    /// current content, before both drawing and input, so the frame the user
    /// clicked on and the geometry the click is resolved against are the same
    /// one. Rendering reads it and writes nothing back.
    pub(crate) layout: LayoutSnapshot,
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
            record_limit_reached: false,
            filter: FilterState::default(),
            rows: Vec::new(),
            selected: 0,
            modal: ModalState::Browse,
            type_filter_index: 0,
            status,
            should_quit: false,
            reload_requested: Arc::new(AtomicBool::new(false)),
            config_loader: None,
            reload_diagnostics: Vec::new(),
            discovery_factory: None,
            tab_counts: [0; GroupingMode::TABS.len()],
            ticks: 0,
            command_groups: Vec::new(),
            help_scroll: 0,
            details_scroll: 0,
            // Nothing has been drawn yet, and a layout claiming otherwise would
            // let a click land on a row that does not exist. The event loop
            // replaces this before the first frame.
            layout: LayoutSnapshot::default(),
        }
    }

    pub(crate) fn mode(&self) -> AppMode {
        self.modal.mode()
    }

    /// Enter a non-picker mode. Picker modes require their data and therefore
    /// have dedicated constructors below rather than a discriminant-only set.
    #[cfg(test)]
    pub(crate) fn set_mode(&mut self, mode: AppMode) {
        self.modal = match mode {
            AppMode::Browse => ModalState::Browse,
            AppMode::Search => ModalState::Search,
            AppMode::TypeFilter => ModalState::TypeFilter,
            AppMode::Help => ModalState::Help,
            AppMode::ActionPicker | AppMode::InstancePicker | AppMode::ServicePicker => {
                panic!("picker modes require construction-atomic state")
            }
        };
    }

    pub(crate) fn action_picker(&self) -> Option<(&[MatchResult], usize)> {
        match &self.modal {
            ModalState::ActionPicker { matches, index, .. } => Some((matches, *index)),
            _ => None,
        }
    }

    pub(crate) fn instance_picker(&self) -> Option<(&MatchResult, usize)> {
        match &self.modal {
            ModalState::InstancePicker { action, index, .. } => Some((action, *index)),
            _ => None,
        }
    }

    pub(crate) fn service_picker_index(&self) -> Option<usize> {
        match &self.modal {
            ModalState::ServicePicker { index } => Some(*index),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_action_picker_for_test(&mut self, matches: Vec<MatchResult>, index: usize) {
        self.modal = ModalState::ActionPicker {
            anchor: PickerAnchor::Row(EntryGroupId::ServiceType("test".to_string())),
            matches,
            index,
        };
    }

    #[cfg(test)]
    pub(crate) fn set_instance_picker_for_test(&mut self, action: MatchResult, index: usize) {
        self.modal = ModalState::InstancePicker {
            anchor: PickerAnchor::Row(EntryGroupId::ServiceType("test".to_string())),
            action,
            index,
        };
    }

    #[cfg(test)]
    pub(crate) fn set_service_picker_for_test(&mut self, index: usize) {
        self.modal = ModalState::ServicePicker { index };
    }

    #[cfg(test)]
    pub(crate) fn set_picker_index_for_test(&mut self, new_index: usize) {
        match &mut self.modal {
            ModalState::ActionPicker { index, .. }
            | ModalState::InstancePicker { index, .. }
            | ModalState::ServicePicker { index } => *index = new_index,
            _ => panic!("no picker is open"),
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

    /// The flag whose flip asks the running app to reload its command configs.
    ///
    /// Handed out rather than installed: a signal handler is the caller's to
    /// own — it is process-global, platform-specific, and on a non-unix build
    /// there may be nothing to install. The app's side of the bargain is only
    /// that it polls this between frames and reloads when it finds it set.
    /// Setting it without a [`ConfigLoader`] attached is not an error; the app
    /// reports that reload is unavailable.
    pub fn reload_trigger(&self) -> Arc<AtomicBool> {
        self.reload_requested.clone()
    }

    /// Take `records` as though discovery had produced them, and settle into
    /// the state that follows.
    ///
    /// Test-only, and shared with [`super::render`]'s tests because they need an
    /// app that is *showing* something. It goes through the same recompute the
    /// event loop uses, so a test arranged this way sees the projection the
    /// running app would build — filtering, grouping, matching and selection
    /// included — rather than one the test assembled itself and could get wrong.
    #[cfg(test)]
    pub(crate) fn showing(&mut self, records: &[Entry]) {
        for record in records {
            self.records.insert(record.id(), record.clone());
        }
        self.recompute_visible();
    }

    /// Report that `count` command config files were skipped as invalid while
    /// loading, so the user is told at startup rather than left wondering why a
    /// rule they wrote does nothing.
    ///
    /// Takes the count, not a message: the status line is this module's to word.
    /// A count of zero says nothing, so the caller need not ask whether it has
    /// anything to report.
    pub fn note_skipped_configs(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        self.status = format!("skipped {count} command config file(s); details printed on exit");
    }

    /// Take the diagnostics of the most recent rejected reload, leaving none.
    ///
    /// Taking rather than borrowing is the point: these outlive the terminal.
    /// The status line that announced the rejection dies with the TUI, so the
    /// caller is expected to print them afterwards, and once taken they are the
    /// caller's only copy. The policy is latest-only — a successful reload
    /// clears them — so what comes back describes the configuration as it is
    /// now, not a history of edits already fixed.
    pub fn take_reload_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reload_diagnostics)
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<Option<PreparedCommand>> {
        let _mouse_capture = MouseCaptureGuard::enable();
        self.recompute_visible();
        let mut dirty = true;
        let mut last_animation = std::time::Instant::now();

        loop {
            dirty |= self.poll_reload();
            dirty |= self.drain_discovery();

            if let Some((period, tick_step)) = self.animation_period()
                && last_animation.elapsed() >= period
            {
                self.ticks = self.ticks.wrapping_add(tick_step);
                last_animation = std::time::Instant::now();
                dirty = true;
            }

            if dirty {
                // Build details once. Its height and the rows drawn below are
                // two reads of this same per-frame projection.
                let details = render::DetailsContent::for_app(self);
                terminal.autoresize()?;
                self.update_layout_with_details_height(
                    terminal.get_frame().area(),
                    details.height(),
                );
                terminal.draw(|frame| render::render_with_details(frame, self, details))?;
                dirty = false;
            }

            if event::poll(Duration::from_millis(120))? {
                for _ in 0..MAX_INPUT_EVENTS_PER_TICK {
                    match event::read()? {
                        Event::Key(key) => {
                            if key.kind == KeyEventKind::Release {
                                // Release events do not change the frame.
                            } else {
                                // Input is bounded by the geometry of the frame the
                                // user saw. A burst updates that state repeatedly,
                                // then pays for one replacement frame.
                                if let Some(command) = self.handle_key(key, self.layout.area())? {
                                    return Ok(Some(command));
                                }
                                dirty = true;
                                if self.should_quit {
                                    return Ok(None);
                                }
                            }
                        }
                        Event::Mouse(mouse) => {
                            self.handle_mouse(mouse);
                            dirty = true;
                        }
                        Event::Resize(_, _) => dirty = true,
                        _ => {}
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
    }

    /// Work out where this frame's panels are, and bring the details scroll
    /// back inside them.
    ///
    /// Called once per tick before drawing and before any event is handled, so
    /// the geometry render draws with, the bounds a scroll key is clamped to,
    /// and the rectangles a click is tested against are all the same snapshot.
    /// A resize or a shorter selection therefore cannot strand the reader below
    /// the end of the content: the next snapshot pulls the scroll back up.
    #[cfg(test)]
    pub(crate) fn update_layout(&mut self, area: Rect) {
        let details_height = render::details_content_height(self);
        self.update_layout_with_details_height(area, details_height);
    }

    fn update_layout_with_details_height(&mut self, area: Rect, details_height: usize) {
        self.layout = LayoutSnapshot::compute(
            area,
            Content {
                list_total: self.active_count(),
                details_total: details_height,
            },
        );
        self.details_scroll = self.details_scroll.min(self.layout.details_max_scroll());
    }

    /// The next genuinely changing frame when no state/input event arrives.
    /// Spinner and caret phases align on 240/480ms steps; occurrence ages need
    /// only their displayed second to advance.
    fn animation_period(&self) -> Option<(Duration, u64)> {
        if self.session.state().is_listening() {
            Some((Duration::from_millis(240), 2))
        } else if self.mode() == AppMode::Search {
            Some((Duration::from_millis(480), 4))
        } else if self.visible_details_show_occurrence_ages() {
            Some((Duration::from_secs(1), 8))
        } else {
            None
        }
    }

    fn visible_details_show_occurrence_ages(&self) -> bool {
        self.filter.grouping == GroupingMode::LogicalService
            && self.rows.get(self.selected).is_some()
            && self.layout.details_viewport() > 0
    }

    /// Take everything the session has produced since the last tick, and notice
    /// if it has ended. Crate-visible for the same reason as
    /// [`App::update_layout`]: a test that wants a frame the event loop could
    /// actually have drawn has to drive the loop's steps, not simulate them.
    pub(crate) fn drain_discovery(&mut self) -> bool {
        let mut changed = false;
        let mut visible_changed = false;
        for _ in 0..MAX_DISCOVERY_EVENTS_PER_TICK {
            let event = match self.session.poll() {
                SessionPoll::Event(event) => event,
                SessionPoll::Idle => break,
                // The producer is gone. Reported once, so this reacts to the
                // ending rather than re-applying it on every tick.
                SessionPoll::Ended(state) => {
                    changed |= self.apply_session_end(&state);
                    visible_changed = true;
                    break;
                }
            };
            visible_changed = true;
            match event {
                DiscoveryEvent::Upsert(record) => {
                    let id = record.id();
                    let pending = EntryId::pending(record.registration());
                    let replaces_pending = !id.is_pending() && self.records.contains_key(&pending);
                    let grows = !self.records.contains_key(&id) && !replaces_pending;
                    if grows && self.records.len() >= MAX_DISCOVERED_OCCURRENCES {
                        self.record_limit_reached = true;
                        self.status = format!(
                            "discovery capped at {MAX_DISCOVERED_OCCURRENCES} occurrences; new occurrences are being ignored"
                        );
                        continue;
                    }
                    // A real occurrence supersedes the registration's
                    // unresolved placeholder, if one is still listed.
                    if !id.is_pending() {
                        self.records.remove(&pending);
                    }
                    self.records.insert(id, record);
                    changed = true;
                }
                DiscoveryEvent::Remove(id) => {
                    // Exactly the named occurrence: siblings of the same
                    // registration on other interfaces stay live.
                    changed |= self.records.remove(&id).is_some();
                    self.reopen_record_capacity();
                }
                DiscoveryEvent::RemoveRegistration(registration) => {
                    let before = self.records.len();
                    self.records
                        .retain(|id, _| *id.registration() != registration);
                    changed |= self.records.len() != before;
                    self.reopen_record_capacity();
                }
                DiscoveryEvent::Status(status) => {
                    if !self.record_limit_reached {
                        self.status = status;
                    }
                }
            }
        }
        if changed {
            self.recompute_visible();
        }
        visible_changed
    }

    fn reopen_record_capacity(&mut self) {
        if self.record_limit_reached && self.records.len() < MAX_DISCOVERED_OCCURRENCES {
            self.record_limit_reached = false;
            self.status = "discovery capacity is available again".to_string();
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
                self.record_limit_reached = false;
                // Set last: the cause must be what the user is left looking at.
                self.status = failure.message();
                had_records
            }
        }
    }

    fn recompute_visible(&mut self) {
        let records = self.records.values().collect::<Vec<_>>();
        self.filter.observe_types(records.iter().copied());
        let filtered = self.filter.apply(records.iter().copied());

        // Command rows contain logical services, so that projection is useful
        // even when no browse tab is active. The projection also counts every
        // browse tab during the same walk over `filtered`.
        let active_browse = self
            .filter
            .grouping
            .browse_mode()
            .unwrap_or(BrowseMode::LogicalService);
        let projection = browse_projection(&filtered, active_browse);
        self.tab_counts = [
            projection.counts[0],
            projection.counts[1],
            projection.counts[2],
            self.matcher.command_count(),
        ];

        // The command tab groups configured rules rather than projecting the
        // discovered entries, so it has its own row builder.
        if self.filter.grouping.browse_mode().is_none() {
            self.recompute_command_groups(projection.groups);
            return;
        }

        self.command_groups = Vec::new();
        let previous = self
            .rows
            .get(self.selected)
            .map(|row| row.group.id().clone());
        // Each row is matched as it is built, so a row cannot exist without the
        // matches that belong to it.
        self.rows = projection
            .groups
            .into_iter()
            .map(|group| BrowseRow {
                matches: self.matcher.matches_group(&group),
                group,
            })
            .collect();
        // Structured row identity, not the list position: the cursor stays on
        // the row the user chose even when rows appear, vanish, or re-sort.
        match find_selection(&self.rows, previous.clone(), |row| row.group.id().clone()) {
            Some(index) => self.selected = index,
            None => self.clamp_selection(),
        }
        self.refocus_details(previous, |app| {
            app.rows.get(app.selected).map(|row| row.group.id().clone())
        });
        self.reconcile_action_pickers();
    }

    /// Settle the details scroll after the rows underneath the cursor have been
    /// rebuilt, given what was focused `before`.
    ///
    /// Scroll position is a place inside one row's details, so it only means
    /// anything for as long as that row is the one in focus. When the identity
    /// under the cursor changes — the row was removed, filtered away, or the
    /// clamp moved the cursor onto a neighbour — the reader is looking at a
    /// different service, and starting them part-way down its details is at best
    /// arbitrary. The same identity keeps its scroll; the next
    /// [`App::update_layout`] clamps it to whatever the new content is tall
    /// enough for.
    fn refocus_details<K: PartialEq>(
        &mut self,
        before: Option<K>,
        focused: impl Fn(&Self) -> Option<K>,
    ) {
        if focused(self) != before {
            self.details_scroll = 0;
        }
    }

    /// The rules matching an anchor, freshly matched against current records.
    ///
    /// Empty when the anchor no longer resolves — its row was filtered away, its
    /// service was retracted, or its rule stopped matching — which is exactly the
    /// signal reconciliation needs.
    fn resolve_anchor(&self, anchor: &PickerAnchor) -> Vec<MatchResult> {
        match anchor {
            PickerAnchor::Row(id) => self
                .rows
                .iter()
                .find(|row| row.group.id() == id)
                .map(|row| row.matches.clone())
                .unwrap_or_default(),
            PickerAnchor::Service { command, service } => self
                .command_groups
                .iter()
                .find(|group| group.command.name == *command)
                .and_then(|group| group.services.iter().find(|s| s.id() == service))
                .map(|group| {
                    self.matcher
                        .matches_group(group)
                        .into_iter()
                        .filter(|result| result.command.name == *command)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// What an anchor is called, for a status line the user can act on.
    fn anchor_label(&self, anchor: &PickerAnchor) -> String {
        match anchor {
            PickerAnchor::Row(id) => self
                .rows
                .iter()
                .find(|row| row.group.id() == id)
                .map(|row| row.group.label().to_string())
                .unwrap_or_else(|| "the selected service".to_string()),
            PickerAnchor::Service { service, .. } => self
                .command_groups
                .iter()
                .flat_map(|group| &group.services)
                .find(|s| s.id() == service)
                .map(|group| group.label().to_string())
                .unwrap_or_else(|| "the selected service".to_string()),
        }
    }

    /// Rebuild an open action/instance picker from current records, keeping its
    /// cursor on the identity the user chose.
    ///
    /// Called after every recompute, which is the only thing that can change the
    /// records underneath. The picker therefore always lists what discovery
    /// currently says, and the key handler — which runs against the same state
    /// the frame was drawn from — cannot confirm anything else.
    fn reconcile_action_pickers(&mut self) {
        let previous = std::mem::replace(&mut self.modal, ModalState::Browse);
        match previous {
            ModalState::ActionPicker {
                anchor,
                matches: old_matches,
                index: old_index,
            } => {
                let chosen = old_matches
                    .get(old_index)
                    .map(|result| result.command.name.as_str());
                let matches = self.resolve_anchor(&anchor);
                if matches.is_empty() {
                    let label = self.anchor_label(&anchor);
                    self.abandon_picker(format!("`{label}` is no longer available"));
                    return;
                }
                let Some(index) = chosen.and_then(|name| {
                    matches
                        .iter()
                        .position(|result| result.command.name == name)
                }) else {
                    self.abandon_picker("the selected action no longer matches".to_string());
                    return;
                };
                self.modal = ModalState::ActionPicker {
                    anchor,
                    matches,
                    index,
                };
            }
            ModalState::InstancePicker {
                anchor,
                action,
                index,
            } => {
                let name = action.command.name.clone();
                // A target's identity is the command it would run — task 006's
                // dedup key. Address-expanded candidates share an occurrence
                // id, while the prepared argv keeps them distinct.
                let chosen_target = action
                    .targets
                    .get(index)
                    .map(|target| action.command.action.prepare(target).ok());
                let matches = self.resolve_anchor(&anchor);
                if matches.is_empty() {
                    let label = self.anchor_label(&anchor);
                    self.abandon_picker(format!("`{label}` is no longer available"));
                    return;
                }
                let Some(live) = matches
                    .into_iter()
                    .find(|result| result.command.name == name)
                else {
                    self.abandon_picker(format!("`{name}` no longer matches"));
                    return;
                };
                let position = chosen_target.flatten().and_then(|chosen| {
                    live.targets.iter().position(|target| {
                        live.command.action.prepare(target).ok() == Some(chosen.clone())
                    })
                });
                match position {
                    Some(index) => {
                        self.modal = ModalState::InstancePicker {
                            anchor,
                            action: live,
                            index,
                        };
                    }
                    None => self.abandon_picker("the selected target is gone".to_string()),
                }
            }
            other => self.modal = other,
        }
    }

    /// Build the command-grouped rows: each configured command paired with the
    /// distinct logical services that match at least one of its instances.
    fn recompute_command_groups(&mut self, service_groups: Vec<EntryGroup>) {
        let previous = self
            .command_groups
            .get(self.selected)
            .map(|group| group.command.name.clone());
        // Read before the rows go: an index into the old service list means
        // nothing once it has been rebuilt.
        let chosen_service = self
            .command_groups
            .get(self.selected)
            .and_then(|group| {
                self.service_picker_index()
                    .and_then(|index| group.services.get(index))
            })
            .map(|service| service.id().clone());

        let mut command_groups: Vec<CommandGroup> = self
            .matcher
            .commands()
            .into_iter()
            .map(|command| CommandGroup {
                command,
                services: Vec::new(),
            })
            .collect();
        let index: HashMap<String, usize> = command_groups
            .iter()
            .enumerate()
            .map(|(i, group)| (group.command.name.clone(), i))
            .collect();
        for service_group in &service_groups {
            for command_name in self.matcher.matching_command_names(service_group) {
                if let Some(&i) = index.get(&command_name) {
                    command_groups[i].services.push(service_group.clone());
                }
            }
        }

        self.command_groups = command_groups;
        self.rows = Vec::new();
        match find_selection(&self.command_groups, previous.clone(), |group| {
            group.command.name.clone()
        }) {
            Some(index) => self.selected = index,
            None => self.clamp_selection(),
        }
        self.refocus_details(previous, |app| {
            app.command_groups
                .get(app.selected)
                .map(|group| group.command.name.clone())
        });
        self.reconcile_service_picker(chosen_service);
        self.reconcile_action_pickers();
    }

    /// Keep an open service picker on the service the user chose, after the
    /// command rows underneath it have been rebuilt.
    ///
    /// The cursor was an index into a list discovery can reorder or shorten, so
    /// without this a removed service hands its position — and the user's
    /// pending Enter — to whichever service slid into it.
    fn reconcile_service_picker(&mut self, chosen: Option<EntryGroupId>) {
        if self.mode() != AppMode::ServicePicker {
            return;
        }
        let Some(services) = self.command_groups.get(self.selected).map(|g| &g.services) else {
            self.abandon_picker("the selected command is gone".to_string());
            return;
        };
        match chosen.and_then(|id| services.iter().position(|service| *service.id() == id)) {
            Some(index) => self.modal = ModalState::ServicePicker { index },
            None => self.abandon_picker("the selected service is gone".to_string()),
        }
    }

    /// Number of rows in the currently active left-hand list.
    fn active_count(&self) -> usize {
        if self.filter.grouping == GroupingMode::Command {
            self.command_groups.len()
        } else {
            self.rows.len()
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
        self.modal = ModalState::Browse;
    }

    /// Close an open picker because what it was showing is gone, saying so.
    ///
    /// Reconciliation calls this rather than quietly moving the cursor onto a
    /// neighbour: the user chose a specific service, and silently retargeting
    /// their pending Enter to a different one is the failure this guards.
    fn abandon_picker(&mut self, reason: String) {
        self.return_to_browse();
        self.status = reason;
    }

    /// Resolve the key to at most one action for the active mode, then act on
    /// it. Resolution is the keymap's job, so no handler re-checks keys and no
    /// binding can be shadowed by the order the handlers are written in.
    ///
    /// `area` is the screen the modal windows are computed against, so that a
    /// key which moves a window is bounded by the same geometry that will draw
    /// it. It is passed in rather than written back by the renderer: the app
    /// state a key changes stays something only key handling changes.
    fn handle_key(&mut self, key: KeyEvent, area: Rect) -> Result<Option<PreparedCommand>> {
        let mode = self.mode();
        let action = self.keybindings.resolve(mode.key_mode(), key);
        if action == Some(Action::Quit) {
            self.should_quit = true;
            return Ok(None);
        }

        match mode {
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
                self.handle_help_key(action, area);
                Ok(None)
            }
        }
    }

    /// Help is read, not selected from, so its navigation moves the window
    /// rather than a cursor. `area` is the screen the next frame will be drawn
    /// on: the same geometry decides how far down the content a scroll can go.
    fn handle_help_key(&mut self, action: Option<Action>, area: Rect) {
        match action {
            Some(Action::HelpClose) => self.return_to_browse(),
            Some(Action::HelpDown) => self.scroll_help(1, area),
            Some(Action::HelpUp) => self.scroll_help(-1, area),
            _ => {}
        }
    }

    /// Scroll help by `delta` lines, clamped to what is left to read. Clamping
    /// here is what keeps the overlay's own keys honest: at the bottom, further
    /// presses do nothing rather than banking scroll the reader would have to
    /// undo before the content moved again.
    fn scroll_help(&mut self, delta: isize, area: Rect) {
        let total = render::help_lines(self).len();
        let max = Window::max_scroll(total, render::help_viewport(area)) as isize;
        self.help_scroll = (self.help_scroll as isize + delta).clamp(0, max) as usize;
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
            Some(Action::OpenSearch) => self.modal = ModalState::Search,
            Some(Action::OpenTypeFilter) => {
                self.type_filter_index = 0;
                self.modal = ModalState::TypeFilter;
            }
            Some(Action::TabNext) => self.cycle_tab(1),
            Some(Action::TabPrev) => self.cycle_tab(-1),
            Some(Action::SameHost) => self.toggle_same_host_filter(),
            Some(Action::Refresh) => self.refresh_services(),
            Some(Action::DetailsDown) => self.scroll_details(1),
            Some(Action::DetailsUp) => self.scroll_details(-1),
            // Help opens where it is read from: the top.
            Some(Action::OpenHelp) => {
                self.help_scroll = 0;
                self.modal = ModalState::Help;
            }
            // Typing a character with nothing bound to it starts a search with
            // it, so the query never loses the keystroke that opened it.
            None => {
                if let Some(ch) = typed_char(key) {
                    self.filter.text_query.push(ch);
                    self.modal = ModalState::Search;
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
        let count = self.filter.discovered_types().len();
        match action {
            Some(Action::TypeFilterClose) => self.return_to_browse(),
            Some(Action::TypeFilterDown) => {
                self.type_filter_index = move_index(self.type_filter_index, count, 1);
            }
            Some(Action::TypeFilterUp) => {
                self.type_filter_index = move_index(self.type_filter_index, count, -1);
            }
            Some(Action::TypeFilterToggle) => {
                if let Some(service_type) = self
                    .filter
                    .discovered_types()
                    .get(self.type_filter_index)
                    .cloned()
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
        let (len, current) = self
            .action_picker()
            .map(|(matches, index)| (matches.len(), index))
            .unwrap_or_default();
        match action {
            Some(Action::PickerClose) => self.return_to_browse(),
            Some(Action::PickerDown) => {
                if let ModalState::ActionPicker { index, .. } = &mut self.modal {
                    *index = move_index(current, len, 1);
                }
            }
            Some(Action::PickerUp) => {
                if let ModalState::ActionPicker { index, .. } = &mut self.modal {
                    *index = move_index(current, len, -1);
                }
            }
            Some(Action::PickerSelect) => {
                let chosen = match &self.modal {
                    ModalState::ActionPicker {
                        anchor,
                        matches,
                        index,
                    } => matches
                        .get(*index)
                        .cloned()
                        .map(|action| (action, anchor.clone())),
                    _ => None,
                };
                if let Some((chosen, anchor)) = chosen {
                    return self.choose_action(chosen, anchor);
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
        let (count, current) = self
            .instance_picker()
            .map(|(action, index)| (action.targets.len(), index))
            .unwrap_or_default();
        match action {
            Some(Action::PickerClose) => self.return_to_browse(),
            Some(Action::PickerDown) => {
                if let ModalState::InstancePicker { index, .. } = &mut self.modal {
                    *index = move_index(current, count, 1);
                }
            }
            Some(Action::PickerUp) => {
                if let ModalState::InstancePicker { index, .. } = &mut self.modal {
                    *index = move_index(current, count, -1);
                }
            }
            Some(Action::PickerSelect) => {
                let pending = match &self.modal {
                    ModalState::InstancePicker { action, index, .. } => {
                        Some((action.clone(), *index))
                    }
                    _ => None,
                };
                let Some((pending, index)) = pending else {
                    self.return_to_browse();
                    return Ok(None);
                };
                let Some(record) = pending.targets.get(index) else {
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
        let step = (self.layout.details_viewport() / 2).max(1) as isize;
        self.scroll_details_by(direction * step);
    }

    /// Scroll the details pane by `delta` lines, clamped to the bounds of the
    /// pane the user is looking at — the same snapshot it was drawn from, so a
    /// key cannot scroll past the end of a frame that is on screen.
    fn scroll_details_by(&mut self, delta: isize) {
        let max = self.layout.details_max_scroll() as isize;
        self.details_scroll = (self.details_scroll as isize + delta).clamp(0, max) as usize;
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Modal pickers and help stay keyboard-driven; the mouse only drives
        // the browse layer (which remains visible while searching).
        if !matches!(self.mode(), AppMode::Browse | AppMode::Search) {
            return;
        }
        let position = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.mouse_scroll(position, 1),
            MouseEventKind::ScrollUp => self.mouse_scroll(position, -1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = self.layout.list_row_at(position, self.selected) {
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
        if self.layout.is_over_details(position) {
            self.scroll_details_by(direction);
        } else if self.layout.list().contains(position) {
            self.move_selection(direction);
        }
    }

    fn invoke_selected(&mut self) -> Result<Option<PreparedCommand>> {
        if self.filter.grouping == GroupingMode::Command {
            return self.invoke_command();
        }
        let Some(row) = self.rows.get(self.selected) else {
            self.status = "no service selected".to_string();
            return Ok(None);
        };
        let matches = row.matches.clone();
        match matches.len() {
            0 => {
                self.status = format!("no configured actions match `{}`", row.group.label());
                Ok(None)
            }
            _ => {
                // Anchor to the row itself, not to its position: whatever the
                // picker goes on to show is rebuilt from this.
                let anchor = PickerAnchor::Row(row.group.id().clone());
                if matches.len() == 1 {
                    return self.choose_action(matches.into_iter().next().unwrap(), anchor);
                }
                self.modal = ModalState::ActionPicker {
                    anchor,
                    matches,
                    index: 0,
                };
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
                self.modal = ModalState::ServicePicker { index: 0 };
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
        // The command view lists rules, not browse rows, so an instance picker
        // opened from here anchors to the service the user picked.
        let anchor = PickerAnchor::Service {
            command: command.name.clone(),
            service: service.id().clone(),
        };
        self.choose_action(result, anchor)
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
                if let ModalState::ServicePicker { index } = &mut self.modal {
                    *index = move_index(*index, count, 1);
                }
            }
            Some(Action::PickerUp) => {
                if let ModalState::ServicePicker { index } = &mut self.modal {
                    *index = move_index(*index, count, -1);
                }
            }
            Some(Action::PickerSelect) => {
                let Some(group) = self.command_groups.get(self.selected) else {
                    self.return_to_browse();
                    return Ok(None);
                };
                let Some(service) = self
                    .service_picker_index()
                    .and_then(|index| group.services.get(index))
                    .cloned()
                else {
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

    fn choose_action(
        &mut self,
        action: MatchResult,
        anchor: PickerAnchor,
    ) -> Result<Option<PreparedCommand>> {
        // The rule decides whether there is a choice to make: it knows what its
        // candidates would actually run. Anything left to pick between here
        // genuinely differs, and one target means the alternatives were the
        // same command, not that the difference was deemed unimportant.
        if action.needs_selection() {
            self.modal = ModalState::InstancePicker {
                anchor,
                action,
                index: 0,
            };
            return Ok(None);
        }

        let Some(record) = action.targets.first() else {
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
    fn poll_reload(&mut self) -> bool {
        if self.reload_requested.swap(false, Ordering::Relaxed) {
            self.reload_config();
            true
        } else {
            false
        }
    }

    /// Reload command configs through the injected loader, atomically or not at
    /// all. A reload failure must not tear down the TUI — it is reported on the
    /// status line like action failures, and the rules already in force stay in
    /// force.
    ///
    /// The loader has already decided whether the configuration on disk is
    /// complete, so there is exactly one moment here where anything changes:
    /// either the whole candidate rule set replaces the whole active one, or
    /// nothing is touched. A reload the user is midway through writing leaves
    /// the session exactly as it was, still able to run every command it could a
    /// moment ago.
    fn reload_config(&mut self) {
        let Some(loader) = &self.config_loader else {
            self.status = "config reload is not available".to_string();
            return;
        };
        match loader(&self.cli) {
            ReloadOutcome::Loaded(matcher) => {
                self.matcher = matcher;
                // The reload spoke for the whole configuration, and it is valid:
                // whatever the last one complained about is either fixed or gone.
                self.reload_diagnostics.clear();
                self.close_pickers();
                self.recompute_visible();
                self.status = format!("reloaded {} command(s)", self.matcher.command_count());
            }
            ReloadOutcome::Rejected(diagnostics) => {
                // Say what is still true — the old rules are running — rather
                // than only what failed. The detail is kept for the exit report;
                // the status line cannot hold it and will not survive the tick.
                self.status = format!(
                    "config reload rejected: {} invalid config file(s); \
                     keeping the {} command(s) already loaded; details printed on exit",
                    diagnostics.len(),
                    self.matcher.command_count()
                );
                self.reload_diagnostics = diagnostics;
            }
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
        self.record_limit_reached = false;
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
            self.mode(),
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
        let Some(row) = self.rows.get(self.selected) else {
            self.status = "no row selected".to_string();
            return;
        };
        match row.group.facts().host() {
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
    let modified = !(key.modifiers - KeyModifiers::SHIFT).is_empty();
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
    use std::{net::IpAddr, num::NonZeroU32, sync::mpsc, time::Instant};

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
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.instances().len(), 1);
        assert_eq!(
            app.rows[0].group.facts().host(),
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
command = "ssh -- {hostname}"
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
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.instances().len(), 1);
        assert_eq!(app.rows[0].group.instances()[0].addresses.len(), 2);
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
command = "ssh -- {hostname}"
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

    /// Repeatable projection workload for task 102. Run alone in release mode:
    ///
    /// `cargo test --release benchmark_recompute_workload -- --ignored --nocapture`
    #[test]
    #[ignore = "manual performance workload"]
    fn benchmark_recompute_workload() {
        let mut builder = MatcherBuilder::new();
        for index in 0..24 {
            let service_type = ["_ssh._tcp", "_http._tcp", "_ipp._tcp", "_smb._tcp"][index % 4];
            builder
                .add_str(
                    &format!("rule-{index}"),
                    &format!(
                        r#"
[metadata]
name = "rule-{index}"

[match.service_type]
equals = "{service_type}"

[action]
command = "tool http://{{hostname}}/{{name}}"
mode = "execute"
"#
                    ),
                )
                .unwrap();
        }
        let records: Vec<Entry> = (0..2_000)
            .map(|index| {
                let service_type = ["_ssh._tcp", "_http._tcp", "_ipp._tcp", "_smb._tcp"][index % 4];
                let mut entry = Entry::new(format!("service-{index}"), service_type, "local");
                entry.hostname = Some(format!("host-{}.local", index % 500));
                entry.addresses = vec![format!("192.0.2.{}", index % 250 + 1).parse().unwrap()];
                entry.port = Some(1_000 + (index % 50) as u16);
                entry
                    .txt
                    .insert("path".to_string(), format!("/item/{index}"));
                entry
            })
            .collect();
        let mut app = app_with(builder.build(), records);

        let started = Instant::now();
        for iteration in 0..12 {
            app.filter.grouping = GroupingMode::TABS[iteration % GroupingMode::TABS.len()];
            app.filter.text_query = if iteration % 3 == 0 {
                "host 12".to_string()
            } else {
                String::new()
            };
            app.recompute_visible();
            std::hint::black_box(app.active_count());
        }
        let elapsed = started.elapsed();

        eprintln!(
            "recompute workload: 12 projections, 2000 entries, 24 rules: {:.3} ms total ({:.3} ms/projection)",
            elapsed.as_secs_f64() * 1_000.0,
            elapsed.as_secs_f64() * 1_000.0 / 12.0
        );
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
        app.showing(&records);
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

    /// The screen every mouse and detail-scroll test below is resolved against.
    /// Deliberately short: its body leaves a bordered list panel with five
    /// content rows at y = 3..=7, so a list of eight has to scroll and a click
    /// has a window to be wrong about.
    const MOUSE_SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 10,
    };

    /// An app with `count` distinct services, laid out for [`MOUSE_SCREEN`] the
    /// way the event loop lays one out before a frame: the list panel occupies
    /// x = 0..58 and the details pane x = 58..100, both at y = 2..=9.
    ///
    /// The layout is computed rather than asserted into place, so these tests
    /// hit-test against the same geometry a real frame would be drawn with.
    fn mouse_app(count: usize) -> App {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let records = names[..count]
            .iter()
            .map(|name| ssh(name, "10.0.0.1"))
            .collect();
        let mut app = app_with(Matcher::default(), records);
        app.update_layout(MOUSE_SCREEN);
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
        // The bound the wheel is clamped to is the one the layout worked out
        // from the pane and the selected row's details, not a number this test
        // asserted into place.
        let max = app.layout.details_max_scroll();
        assert!(max > 2, "the fixture's details must overflow its pane");

        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        assert_eq!(app.details_scroll, 2);

        // Clamped to the content bounds on both ends.
        for _ in 0..max + 5 {
            app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        }
        assert_eq!(app.details_scroll, max);
        for _ in 0..max + 5 {
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
        app.set_mode(AppMode::Help);

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

    /// A comfortable terminal, for the tests whose subject is not the geometry.
    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 40,
    };

    fn send(app: &mut App, code: KeyCode) -> Option<PreparedCommand> {
        app.handle_key(key(code), SCREEN).unwrap()
    }

    #[test]
    fn navigation_moves_and_clamps_selection_and_resets_scroll() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        assert_eq!(app.rows.len(), 2);
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

        assert_eq!(app.mode(), AppMode::Search);
        assert_eq!(app.filter.text_query, "z");
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.label(), "zulu");
    }

    #[test]
    fn search_backspace_clear_and_close() {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        send(&mut app, KeyCode::Char('z'));
        send(&mut app, KeyCode::Char('u'));
        assert_eq!(app.filter.text_query, "zu");

        send(&mut app, KeyCode::Backspace);
        assert_eq!(app.filter.text_query, "z");

        app.handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            SCREEN,
        )
        .unwrap();
        assert_eq!(app.filter.text_query, "", "ctrl-u clears the query");

        send(&mut app, KeyCode::Enter);
        assert_eq!(app.mode(), AppMode::Browse, "enter closes search");
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
        assert_eq!(app.mode(), AppMode::Search, "editing stays open");
    }

    /// Deleting re-filters the list; a stale row set would misreport the query.
    #[test]
    fn deleting_a_character_recomputes_the_visible_rows() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), ssh("zulu", "10.0.0.2")],
        );

        send(&mut app, KeyCode::Char('z'));
        assert_eq!(app.rows.len(), 1);

        send(&mut app, KeyCode::Delete);
        assert_eq!(app.filter.text_query, "");
        assert_eq!(app.rows.len(), 2, "both rows are visible again");
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
            assert_eq!(app.mode(), AppMode::Search);

            send(&mut app, close);

            assert_eq!(app.mode(), AppMode::Browse, "{close:?} leaves search");
            assert_eq!(
                app.filter.text_query, "z",
                "{close:?} must keep the active query"
            );
            assert_eq!(app.rows.len(), 1, "{close:?} keeps the list filtered");
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
        assert_eq!(app.rows.len(), 1);

        app.handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            SCREEN,
        )
        .unwrap();

        assert_eq!(app.filter.text_query, "");
        assert_eq!(app.mode(), AppMode::Search, "clearing stays in search");
        assert_eq!(app.rows.len(), 2);
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

        app.handle_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            SCREEN,
        )
        .unwrap();
        assert_eq!(app.filter.text_query, "z", "ctrl-u no longer clears");

        app.handle_key(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            SCREEN,
        )
        .unwrap();
        assert_eq!(app.filter.text_query, "");

        remove(&path);
    }

    // ── help scrolling ─────────────────────────────────────────────────────

    /// The 60×18 terminal the midpoint review reproduced the help clipping on.
    const SHORT_SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 60,
        height: 18,
    };

    fn help_app() -> App {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        app.set_mode(AppMode::Help);
        app
    }

    fn help_max_scroll(app: &App, area: Rect) -> usize {
        Window::max_scroll(render::help_lines(app).len(), render::help_viewport(area))
    }

    #[test]
    fn help_opens_at_the_top_and_scrolls_down_a_row_at_a_time() {
        let mut app = help_app();
        app.help_scroll = 7;
        app.set_mode(AppMode::Browse);

        send(&mut app, KeyCode::Char('?'));
        assert_eq!(app.mode(), AppMode::Help);
        assert_eq!(app.help_scroll, 0, "help must open where it is read from");

        app.handle_key(key(KeyCode::Down), SHORT_SCREEN).unwrap();
        assert_eq!(app.help_scroll, 1);
        app.handle_key(key(KeyCode::Char('j')), SHORT_SCREEN)
            .unwrap();
        assert_eq!(app.help_scroll, 2);
        app.handle_key(key(KeyCode::Up), SHORT_SCREEN).unwrap();
        assert_eq!(app.help_scroll, 1);
    }

    /// Scrolling must stop at the ends. Banking scroll past the bottom would
    /// leave the reader pressing up several times before anything moved.
    #[test]
    fn help_scrolling_stops_at_both_ends_of_the_content() {
        let mut app = help_app();
        let max = help_max_scroll(&app, SHORT_SCREEN);
        assert!(
            max > 0,
            "help must be clipped at 60x18 for this to mean much"
        );

        for _ in 0..max + 10 {
            app.handle_key(key(KeyCode::Down), SHORT_SCREEN).unwrap();
        }
        assert_eq!(app.help_scroll, max);

        for _ in 0..max + 10 {
            app.handle_key(key(KeyCode::Up), SHORT_SCREEN).unwrap();
        }
        assert_eq!(app.help_scroll, 0);
    }

    /// On a terminal tall enough to show every row there is nothing to scroll,
    /// so the scroll keys must not move a window that is already complete.
    #[test]
    fn help_does_not_scroll_when_all_of_it_fits() {
        let mut app = help_app();
        assert_eq!(help_max_scroll(&app, SCREEN), 0);

        app.handle_key(key(KeyCode::Down), SCREEN).unwrap();

        assert_eq!(app.help_scroll, 0);
    }

    /// Task 013's resolver stays the single source of truth: rebinding the
    /// scroll keys must move dispatch with the binding, and the defaults they
    /// replaced must stop working.
    #[test]
    fn rebound_help_scroll_keys_replace_the_defaults() {
        let path = temp_file(
            "help-scroll",
            r#"
[help]
down = ["ctrl-n"]
up = ["ctrl-p"]
"#,
        );
        let mut app = help_app();
        app.keybindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        app.handle_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            SHORT_SCREEN,
        )
        .unwrap();
        assert_eq!(app.help_scroll, 1);

        app.handle_key(
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            SHORT_SCREEN,
        )
        .unwrap();
        assert_eq!(app.help_scroll, 0);

        // The default it replaced no longer scrolls.
        app.handle_key(key(KeyCode::Down), SHORT_SCREEN).unwrap();
        assert_eq!(app.help_scroll, 0, "`down` is no longer bound to help.down");

        remove(&path);
    }

    /// Unbinding help scrolling leaves the overlay static rather than falling
    /// back to a hard-coded key beside the resolver.
    #[test]
    fn unbound_help_scroll_keys_do_nothing() {
        let path = temp_file(
            "help-scroll-off",
            r#"
[help]
down = []
up = []
"#,
        );
        let mut app = help_app();
        app.keybindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();

        app.handle_key(key(KeyCode::Down), SHORT_SCREEN).unwrap();
        app.handle_key(key(KeyCode::Char('j')), SHORT_SCREEN)
            .unwrap();

        assert_eq!(app.help_scroll, 0);

        remove(&path);
    }

    /// A window scrolled to the bottom of a short terminal must not strand the
    /// content when the terminal grows: the offset is clamped against whatever
    /// geometry the next key is handled on.
    #[test]
    fn growing_the_terminal_pulls_a_scrolled_help_window_back_onto_its_content() {
        let mut app = help_app();
        app.help_scroll = help_max_scroll(&app, SHORT_SCREEN);
        assert!(app.help_scroll > 0);

        // The same key, now on a screen that shows all of help.
        app.handle_key(key(KeyCode::Down), SCREEN).unwrap();

        assert_eq!(app.help_scroll, 0, "nothing left to scroll to");
    }

    /// An unbound control chord is a shortcut that did nothing, not text: it
    /// must not silently type its letter into the query.
    #[test]
    fn an_unbound_control_chord_does_not_type_into_the_search_query() {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        send(&mut app, KeyCode::Char('z'));

        app.handle_key(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
            SCREEN,
        )
        .unwrap();

        assert_eq!(app.filter.text_query, "z");
    }

    #[test]
    fn super_and_meta_chords_do_not_type_into_the_search_query() {
        let mut app = app_with(Matcher::default(), vec![ssh("zulu", "10.0.0.2")]);
        send(&mut app, KeyCode::Char('z'));

        for modifier in [KeyModifiers::SUPER, KeyModifiers::META] {
            app.handle_key(KeyEvent::new(KeyCode::Char('w'), modifier), SCREEN)
                .unwrap();
        }

        assert_eq!(app.filter.text_query, "z");
    }

    #[test]
    fn an_input_burst_moves_the_selection_before_the_next_frame() {
        let mut app = app_with(
            Matcher::default(),
            vec![
                ssh("alpha", "10.0.0.1"),
                ssh("beta", "10.0.0.2"),
                ssh("gamma", "10.0.0.3"),
                ssh("zulu", "10.0.0.4"),
            ],
        );

        for _ in 0..3 {
            app.handle_key(key(KeyCode::Down), SCREEN).unwrap();
        }

        assert_eq!(app.selected, 3);
    }

    /// Shift is folded into the character, so capitals must still type.
    #[test]
    fn shifted_characters_type_into_the_search_query() {
        let mut app = app_with(Matcher::default(), vec![ssh("Zulu", "10.0.0.2")]);

        app.handle_key(
            KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
            SCREEN,
        )
        .unwrap();

        assert_eq!(app.mode(), AppMode::Search);
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
        assert_eq!(app.mode(), AppMode::Search);
        assert_eq!(app.filter.text_query, "j");
        send(&mut app, KeyCode::Esc);

        send(&mut app, KeyCode::F(1));
        assert_eq!(app.mode(), AppMode::Help, "the rebound help key opens help");

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
        assert_eq!(app.mode(), AppMode::Search);

        remove(&path);
    }

    #[test]
    fn type_filter_toggle_hides_a_service_type() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), http("web")],
        );
        assert_eq!(app.rows.len(), 2);

        send(&mut app, KeyCode::Char('t'));
        assert_eq!(app.mode(), AppMode::TypeFilter);
        // Discovered types are sorted: _http._tcp is first.
        send(&mut app, KeyCode::Char(' '));

        assert!(
            app.rows
                .iter()
                .all(|row| row.group.facts().service_type()
                    == RowServiceType::Invariant("_ssh._tcp"))
        );
        assert_eq!(app.rows.len(), 1);
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
        assert_eq!(app.mode(), AppMode::Browse);

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

        assert_eq!(command.argv, vec!["ssh", "--", "alpha.local"]);
    }

    #[test]
    fn invoke_with_multiple_actions_opens_picker_then_runs_selection() {
        let mut app = app_with(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")]);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ActionPicker);
        assert_eq!(app.action_picker().unwrap().0.len(), 2);

        // action_index 0 is `ssh` (insertion order); selecting it runs that action.
        let command = send(&mut app, KeyCode::Enter).expect("picked action runs");
        assert_eq!(command.argv, vec!["ssh", "--", "alpha.local"]);
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
        assert_eq!(app.mode(), AppMode::Browse);
        assert!(app.status.contains("launched `ping`"));
    }

    /// A rule whose mandatory dependency is missing fails for the whole rule,
    /// not for one candidate. It must say so rather than quietly try the next
    /// service, which would run a command against something the user did not
    /// choose.
    #[test]
    fn a_missing_requirement_reports_failure_without_trying_another_target() {
        const NEEDS_ABSENT: &str = r#"
[metadata]
name = "needs-absent"
requirements = ["definitely-absent-xyz"]
[match.service_type]
equals = "_ssh._tcp"
[action]
command = "ssh -- {hostname}"
mode = "execute"
"#;
        let mut app = app_with(
            matcher_from(&[NEEDS_ABSENT]),
            vec![
                service_on("alpha", "_ssh._tcp", "alpha.local", 22),
                service_on("beta", "_ssh._tcp", "beta.local", 22),
            ],
        );
        app.filter.grouping = GroupingMode::ServiceType;
        app.recompute_visible();

        // The row's two hosts differ, so the action is offered for selection.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::InstancePicker);

        // Choosing one reports the rule's failure; nothing else runs.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert!(
            app.status.contains("definitely-absent-xyz"),
            "status should name the missing dependency, was: {}",
            app.status
        );
    }

    #[test]
    fn instance_picker_disambiguates_then_executes_chosen_address() {
        // One logical service reachable at two addresses; an address-specific
        // command expands them into per-address candidates to pick between.
        let mut app = app_with(
            matcher_from(&[PING_ADDR]),
            vec![ssh_multi("alpha", &["10.0.0.1", "10.0.0.2"])],
        );
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.instances().len(), 1);
        assert_eq!(app.rows[0].group.instances()[0].addresses.len(), 2);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::InstancePicker);

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
                app.rows.len()
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

        assert_eq!(app.rows.len(), 1);
        let host = &app.rows[0].group;
        assert_eq!(host.label(), "nas.local");
        // The row states the host; the differing types stay on the children.
        assert_eq!(host.facts().host(), RowHost::Resolved("nas.local"));
        assert_eq!(host.facts().service_type(), RowServiceType::Varies);
        assert_eq!(host.logical_service_count(), 2);

        // Invoking the aggregate runs the command against the concrete child
        // that matches it, not against the row.
        let command = send(&mut app, KeyCode::Enter).expect("the ssh child runs");
        assert_eq!(command.argv, vec!["ssh", "--", "nas.local"]);
    }

    #[test]
    fn a_service_type_row_targets_the_concrete_child_the_user_picks() {
        // One type offered by two hosts with different addresses and ports.
        let mut alpha = service_on("alpha", "_ssh._tcp", "alpha.local", 22);
        alpha.addresses = vec!["10.0.0.1".parse().unwrap()];
        let mut beta = service_on("beta", "_ssh._tcp", "beta.local", 2222);
        beta.addresses = vec!["10.0.0.2".parse().unwrap()];

        // `ssh -- {hostname}` names no address or port, so nothing about the rule
        // looks instance-specific — which is exactly why an aggregate row used
        // to run its first child without asking.
        let mut app = app_with(matcher_from(&[SSH]), vec![alpha, beta]);
        app.filter.grouping = GroupingMode::ServiceType;
        app.recompute_visible();

        assert_eq!(app.rows.len(), 1);
        let by_type = &app.rows[0].group;
        assert_eq!(by_type.label(), "_ssh._tcp");
        // No host is type-wide, so the row names none.
        assert_eq!(by_type.facts().host(), RowHost::Varies);
        assert_eq!(by_type.resolved_host_count(), 2);

        // The two children would ssh to two different hosts, so the aggregate
        // offers them up rather than answering for them.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::InstancePicker);
        send(&mut app, KeyCode::Down);
        let command = send(&mut app, KeyCode::Enter).expect("the chosen child runs");
        assert_eq!(command.argv, vec!["ssh", "--", "beta.local"]);
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
        assert_eq!(app.mode(), AppMode::ServicePicker);
        let command = send(&mut app, KeyCode::Enter).expect("the picked service runs");
        assert_eq!(command.argv, vec!["ssh", "--", "alpha.local"]);
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
        assert_eq!(app.rows.len(), 2);
        assert!(
            app.rows
                .iter()
                .all(|row| row.group.label() == UNRESOLVED_HOST_LABEL)
        );

        // The impostor is a real host, so it can be filtered by.
        send(&mut app, KeyCode::Char('s'));
        assert_eq!(
            app.filter.host_filter.as_deref(),
            Some(UNRESOLVED_HOST_LABEL)
        );
        assert_eq!(app.rows.len(), 1);
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
        let chosen = app.rows[app.selected].group.id().clone();

        // A new row sorts in ahead of the selection.
        let earlier = service_on("alpha", "_ssh._tcp", "alpha.local", 22);
        app.records.insert(earlier.id(), earlier);
        app.recompute_visible();

        assert_eq!(app.selected, 2, "the cursor followed its row");
        assert_eq!(*app.rows[app.selected].group.id(), chosen);
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
        assert_eq!(app.rows.len(), 1);

        send(&mut app, KeyCode::Char('s'));
        assert!(app.filter.host_filter.is_none());
        assert_eq!(app.rows.len(), 2);
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
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.instances().len(), 2);
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
        assert_eq!(app.rows.len(), 1);
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
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.label(), "beta");
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
        assert_eq!(command.argv, vec!["ssh", "--", "alpha.local"]);

        let mut many = app_with(
            matcher_from(&[SSH]),
            vec![ssh("alpha", "10.0.0.1"), ssh("beta", "10.0.0.2")],
        );
        many.filter.grouping = GroupingMode::Command;
        many.recompute_visible();
        assert_eq!(many.command_groups[0].services.len(), 2);

        assert!(send(&mut many, KeyCode::Enter).is_none());
        assert_eq!(many.mode(), AppMode::ServicePicker);
        // Services sort by label; index 1 is `beta`.
        send(&mut many, KeyCode::Down);
        let command = send(&mut many, KeyCode::Enter).expect("picked service runs");
        assert_eq!(command.argv, vec!["ssh", "--", "beta.local"]);
    }

    // ── refresh & config reload ─────────────────────────────────────────────
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

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
        assert_eq!(app.rows.len(), 2);

        send(&mut app, KeyCode::Char('r'));

        // The list is empty and a new session runs.
        assert!(app.records.is_empty());
        assert!(app.rows.is_empty());
        assert!(app.status.contains("refresh"));
        assert_eq!(spawned.lock().unwrap().len(), 1);

        // Events from the replacement session repopulate the list.
        spawned.lock().unwrap()[0]
            .send(DiscoveryEvent::Upsert(ssh("gamma", "10.0.0.3")))
            .unwrap();
        app.drain_discovery();
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.label(), "gamma");
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
        assert_eq!(app.rows.len(), 1);

        drop(tx);
        app.drain_discovery();

        assert!(app.records.is_empty(), "unverifiable records must not stay");
        assert!(app.rows.is_empty());
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

    #[test]
    fn discovery_drain_yields_after_its_per_frame_budget() {
        let (mut app, tx) = app_with_session(Matcher::default(), Vec::new());
        for index in 0..=MAX_DISCOVERY_EVENTS_PER_TICK {
            tx.send(DiscoveryEvent::Upsert(Entry::new(
                format!("service-{index}"),
                "_http._tcp",
                "local",
            )))
            .unwrap();
        }

        app.drain_discovery();
        assert_eq!(app.records.len(), MAX_DISCOVERY_EVENTS_PER_TICK);

        app.drain_discovery();
        assert_eq!(app.records.len(), MAX_DISCOVERY_EVENTS_PER_TICK + 1);
    }

    #[test]
    fn record_capacity_rejects_new_occurrences_but_allows_updates_and_reopens() {
        let (mut app, tx) = app_with_session(Matcher::default(), Vec::new());
        for index in 0..MAX_DISCOVERED_OCCURRENCES {
            let record = Entry::new(format!("service-{index}"), "_http._tcp", "local")
                .with_occurrence(Some(OccurrenceId(NonZeroU32::new(1).unwrap())));
            app.records.insert(record.id(), record);
        }

        let mut updated = Entry::new("service-0", "_http._tcp", "local")
            .with_occurrence(Some(OccurrenceId(NonZeroU32::new(1).unwrap())));
        updated.port = Some(8080);
        tx.send(DiscoveryEvent::Upsert(updated.clone())).unwrap();
        tx.send(DiscoveryEvent::Upsert(Entry::new(
            "overflow",
            "_http._tcp",
            "local",
        )))
        .unwrap();
        app.drain_discovery();

        assert_eq!(app.records.len(), MAX_DISCOVERED_OCCURRENCES);
        assert_eq!(
            app.records.get(&updated.id()).and_then(|entry| entry.port),
            Some(8080)
        );
        assert!(app.status.contains("discovery capped"), "{}", app.status);

        let removed = Entry::new("service-1", "_http._tcp", "local")
            .with_occurrence(Some(OccurrenceId(NonZeroU32::new(1).unwrap())));
        tx.send(DiscoveryEvent::Remove(removed.id())).unwrap();
        app.drain_discovery();
        assert_eq!(app.records.len(), MAX_DISCOVERED_OCCURRENCES - 1);
        assert!(app.status.contains("available again"), "{}", app.status);

        tx.send(DiscoveryEvent::Upsert(Entry::new(
            "replacement",
            "_http._tcp",
            "local",
        )))
        .unwrap();
        app.drain_discovery();
        assert_eq!(app.records.len(), MAX_DISCOVERED_OCCURRENCES);
    }

    /// A startup error's cause must survive: it is carried by the failure, not
    /// left on a status line for the next event to erase.
    #[test]
    fn a_startup_failure_keeps_its_cause_text_across_later_drains() {
        let (mut app, tx) = app_with_session(Matcher::default(), Vec::new());
        tx.send(DiscoveryEvent::Status(
            "mDNS discovery unavailable (no such device); try --backend fake in a build with the fake feature for sample records, or refresh to retry"
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
        assert_eq!(app.mode(), AppMode::ActionPicker);

        drop(tx);
        app.drain_discovery();

        assert_eq!(app.mode(), AppMode::Browse);
        assert!(app.records.is_empty());
    }

    // ── pickers against live discovery ──────────────────────────────────────

    /// The heart of this task: a picker listed a service, discovery retracted
    /// it, and confirming the picker ran a command at a host that had gone.
    #[test]
    fn removing_the_selected_service_closes_an_open_action_picker() {
        let alpha = ssh("alpha", "10.0.0.1");
        let (mut app, tx) = app_with_session(matcher_from(&[SSH, PING]), vec![alpha.clone()]);
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ActionPicker);

        tx.send(DiscoveryEvent::Remove(alpha.id())).unwrap();
        app.drain_discovery();

        assert_eq!(app.mode(), AppMode::Browse, "the picker must not survive");
        assert!(
            send(&mut app, KeyCode::Enter).is_none(),
            "confirming must not run the retracted service"
        );
    }

    /// The instance picker's targets are occurrences. Losing the one under the
    /// cursor must not hand the user's pending Enter to a sibling.
    #[test]
    fn removing_the_selected_target_closes_an_open_instance_picker() {
        let alpha = service_on("alpha", "_ssh._tcp", "alpha.local", 22);
        let beta = service_on("beta", "_ssh._tcp", "beta.local", 22);
        let (mut app, tx) =
            app_with_session(matcher_from(&[SSH]), vec![alpha.clone(), beta.clone()]);
        app.filter.grouping = GroupingMode::ServiceType;
        app.recompute_visible();

        // Two hosts prepare two different commands, so the picker opens.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::InstancePicker);
        assert_eq!(app.instance_picker().unwrap().0.targets.len(), 2);

        // Move to beta, then have discovery retract exactly beta.
        send(&mut app, KeyCode::Down);
        tx.send(DiscoveryEvent::Remove(beta.id())).unwrap();
        app.drain_discovery();

        assert_eq!(app.mode(), AppMode::Browse);
        assert!(app.status.contains("gone"), "status was: {}", app.status);
    }

    /// The stale-argv case. A service keeps its identity — the registration and
    /// endpoint are untouched — while an address it advertises is replaced. A
    /// picker holding cloned candidates would still list, and run, the address
    /// that has gone.
    #[test]
    fn updating_the_selected_address_cannot_execute_the_stale_argv() {
        let original = ssh_multi("alpha", &["10.0.0.1", "10.0.0.2"]);
        let (mut app, tx) = app_with_session(matcher_from(&[PING_ADDR]), vec![original.clone()]);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::InstancePicker);
        // Address order gives index 1 = 10.0.0.2.
        send(&mut app, KeyCode::Down);

        // The same occurrence renumbers: .2 is gone, .9 is new. Addresses are
        // not part of an occurrence's identity, so this is an update.
        let mut renumbered = original.clone();
        renumbered.addresses = ["10.0.0.1", "10.0.0.9"]
            .iter()
            .map(|a| a.parse().unwrap())
            .collect();
        assert_eq!(
            renumbered.id(),
            original.id(),
            "the occurrence must keep its identity"
        );
        tx.send(DiscoveryEvent::Upsert(renumbered)).unwrap();
        app.drain_discovery();

        assert_eq!(
            app.mode(),
            AppMode::Browse,
            "the chosen address is gone, so the picker must not stand"
        );
        assert!(
            send(&mut app, KeyCode::Enter).is_none(),
            "`echo 10.0.0.2` must be unrunnable once .2 is retracted"
        );
    }

    /// The inverse: an address the user was not on changing leaves their choice
    /// alone, still selected and still runnable.
    #[test]
    fn updating_an_unselected_address_keeps_the_instance_picker() {
        let original = ssh_multi("alpha", &["10.0.0.1", "10.0.0.2"]);
        let (mut app, tx) = app_with_session(matcher_from(&[PING_ADDR]), vec![original.clone()]);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        send(&mut app, KeyCode::Down); // 10.0.0.2

        // Replace the *other* address.
        let mut renumbered = original.clone();
        renumbered.addresses = ["10.0.0.7", "10.0.0.2"]
            .iter()
            .map(|a| a.parse().unwrap())
            .collect();
        tx.send(DiscoveryEvent::Upsert(renumbered)).unwrap();
        app.drain_discovery();

        assert_eq!(
            app.mode(),
            AppMode::InstancePicker,
            "the picker must survive"
        );
        let command = send(&mut app, KeyCode::Enter).expect("the chosen address runs");
        assert_eq!(
            command.argv,
            vec!["echo", "10.0.0.2"],
            "the cursor must still be on the address the user chose"
        );
    }

    /// A service the user was not on disappearing is not a reason to throw away
    /// their picker.
    #[test]
    fn an_unrelated_removal_keeps_the_picker_and_its_selection() {
        let alpha = ssh("alpha", "10.0.0.1");
        let unrelated = http("web");
        let (mut app, tx) = app_with_session(
            matcher_from(&[SSH, PING]),
            vec![alpha.clone(), unrelated.clone()],
        );
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ActionPicker);
        // Move onto `ping`, so the retained selection is observable.
        send(&mut app, KeyCode::Down);
        let (matches, index) = app.action_picker().unwrap();
        let chosen = matches[index].command.name.clone();

        tx.send(DiscoveryEvent::Remove(unrelated.id())).unwrap();
        app.drain_discovery();

        assert_eq!(app.mode(), AppMode::ActionPicker, "the picker must survive");
        let (matches, index) = app.action_picker().unwrap();
        assert_eq!(
            matches[index].command.name, chosen,
            "the cursor must stay on the action the user chose"
        );
    }

    /// The sharpest form of the defect: the cursor is an index into a list
    /// discovery can shorten. Removing a service *above* the chosen one slides
    /// a different service into its position, so a pending Enter would run a
    /// service the user never selected.
    #[test]
    fn removing_a_service_above_the_cursor_does_not_retarget_the_service_picker() {
        let alpha = service_on("alpha", "_ssh._tcp", "alpha.local", 22);
        let (mut app, tx) = app_with_session(
            matcher_from(&[SSH]),
            vec![
                alpha.clone(),
                service_on("beta", "_ssh._tcp", "beta.local", 22),
                service_on("gamma", "_ssh._tcp", "gamma.local", 22),
            ],
        );
        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ServicePicker);
        assert_eq!(app.command_groups[0].services.len(), 3);

        // Index 1 of [alpha, beta, gamma] is beta.
        send(&mut app, KeyCode::Down);
        assert_eq!(app.service_picker_index(), Some(1));

        // alpha goes; beta is now index 0 and gamma inherits index 1.
        tx.send(DiscoveryEvent::Remove(alpha.id())).unwrap();
        app.drain_discovery();

        assert_eq!(
            app.mode(),
            AppMode::ServicePicker,
            "the picker must survive"
        );
        assert_eq!(
            app.service_picker_index(),
            Some(0),
            "the cursor must follow beta, not stay on the index gamma now holds"
        );
        let command = send(&mut app, KeyCode::Enter).expect("the chosen service runs");
        assert_eq!(
            command.argv,
            vec!["ssh", "--", "beta.local"],
            "the service the user chose must run, never the one that inherited its index"
        );
    }

    /// A rule can stop matching without its service going anywhere: a TXT value
    /// the predicate depends on simply changes. The action under the cursor then
    /// refers to something that no longer exists, and must not be confirmable.
    #[test]
    fn a_txt_update_that_unmatches_the_rule_closes_the_action_picker() {
        const TXT_RULE: &str = r#"
[metadata]
name = "open-admin"
[match.txt.role]
equals = "admin"
[action]
command = "open {hostname}"
mode = "execute"
allow_option_like_values = true
"#;
        const ANY_HTTP: &str = r#"
[metadata]
name = "curl"
[match.service_type]
equals = "_http._tcp"
[action]
command = "curl {hostname}"
mode = "execute"
allow_option_like_values = true
"#;
        let mut record = Entry::new("nas", "_http._tcp", "local");
        record.hostname = Some("nas.local".to_string());
        record.txt.insert("role".to_string(), "admin".to_string());
        // Two matching rules, so invoking opens the action picker.
        let (mut app, tx) =
            app_with_session(matcher_from(&[TXT_RULE, ANY_HTTP]), vec![record.clone()]);

        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ActionPicker);
        let (matches, index) = app.action_picker().unwrap();
        assert_eq!(matches[index].command.name, "open-admin");

        // The role changes; `open-admin` stops matching, `curl` still does.
        let mut demoted = record.clone();
        demoted.txt.insert("role".to_string(), "guest".to_string());
        assert_eq!(demoted.id(), record.id(), "the service itself is unchanged");
        tx.send(DiscoveryEvent::Upsert(demoted)).unwrap();
        app.drain_discovery();

        assert_eq!(
            app.mode(),
            AppMode::Browse,
            "the action under the cursor no longer matches, so the picker must go"
        );
        assert!(
            app.status.contains("no longer matches"),
            "status was: {}",
            app.status
        );
        // And it must not have quietly slid onto `curl`.
        assert!(app.action_picker().is_none());
    }

    /// Explicit fake discovery: running out of samples is the normal ending of
    /// a finite stream, so the samples stay and nothing is reported as broken.
    #[cfg(feature = "fake")]
    #[test]
    fn finite_fake_completion_keeps_its_samples_and_reports_completion() {
        let mut cli = test_cli();
        cli.backend = crate::discovery::DiscoveryBackend::Fake;
        // Filtering to one type keeps the stream short.
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
            2,
            "a finished sample stream keeps its records"
        );
        assert!(app.status.contains("complete"));
        assert!(
            !app.status.contains("failed") && !app.status.contains("stopped"),
            "finishing a finite stream is not a failure: {}",
            app.status
        );
    }

    #[test]
    fn idle_redraws_only_for_content_that_visibly_animates() {
        let idle = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::ended(SessionState::Complete),
        );
        assert_eq!(idle.animation_period(), None);

        let live = app_with(Matcher::default(), vec![]);
        assert_eq!(
            live.animation_period(),
            Some((Duration::from_millis(240), 2))
        );

        let mut aged = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        aged.session = DiscoverySession::ended(SessionState::Complete);
        aged.update_layout(SCREEN);
        assert_eq!(
            aged.animation_period(),
            Some((Duration::from_secs(1), 8)),
            "logical-service details display occurrence ages"
        );

        aged.filter.grouping = GroupingMode::Host;
        aged.recompute_visible();
        aged.update_layout(SCREEN);
        assert_eq!(
            aged.animation_period(),
            None,
            "host details do not display occurrence ages"
        );

        aged.set_mode(AppMode::Search);
        assert_eq!(
            aged.animation_period(),
            Some((Duration::from_millis(480), 4))
        );
    }

    /// Fake discovery is the smoke-test surface, so it has to be able to show
    /// the behavior the app actually has. Task 006 makes an aggregate row ask
    /// which host to act on when its children would run different commands —
    /// which the sample set could not produce while it advertised a single SSH
    /// service. This pins the sample set against that behavior, so `--backend
    /// fake` stays enough to exercise the picker by hand.
    #[cfg(feature = "fake")]
    #[test]
    fn fake_samples_offer_host_selection_on_the_service_type_row() {
        let mut cli = test_cli();
        cli.backend = crate::discovery::DiscoveryBackend::Fake;
        cli.service_type = Some("_ssh._tcp".to_string());
        let session = crate::discovery::start(
            &cli.discovery_options()
                .expect("valid test discovery options"),
        );
        let mut app = App::new(cli, matcher_from(&[SSH]), KeyBindings::default(), session);
        while app.session.state().is_listening() {
            app.drain_discovery();
            std::thread::yield_now();
        }

        app.filter.grouping = GroupingMode::ServiceType;
        app.recompute_visible();

        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.label(), "_ssh._tcp");
        assert_eq!(
            app.rows[0].group.resolved_host_count(),
            2,
            "the sample set must put SSH on two hosts"
        );

        // `ssh -- {hostname}` over two hosts is two different commands.
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(
            app.mode(),
            AppMode::InstancePicker,
            "the row must ask which host rather than run one"
        );

        // Occurrences sort by registration name: raspberry-pi, then workstation.
        send(&mut app, KeyCode::Down);
        let command = send(&mut app, KeyCode::Enter).expect("the chosen host runs");
        assert_eq!(command.argv, vec!["ssh", "--", "workstation.local"]);
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
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].group.label(), "gamma");
    }

    /// The other ending: a finished sample stream is not a failure, but it is
    /// still over, and refreshing it must start a live browse rather than
    /// leave the user on a session that can never produce anything again.
    #[test]
    fn refresh_restarts_a_completed_session() {
        let (factory, spawned) = channel_factory();
        let mut app = App::new(
            test_cli(),
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::ended(SessionState::Complete),
        )
        .with_discovery_factory(factory);
        assert!(!app.session.state().is_listening());

        send(&mut app, KeyCode::Char('r'));

        assert!(
            app.session.state().is_listening(),
            "refresh must restart a completed session"
        );
        spawned.lock().unwrap()[0]
            .send(DiscoveryEvent::Upsert(ssh("gamma", "10.0.0.3")))
            .unwrap();
        app.drain_discovery();
        assert_eq!(app.rows.len(), 1);
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

        assert_eq!(app.rows.len(), 1);
        assert_eq!(
            app.rows[0].group.label(),
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
        assert_eq!(app.rows.len(), 1);

        send(&mut app, KeyCode::Char('r'));
        // The replacement session's producer dies immediately.
        spawned.lock().unwrap().clear();
        app.drain_discovery();

        assert!(app.records.is_empty());
        assert!(app.rows.is_empty());
        assert!(matches!(app.session.state(), SessionState::Failed(_)));
    }

    /// A loader that hands back a complete rule set built from `sources`.
    fn loads(sources: &'static [&'static str]) -> ConfigLoader {
        Box::new(move |_cli| ReloadOutcome::Loaded(Box::new(matcher_from(sources))))
    }

    /// A loader that rejects the configuration with `diagnostics`, as a reload
    /// does for any invalid file in the overlay.
    fn rejects(diagnostics: &'static [&'static str]) -> ConfigLoader {
        Box::new(move |_cli| {
            ReloadOutcome::Rejected(diagnostics.iter().map(|d| d.to_string()).collect())
        })
    }

    #[test]
    fn requested_reload_swaps_commands_and_recomputes_matches() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(loads(&[SSH]));
        assert_eq!(app.matcher.command_count(), 0);
        assert_eq!(app.rows[0].matches.len(), 0);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.matcher.command_count(), 1);
        assert_eq!(
            app.rows[0].matches.len(),
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
                    ReloadOutcome::Loaded(Box::new(Matcher::default()))
                })
            });
        let status = app.status.clone();

        app.poll_reload();

        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(app.status, status);
    }

    /// The sole command file is being edited and is momentarily malformed. A
    /// reload that skipped it would leave a session that can no longer do
    /// anything; the rules already in force must simply stay, action and all.
    #[test]
    fn rejected_reload_keeps_the_working_rules_runnable() {
        let mut app = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(rejects(&["/cfg/ssh.toml: unterminated quote"]));

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.matcher.command_count(), 1, "old rules stay in force");
        assert_eq!(
            app.rows[0].matches.len(),
            1,
            "and still match the discovered services"
        );
        let command = send(&mut app, KeyCode::Enter).expect("the working action still runs");
        assert_eq!(command.argv, vec!["ssh", "--", "alpha.local"]);
    }

    /// The one the old lenient reload got wrong: a valid file next to an invalid
    /// one used to install *half* the configuration over a complete rule set,
    /// silently dropping whatever the broken file defined.
    #[test]
    fn a_mixed_valid_and_invalid_overlay_does_not_partially_swap() {
        let mut app = app_with(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(rejects(&["/cfg/ping.toml: unknown placeholder `{bogus}`"]));

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(
            app.matcher.command_count(),
            2,
            "a rule set is installed whole or not at all"
        );
        assert_eq!(app.rows[0].matches.len(), 2);
        assert!(
            app.status
                .contains("keeping the 2 command(s) already loaded")
        );
    }

    /// Diagnostics outlive the status line that announced them: the next tick
    /// overwrites the message, and the detail is what the user needs on exit.
    #[test]
    fn rejected_reload_retains_full_diagnostics_across_status_updates() {
        let mut app = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(rejects(&[
                "/etc/kinjo/commands/a.toml: unterminated quote",
                "/home/u/.config/kinjo/commands/b.toml: unsupported match field `typ`",
            ]));

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();
        assert!(app.status.contains("2 invalid config file(s)"));

        // Anything at all happening in the UI replaces the status line.
        app.fail("something else entirely".to_string()).unwrap();
        assert!(!app.status.contains("invalid config file"));

        assert_eq!(
            app.reload_diagnostics,
            [
                "/etc/kinjo/commands/a.toml: unterminated quote",
                "/home/u/.config/kinjo/commands/b.toml: unsupported match field `typ`",
            ],
            "full source paths and messages survive for the exit report"
        );
    }

    /// Latest-only: a reload reports on the configuration as it is now, so a
    /// fix leaves nothing behind to print about edits already corrected.
    #[test]
    fn a_successful_reload_clears_the_previous_failures_diagnostics() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(rejects(&["/cfg/ssh.toml: unterminated quote"]));

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();
        assert!(!app.reload_diagnostics.is_empty());

        // The user fixes the file and signals again.
        app.config_loader = Some(loads(&[SSH]));
        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert!(app.reload_diagnostics.is_empty(), "latest reload only");
        assert_eq!(app.matcher.command_count(), 1);
        assert!(app.status.contains("reloaded 1 command"));
    }

    /// A later failure replaces the earlier one rather than accumulating.
    #[test]
    fn a_second_rejected_reload_replaces_the_earlier_diagnostics() {
        let mut app = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(rejects(&["/cfg/first.toml: unterminated quote"]));
        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        app.config_loader = Some(rejects(&["/cfg/second.toml: invalid action mode `frok`"]));
        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(
            app.reload_diagnostics,
            ["/cfg/second.toml: invalid action mode `frok`"]
        );
    }

    /// The command view projects rules, so an atomic swap has to be visible in
    /// its rows — not just in the matcher's count.
    #[test]
    fn a_valid_reload_swaps_every_rule_together_and_rebuilds_command_groups() {
        let mut app = app_with(matcher_from(&[SSH]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(loads(&[SSH, PING]));
        app.filter.grouping = GroupingMode::Command;
        app.recompute_visible();
        assert_eq!(app.command_groups.len(), 1);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(
            app.command_groups
                .iter()
                .map(|group| group.command.name.as_str())
                .collect::<Vec<_>>(),
            ["ssh", "ping"],
            "both new rules arrive at once, projected against current records"
        );
        assert!(app.reload_diagnostics.is_empty());
    }

    #[test]
    fn reload_closes_a_picker_built_from_the_old_rules() {
        let mut app = app_with(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(Box::new(|_cli| {
                ReloadOutcome::Loaded(Box::new(Matcher::default()))
            }));
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ActionPicker);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.mode(), AppMode::Browse);
        assert!(app.action_picker().is_none());
    }

    /// The mirror of the test above: a picker is stale only because the rules
    /// under it changed. A rejected reload changes nothing, so the choice the
    /// user is in the middle of making is still valid and must not be dropped.
    #[test]
    fn a_rejected_reload_leaves_an_open_picker_alone() {
        let mut app = app_with(matcher_from(&[SSH, PING]), vec![ssh("alpha", "10.0.0.1")])
            .with_config_loader(rejects(&["/cfg/ssh.toml: unterminated quote"]));
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::ActionPicker);

        app.reload_requested.store(true, Ordering::Relaxed);
        app.poll_reload();

        assert_eq!(app.mode(), AppMode::ActionPicker);
        assert_eq!(app.action_picker().unwrap().0.len(), 2);
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
                .handle_key(
                    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                    SCREEN
                )
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
        assert_eq!(app.mode(), AppMode::Help);

        app.handle_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            SCREEN,
        )
        .unwrap();

        assert!(app.should_quit, "ctrl-c must quit while a modal is open");
    }

    #[test]
    fn help_modal_opens_and_closes() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);

        send(&mut app, KeyCode::Char('?'));
        assert_eq!(app.mode(), AppMode::Help);

        send(&mut app, KeyCode::Esc);
        assert_eq!(app.mode(), AppMode::Browse);
    }

    #[test]
    fn scroll_details_steps_by_half_viewport_and_clamps() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        // A pane too short for the details, so half a page is a real step and
        // the far end is reachable. Both come from the layout, not from bounds
        // pushed into the app by hand.
        app.update_layout(MOUSE_SCREEN);
        let max = app.layout.details_max_scroll();
        let half = app.layout.details_viewport() / 2;
        assert!(half > 0 && max > half, "max={max} half={half}");

        send(&mut app, KeyCode::Char('d'));
        assert_eq!(app.details_scroll, half);

        for _ in 0..max {
            send(&mut app, KeyCode::Char('d'));
        }
        assert_eq!(app.details_scroll, max, "cannot scroll past the maximum");

        for _ in 0..max {
            send(&mut app, KeyCode::Char('u'));
        }
        assert_eq!(app.details_scroll, 0, "scrolling up returns to the top");
    }

    /// The bug the layout snapshot exists to make impossible: the details
    /// bounds used to be written back by the renderer, so a scroll key handled
    /// before the first frame — or against a terminal that had since been
    /// resized — was clamped to whatever the last frame happened to leave
    /// behind. The snapshot is computed before both, so there is no "last
    /// frame" to be stale.
    #[test]
    fn details_scroll_is_bounded_before_anything_has_been_drawn() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);

        // No layout has been computed: nothing has any room, so nothing scrolls.
        send(&mut app, KeyCode::Char('d'));
        assert_eq!(app.details_scroll, 0);
        app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 70, 4));
        assert_eq!(app.details_scroll, 0);
        // And no click can land on a row of a list that was never drawn.
        app.handle_mouse(mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 3));
        assert_eq!(app.selected, 0);
    }

    /// Scrolling to the bottom and then growing the terminal must not strand
    /// the reader below the content: the next snapshot is what bounds them.
    #[test]
    fn a_resize_reclamps_the_details_scroll_before_the_next_input() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        app.update_layout(MOUSE_SCREEN);
        let short_max = app.layout.details_max_scroll();
        app.details_scroll = short_max;
        assert!(short_max > 0);

        // A terminal tall enough to show the whole thing has nothing to scroll.
        app.update_layout(SCREEN);
        assert_eq!(app.layout.details_max_scroll(), 0);
        assert_eq!(app.details_scroll, 0, "the resize pulled the scroll back");

        send(&mut app, KeyCode::Char('d'));
        assert_eq!(app.details_scroll, 0, "there is nothing below to scroll to");
    }

    // ── selection identity and detail scroll ───────────────────────────────

    /// Scroll position is a place inside one row's details. When the row the
    /// user was reading is retracted, the cursor lands on a neighbour — and
    /// dropping the reader part-way down a *different* service's details is
    /// nonsense, so the replacement starts from the top.
    #[test]
    fn removing_the_selected_row_focuses_a_replacement_at_the_top_of_its_details() {
        let mut app = app_with(
            Matcher::default(),
            vec![
                ssh("alpha", "10.0.0.1"),
                ssh("beta", "10.0.0.2"),
                ssh("gamma", "10.0.0.3"),
            ],
        );
        app.update_layout(MOUSE_SCREEN);
        app.selected = 1;
        app.details_scroll = 2;
        assert_eq!(app.rows[app.selected].group.label(), "beta");

        app.records.remove(&ssh("beta", "10.0.0.2").id());
        app.recompute_visible();

        // Deterministic: the row that took beta's place, not beta's old index
        // pointing at whatever slid into it.
        assert_eq!(app.rows[app.selected].group.label(), "gamma");
        assert_eq!(app.details_scroll, 0);
    }

    /// Filtering a row away is the same event as it being retracted, as far as
    /// the reader is concerned: what they were reading is no longer on screen.
    #[test]
    fn filtering_out_the_selected_row_resets_the_details_scroll() {
        let mut app = app_with(
            Matcher::default(),
            vec![ssh("alpha", "10.0.0.1"), http("beta")],
        );
        app.update_layout(MOUSE_SCREEN);
        app.selected = app
            .rows
            .iter()
            .position(|row| row.group.label() == "beta")
            .expect("beta is listed");
        app.details_scroll = 2;

        app.filter.toggle_service_type("_http._tcp");
        app.recompute_visible();

        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[app.selected].group.label(), "alpha");
        assert_eq!(app.details_scroll, 0);
    }

    /// The other half of the rule: an update to the row being read is not a
    /// change of subject. Discovery re-reports a service constantly, and
    /// snapping the reader back to the top on every refresh would make the
    /// details unreadable.
    #[test]
    fn updating_the_selected_row_keeps_its_place_in_the_details() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        app.update_layout(MOUSE_SCREEN);
        app.details_scroll = 2;

        // The same registration, re-reported with another address.
        let mut updated = ssh("alpha", "10.0.0.1");
        updated.port = Some(2222);
        app.records.insert(updated.id(), updated);
        app.recompute_visible();

        assert_eq!(app.rows[app.selected].group.label(), "alpha");
        assert_eq!(app.details_scroll, 2, "the reader stayed where they were");
    }

    /// Kept scroll is only kept as far as the new content reaches: an update
    /// that shortens the details must not leave the reader below the end of
    /// them. Identity survives a TXT change, so this is the "same row, less to
    /// say" case rather than a change of subject.
    #[test]
    fn a_kept_scroll_is_clamped_to_the_updated_content() {
        let mut verbose = ssh("alpha", "10.0.0.1");
        for i in 0..12 {
            verbose.txt.insert(format!("key{i:02}"), i.to_string());
        }
        let mut app = app_with(Matcher::default(), vec![verbose.clone()]);
        app.update_layout(MOUSE_SCREEN);

        let tall = app.layout.details_max_scroll();
        app.details_scroll = tall;

        // The same occurrence, re-reported with its TXT data gone.
        let terse = ssh("alpha", "10.0.0.1");
        assert_eq!(terse.id(), verbose.id(), "the row's identity is unchanged");
        app.records.insert(terse.id(), terse);
        app.recompute_visible();
        app.update_layout(MOUSE_SCREEN);

        let short = app.layout.details_max_scroll();
        assert!(
            short < tall,
            "the details must have got shorter: {tall} → {short}"
        );
        assert_eq!(
            app.details_scroll, short,
            "the reader was pulled back to the new end, not stranded past it"
        );
    }

    /// An emptied list has no row to focus and nothing to read.
    #[test]
    fn losing_every_row_leaves_the_details_at_the_top() {
        let mut app = app_with(Matcher::default(), vec![ssh("alpha", "10.0.0.1")]);
        app.update_layout(MOUSE_SCREEN);
        app.details_scroll = 2;

        app.records.clear();
        app.recompute_visible();

        assert!(app.rows.is_empty());
        assert_eq!(app.selected, 0);
        assert_eq!(app.details_scroll, 0);
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
        assert_eq!(app.mode(), AppMode::Browse);
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
        assert_eq!(app.mode(), AppMode::Browse);
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
        assert_eq!(app.mode(), AppMode::ActionPicker);
        assert_eq!(app.action_picker().unwrap().0.len(), 2);

        // action_index 1 is the missing-binary command (insertion order).
        send(&mut app, KeyCode::Down);
        assert!(send(&mut app, KeyCode::Enter).is_none());
        assert_eq!(app.mode(), AppMode::Browse);
        assert!(app.action_picker().is_none());
        assert!(app.status.contains("cannot run `ghost`"));
    }
}
