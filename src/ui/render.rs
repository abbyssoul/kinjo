use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table,
    },
};

use crate::{
    discovery::{
        ChildService, Entry, EntryGroup, GroupFacts, GroupingMode, HostAggregate, LogicalService,
        RowHost, RowServiceType, ServiceTypeAggregate, SessionState, TxtValue,
    },
    plumber::{MatchResult, Requirement},
};

use super::{
    app::{App, AppMode, CommandGroup},
    display,
    keymap::Action,
    viewport::Window,
};

// ── palette ──────────────────────────────────────────────────────────────
const ACCENT: Color = Color::Rgb(0x7a, 0xa2, 0xf7); // soft blue
const ACCENT_DIM: Color = Color::Rgb(0x3b, 0x4a, 0x6b);
const BG_BAR: Color = Color::Rgb(0x1a, 0x1b, 0x26);
const BG_SEL: Color = Color::Rgb(0x28, 0x3b, 0x5c);
const FG_DIM: Color = Color::Rgb(0x6b, 0x70, 0x89);
const GOOD: Color = Color::Rgb(0x9e, 0xce, 0x6a); // green
const WARN: Color = Color::Rgb(0xe0, 0xaf, 0x68); // amber
const STAR: Color = Color::Rgb(0xf7, 0xce, 0x52); // yellow

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// The indicators for a session that has finished and one that has stopped.
/// Single-width, like the spinner frames they stand in for, so the tab strip
/// beside them sits in the same column whatever discovery is doing.
const DONE: &str = "✓";
const STOPPED: &str = "✗";

/// What the UI says discovery is doing.
///
/// Animation is a claim: something is happening in the background, and the list
/// may still change. Only a listening session may make it. A finished sample
/// stream and a dead browse are both *over* — a spinner above either one invites
/// the user to go on waiting for services that can never arrive. That is the
/// lie task 002 removed from the list body; this removes the rest of it.
///
/// The mapping is a pure function of [`SessionState`], and both the top bar and
/// the empty list body render from one of these, so the indicator and the words
/// under it cannot come to different conclusions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activity {
    /// The producer is running; the list may still change.
    Listening,
    /// A finite sample stream ended normally. Nothing is arriving, and nothing
    /// is wrong.
    Complete,
    /// The browse is over and its records are no longer being confirmed.
    Failed,
}

impl Activity {
    fn of(state: &SessionState) -> Self {
        match state {
            SessionState::Listening => Self::Listening,
            SessionState::Complete => Self::Complete,
            SessionState::Failed(_) => Self::Failed,
        }
    }

    /// The glyph for tick `ticks`. Only [`Activity::Listening`] varies with the
    /// tick, so a session that has ended draws the same frame however long it
    /// is left on screen.
    fn symbol(self, ticks: u64) -> &'static str {
        match self {
            Self::Listening => SPINNER[(ticks / 2) as usize % SPINNER.len()],
            Self::Complete => DONE,
            Self::Failed => STOPPED,
        }
    }

    /// The palette's existing "fine" and "needs attention" vocabulary, so a
    /// stopped browse reads the same way as the failure text beneath it.
    fn color(self) -> Color {
        match self {
            Self::Listening | Self::Complete => GOOD,
            Self::Failed => WARN,
        }
    }
}

/// Draw the whole screen from `app` and the layout it already computed.
///
/// Reads only: every rectangle and bound comes from `app.layout`, which the
/// event loop worked out before this was called. Nothing here decides geometry,
/// so nothing here has to be told to input handling afterwards.
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let layout = app.layout;

    render_top_bar(frame, app, layout.top_bar());
    render_filter_bar(frame, app, layout.filter_bar());

    if app.filter.grouping == GroupingMode::Command {
        render_commands(frame, app, layout.list());
        render_command_details(frame, app, layout.details());
    } else {
        render_services(frame, app, layout.list());
        render_details(frame, app, layout.details());
    }

    render_footer(frame, app, layout.footer());

    match app.mode {
        AppMode::TypeFilter => render_type_filter(frame, app),
        AppMode::ActionPicker => render_action_picker(frame, app),
        AppMode::InstancePicker => render_instance_picker(frame, app),
        AppMode::ServicePicker => render_service_picker(frame, app),
        AppMode::Help => render_help(frame, app),
        AppMode::Browse | AppMode::Search => {}
    }
}

// ── top bar (view tabs) ────────────────────────────────────────────────────
fn render_top_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // Whether discovery is running is discovery's to say, not something to
    // guess from whether records happen to have arrived.
    let activity = Activity::of(app.session.state());

    let mut spans = vec![
        Span::styled(
            display::text(&format!(" {} ", activity.symbol(app.ticks))),
            Style::default().fg(activity.color()),
        ),
        Span::styled(
            "kinjo  ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
    ];

    // Each tab is labelled with how many rows it lists: logical services, hosts
    // (plus the unresolved row when there is one), service types, or configured
    // commands. `App` counts them from the same filtered records it builds the
    // rows from, so a count can never contradict the list behind its tab.
    for (mode, count) in GroupingMode::TABS.into_iter().zip(app.tab_counts) {
        let active = mode == app.filter.grouping;
        let text = format!(" {} {count} ", mode.tab_title());
        let style = if active {
            Style::default()
                .fg(BG_BAR)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG_DIM)
        };
        spans.push(Span::styled(display::text(&text), style));
        spans.push(Span::raw(" "));
    }

    // right-aligned domain chip
    let domain = Span::styled(
        display::text(&format!("  {}  ", app.cli.domain)),
        Style::default()
            .fg(BG_BAR)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    push_right_aligned(&mut spans, vec![domain], area.width);

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(BG_BAR)),
        area,
    );
}

// ── filter / search bar ──────────────────────────────────────────────────
fn render_filter_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let searching = matches!(app.mode, AppMode::Search);
    let prompt_style = if searching {
        Style::default()
            .fg(BG_BAR)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ACCENT_DIM)
    };

    let mut spans = vec![Span::styled(" / ", prompt_style), Span::raw(" ")];

    if app.filter.text_query.is_empty() && !searching {
        // Name the key that actually opens search; with search unbound there is
        // no key to name, so the bar just says what the field is.
        let placeholder = match app.keybindings.compact(Action::OpenSearch) {
            Some(key) => format!("fuzzy filter — press {key} to search"),
            None => "fuzzy filter".to_string(),
        };
        spans.push(Span::styled(
            display::text(&placeholder),
            Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
        ));
    } else {
        spans.push(Span::styled(
            display::text(&app.filter.text_query),
            Style::default().fg(Color::White),
        ));
        if searching && (app.ticks / 4).is_multiple_of(2) {
            spans.push(Span::styled("▌", Style::default().fg(ACCENT)));
        }
    }

    // right-side chips: type filter + host filter
    let mut chips: Vec<Span> = Vec::new();
    // Both numbers from one read of the filter, over the types discovery is
    // offering now. A count derived here from a set length would go on
    // counting types that have already gone away.
    let (enabled, total_types) = app.filter.type_counts();
    if total_types > 0 {
        let narrowed = enabled < total_types;
        chips.push(chip(
            &format!(" types {enabled}/{total_types} "),
            if narrowed { WARN } else { FG_DIM },
        ));
    }
    if let Some(host) = &app.filter.host_filter {
        chips.push(Span::raw(" "));
        chips.push(chip(&format!(" host:{host} ✕ "), Color::Magenta));
    }

    push_right_aligned(&mut spans, chips, area.width);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn chip(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        display::text(text),
        Style::default()
            .fg(BG_BAR)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

/// What an empty list says about discovery itself, once filters have been ruled
/// out as the reason it is empty.
///
/// Every branch starts from the same [`Activity`] the top bar draws, so the
/// indicator and these words are two renderings of one fact rather than two
/// opinions that have to be kept in step by hand.
fn session_lines(app: &App) -> Vec<Line<'static>> {
    let activity = Activity::of(app.session.state());
    let symbol = activity.symbol(app.ticks);
    let headline = Style::default().fg(activity.color());
    let detail = Style::default().fg(FG_DIM);

    match app.session.state() {
        SessionState::Listening => vec![
            Line::from(""),
            Line::from(Span::styled(
                display::text(&format!(
                    "  {symbol} listening for mDNS services on {}…",
                    app.cli.domain
                )),
                detail,
            )),
        ],
        // The sample stream ran to its end and had nothing to show. Not a
        // failure — nothing went wrong — but nothing will arrive later either,
        // so "listening…" here would leave the user waiting for an empty list
        // to fill itself in.
        SessionState::Complete => vec![
            Line::from(""),
            Line::from(Span::styled(
                display::text(&format!("  {symbol} sample discovery complete")),
                headline,
            )),
            Line::from(Span::styled("  no sample services to show", detail)),
        ],
        // Discovery is over and its cause is worth acting on.
        SessionState::Failed(failure) => vec![
            Line::from(""),
            Line::from(Span::styled(
                display::text(&format!("  {symbol} {}", failure.headline())),
                headline,
            )),
            Line::from(Span::styled(
                display::text(&format!("  {}", failure.cause)),
                detail,
            )),
        ],
    }
}

// ── service list ─────────────────────────────────────────────────────────
fn render_services(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let total = app.rows.len();

    let empty = if app.filter.is_active() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  no services match the active filters",
                Style::default().fg(WARN),
            )),
            Line::from(Span::styled(
                "  press esc to clear search, t to adjust types",
                Style::default().fg(FG_DIM),
            )),
        ]
    } else {
        session_lines(app)
    };

    // Column layout — Table handles per-cell truncation and alignment so that
    // wide/unicode service names no longer shift the trailing columns.
    let widths = [
        Constraint::Length(2),  // selection gutter + dot
        Constraint::Fill(5),    // name
        Constraint::Length(14), // service type
        Constraint::Length(4),  // instance count badge
        Constraint::Length(4),  // matching-commands badge
        Constraint::Fill(4),    // host
    ];

    // Drive the panel header from the active tab so switching views (services /
    // hosts / types — all rendered here) updates the title to match, instead of
    // always reading " services ".
    let label = format!(" {} ", app.filter.grouping.tab_title());

    render_list_panel(
        frame,
        area,
        ListPanelSpec {
            label: &label,
            selected: app.selected,
            total,
            widths: &widths,
            empty,
        },
        |index| {
            service_row(
                &app.rows[index].group,
                index == app.selected,
                app.rows[index].matches.len(),
            )
        },
    );
}

/// One row of the service/host/type list.
///
/// The two variable columns say only what the row's projection guarantees: a
/// logical service has one type and one host to name, while a host or
/// service-type row instead reports aggregates over its children — never one
/// child's type, port, or host standing in for the whole row.
fn service_row(group: &EntryGroup, selected: bool, matches: usize) -> Row<'static> {
    let color = row_color(group.facts());
    let base = selection_style(selected);
    let gutter = gutter_span(selected);

    let name_style = if selected {
        base.fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        base.fg(Color::White)
    };

    let (detail, detail_color) = match group.facts() {
        GroupFacts::LogicalService(service) => (short_type(&service.service_type), color),
        // How many logical services the aggregate collects — a fact about the
        // whole row, unlike the arbitrary first child's type this used to show.
        GroupFacts::Host(_) | GroupFacts::ServiceType(_) => {
            (format!("{} svc", group.logical_service_count()), FG_DIM)
        }
    };

    let trailing = match group.facts() {
        GroupFacts::LogicalService(service) => service
            .hostname
            .clone()
            .unwrap_or_else(|| "…resolving".to_string()),
        // Every type this host offers, not one of them.
        GroupFacts::Host(_) => group
            .service_types()
            .iter()
            .map(|service_type| short_type(service_type))
            .collect::<Vec<_>>()
            .join(", "),
        // How many hosts offer this type, since no single one does.
        GroupFacts::ServiceType(_) => format!("{} hosts", group.resolved_host_count()),
    };

    // occurrence count badge
    let n = group.occurrence_count();
    let count = if n > 1 {
        Span::styled(
            display::text(&format!("×{n}")),
            base.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("", base)
    };

    // matching-commands badge
    let matches_cell = if matches > 0 {
        Span::styled(
            display::text(&format!("★{matches}")),
            base.fg(STAR).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("·", base.fg(ACCENT_DIM))
    };

    Row::new(vec![
        Cell::from(Line::from(vec![gutter, Span::styled("●", base.fg(color))])),
        Cell::from(Span::styled(display::text(group.label()), name_style)),
        Cell::from(Span::styled(display::text(&detail), base.fg(detail_color))),
        Cell::from(count),
        Cell::from(matches_cell),
        Cell::from(Span::styled(display::text(&trailing), base.fg(FG_DIM))),
    ])
    .style(base)
}

/// A row's accent colour: the category of the service type it can vouch for, or
/// a neutral tone when it aggregates several types and no one colour is honest.
fn row_color(facts: &GroupFacts) -> Color {
    match facts.service_type() {
        RowServiceType::Invariant(service_type) => category_color(service_type),
        RowServiceType::Varies => FG_DIM,
    }
}

/// The host text a row can truthfully show.
fn host_text(facts: &GroupFacts) -> String {
    match facts.host() {
        RowHost::Resolved(host) => host.to_string(),
        RowHost::Unresolved => "…resolving".to_string(),
        RowHost::Varies => "several hosts".to_string(),
    }
}

// ── details / preview ────────────────────────────────────────────────────
/// How many lines the details pane has for the current selection.
///
/// This is the content dimension [`LayoutSnapshot`] needs, and it comes from the
/// same builders that draw the pane rather than from a count kept alongside
/// them: the bound a scroll key is clamped to and the rows that end up on screen
/// cannot disagree about how tall the content is, because they are the same
/// rows. An unselected pane shows a note rather than a table, so it has no lines
/// to scroll.
///
/// [`LayoutSnapshot`]: super::layout::LayoutSnapshot
pub(crate) fn details_content_height(app: &App) -> usize {
    let rows = if app.filter.grouping == GroupingMode::Command {
        command_detail_rows(app)
    } else {
        detail_rows(app)
    };
    rows.map_or(0, |rows| rows.len())
}

fn render_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = panel().title(Line::from(Span::styled(
        " details ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));

    let Some(rows) = detail_rows(app) else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no service selected",
                Style::default().fg(FG_DIM),
            )))
            .block(block),
            area,
        );
        return;
    };

    render_detail_rows(frame, app, area, block, rows);
}

/// The details of the selected browse row, or `None` when nothing is selected.
/// Pure: a function of the app's rows and matches, with no geometry in it.
fn detail_rows(app: &App) -> Option<Vec<Row<'static>>> {
    let row = app.rows.get(app.selected)?;
    let group = &row.group;

    // header — name spans the value column, dot sits in the label column
    let mut rows: Vec<Row> = vec![Row::new(vec![
        Cell::from(Span::styled(
            " ●",
            Style::default().fg(row_color(group.facts())),
        )),
        Cell::from(Span::styled(
            display::text(group.label()),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
    ])];

    // Each projection describes itself. The aggregates list their children
    // rather than presenting one child's fields as though they were the row's.
    match group.facts() {
        GroupFacts::LogicalService(service) => {
            push_logical_service_rows(&mut rows, group, service);
        }
        GroupFacts::Host(host) => push_host_rows(&mut rows, group, host),
        GroupFacts::ServiceType(aggregate) => push_service_type_rows(&mut rows, group, aggregate),
    }

    rows.push(blank_row());
    push_action_rows(&mut rows, &row.matches);

    Some(rows)
}

/// A logical service: the fields all of its occurrences share, its TXT data,
/// and the occurrences themselves.
fn push_logical_service_rows(
    rows: &mut Vec<Row<'static>>,
    group: &EntryGroup,
    service: &LogicalService,
) {
    let color = category_color(&service.service_type);
    rows.push(field_row("type", &service.service_type, color));
    rows.push(field_row("domain", &service.domain, FG_DIM));
    rows.push(field_row(
        "host",
        service.hostname.as_deref().unwrap_or("…resolving"),
        FG_DIM,
    ));
    rows.push(field_row("port", &port_text(service.port), FG_DIM));

    // TXT data is not part of what makes occurrences one logical service, so it
    // may differ between them; only the keys they agree on get a value here.
    let txt = group.txt();
    if !txt.is_empty() {
        rows.push(blank_row());
        rows.push(section_row("TXT records"));
        for (key, value) in &txt {
            rows.push(txt_row(key, value));
        }
    }

    rows.push(blank_row());
    rows.push(section_row(&format!(
        "occurrences ({})",
        group.occurrence_count()
    )));
    let last = group.occurrence_count().saturating_sub(1);
    for (i, record) in group.instances().iter().enumerate() {
        let branch = if i == last { "└─" } else { "├─" };
        rows.push(Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(format!(" {branch} "), Style::default().fg(ACCENT_DIM)),
                Span::styled(
                    "●",
                    Style::default().fg(category_color(&record.service_type)),
                ),
            ])),
            Cell::from(Line::from(vec![
                // Addresses differ per occurrence, so each names its own.
                Span::styled(
                    display::text(&instance_endpoint(record)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    display::text(&format!("  {}s", record.last_seen.elapsed().as_secs())),
                    Style::default().fg(FG_DIM),
                ),
            ])),
        ]));
    }
}

/// A host: its name, how much it is running, and which services those are. The
/// services' types, ports, and TXT data are theirs, so they stay on their own
/// lines rather than being promoted to host-wide fields.
fn push_host_rows(rows: &mut Vec<Row<'static>>, group: &EntryGroup, host: &HostAggregate) {
    rows.push(field_row(
        "host",
        host.hostname.as_deref().unwrap_or("not resolved yet"),
        FG_DIM,
    ));
    rows.push(field_row(
        "seen",
        &format!("{} occurrence(s)", group.occurrence_count()),
        FG_DIM,
    ));

    rows.push(blank_row());
    rows.push(section_row(&format!(
        "services ({})",
        group.logical_service_count()
    )));
    // The host is the row; repeating it on every child would say nothing.
    push_child_service_rows(rows, &group.child_services(), ChildEndpoint::PortOnly);
}

/// A service type: the type, how many hosts offer it, and which services those
/// are. No host, port, or TXT set is type-wide.
fn push_service_type_rows(
    rows: &mut Vec<Row<'static>>,
    group: &EntryGroup,
    aggregate: &ServiceTypeAggregate,
) {
    rows.push(field_row(
        "type",
        &aggregate.service_type,
        category_color(&aggregate.service_type),
    ));
    rows.push(field_row(
        "hosts",
        &format!("{} resolved", group.resolved_host_count()),
        FG_DIM,
    ));
    rows.push(field_row(
        "seen",
        &format!("{} occurrence(s)", group.occurrence_count()),
        FG_DIM,
    ));

    rows.push(blank_row());
    rows.push(section_row(&format!(
        "services ({})",
        group.logical_service_count()
    )));
    // The type is the row, but each child is on its own host.
    push_child_service_rows(rows, &group.child_services(), ChildEndpoint::HostAndPort);
}

/// How much of a child service's endpoint its parent row leaves for it to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildEndpoint {
    /// The parent row already names the host every child is on.
    PortOnly,
    /// The parent spans several hosts, so each child names its own.
    HostAndPort,
}

fn push_child_service_rows(
    rows: &mut Vec<Row<'static>>,
    children: &[ChildService],
    endpoint: ChildEndpoint,
) {
    let last = children.len().saturating_sub(1);
    for (i, child) in children.iter().enumerate() {
        let branch = if i == last { "└─" } else { "├─" };
        let port = port_text(child.facts.port);
        let endpoint = match endpoint {
            ChildEndpoint::PortOnly => format!(":{port}"),
            ChildEndpoint::HostAndPort => format!(
                "{}:{port}",
                child.facts.hostname.as_deref().unwrap_or("…resolving")
            ),
        };
        let occurrences = if child.occurrences > 1 {
            format!("  ×{}", child.occurrences)
        } else {
            String::new()
        };
        rows.push(Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(format!(" {branch} "), Style::default().fg(ACCENT_DIM)),
                Span::styled(
                    "●",
                    Style::default().fg(category_color(&child.facts.service_type)),
                ),
            ])),
            Cell::from(Line::from(vec![
                Span::styled(
                    display::text(&child.facts.name),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    display::text(&format!(
                        "  {}  {endpoint}{occurrences}",
                        short_type(&child.facts.service_type)
                    )),
                    Style::default().fg(FG_DIM),
                ),
            ])),
        ]));
    }
}

/// The actions matching a row. They are matched against the row's concrete
/// occurrences, so this section is valid for every projection.
fn push_action_rows(rows: &mut Vec<Row<'static>>, actions: &[MatchResult]) {
    rows.push(section_row(&format!("actions ({})", actions.len())));
    if actions.is_empty() {
        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled(
                "no configured commands match this row",
                Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
            )),
        ]));
        return;
    }
    for action in actions {
        rows.push(action_row(action));
    }
    rows.push(Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled(
            "press ⏎ to run",
            Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
        )),
    ]));
}

fn txt_row(key: &str, value: &TxtValue) -> Row<'static> {
    let value = match value {
        TxtValue::Shared(value) => {
            Span::styled(display::text(value), Style::default().fg(Color::White))
        }
        // The occurrences disagree: there is no service-wide value to show, and
        // showing one of them would be a claim about all of them.
        TxtValue::Mixed => Span::styled(
            "‹differs per occurrence›",
            Style::default().fg(WARN).add_modifier(Modifier::ITALIC),
        ),
    };
    Row::new(vec![
        Cell::from(Span::styled(
            display::text(&format!(" {key}")),
            Style::default().fg(WARN),
        )),
        Cell::from(Line::from(vec![
            Span::styled("= ", Style::default().fg(ACCENT_DIM)),
            value,
        ])),
    ])
}

fn port_text(port: Option<u16>) -> String {
    port.map(|port| port.to_string())
        .unwrap_or_else(|| "…".to_string())
}

fn field_row(label: &str, value: &str, value_color: Color) -> Row<'static> {
    let label = display::text(label);
    let padding = 7usize.saturating_sub(Span::raw(label.as_str()).width());
    Row::new(vec![
        Cell::from(Span::styled(
            format!("{}{label} ", " ".repeat(padding)),
            Style::default().fg(FG_DIM),
        )),
        Cell::from(Span::styled(
            display::text(value),
            Style::default().fg(value_color),
        )),
    ])
}

fn section_row(title: &str) -> Row<'static> {
    Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled(
            display::text(title),
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
    ])
}

fn blank_row() -> Row<'static> {
    Row::new(vec![Cell::from(""), Cell::from("")])
}

fn action_row(action: &MatchResult) -> Row<'static> {
    let description = action
        .command
        .action
        .description
        .as_deref()
        .or(action.command.description.as_deref())
        .unwrap_or("");
    let mode = action.command.action.mode.to_string();
    let mut spans = vec![Span::styled(
        display::text(&action.command.name),
        Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
    )];
    if !description.is_empty() {
        spans.push(Span::styled(
            display::text(&format!(" — {description}")),
            Style::default().fg(Color::White),
        ));
    }
    spans.push(Span::styled(
        display::text(&format!("  [{mode}]")),
        Style::default().fg(ACCENT_DIM),
    ));
    Row::new(vec![
        Cell::from(Span::styled(" ★", Style::default().fg(STAR))),
        Cell::from(Line::from(spans)),
    ])
}

// ── command list (group-by-command view) ──────────────────────────────────
fn render_commands(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let total = app.command_groups.len();
    let empty = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  no commands configured",
            Style::default().fg(FG_DIM),
        )),
    ];
    let widths = [
        Constraint::Length(2),  // selection gutter + dot
        Constraint::Fill(5),    // command name
        Constraint::Length(11), // matching-services badge
        Constraint::Fill(6),    // description
    ];
    render_list_panel(
        frame,
        area,
        ListPanelSpec {
            label: " commands ",
            selected: app.selected,
            total,
            widths: &widths,
            empty,
        },
        |index| command_row(&app.command_groups[index], index == app.selected),
    );
}

fn command_row(group: &CommandGroup, selected: bool) -> Row<'static> {
    let base = selection_style(selected);
    let count = group.services.len();
    let active = count > 0;
    let gutter = gutter_span(selected);

    let name_style = if active {
        base.fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        base.fg(FG_DIM)
    };

    let count_cell = if active {
        Span::styled(
            display::text(&format!("★{count} svc")),
            base.fg(STAR).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("·", base.fg(ACCENT_DIM))
    };

    let description = group
        .command
        .description
        .as_deref()
        .or(group.command.action.description.as_deref())
        .unwrap_or("");

    Row::new(vec![
        Cell::from(Line::from(vec![
            gutter,
            Span::styled("●", base.fg(if active { STAR } else { ACCENT_DIM })),
        ])),
        Cell::from(Span::styled(display::text(&group.command.name), name_style)),
        Cell::from(count_cell),
        Cell::from(Span::styled(display::text(description), base.fg(FG_DIM))),
    ])
    .style(base)
}

fn render_command_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = panel().title(Line::from(Span::styled(
        " command ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));

    let Some(rows) = command_detail_rows(app) else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no command selected",
                Style::default().fg(FG_DIM),
            )))
            .block(block),
            area,
        );
        return;
    };

    render_detail_rows(frame, app, area, block, rows);
}

/// The details of the selected command row, or `None` when nothing is selected.
/// The command view's counterpart to [`detail_rows`], and pure for the same
/// reason.
fn command_detail_rows(app: &App) -> Option<Vec<Row<'static>>> {
    let group = app.command_groups.get(app.selected)?;

    let command = &group.command;
    let mut rows: Vec<Row> = vec![Row::new(vec![
        Cell::from(Span::styled(" ★", Style::default().fg(STAR))),
        Cell::from(Span::styled(
            display::text(&command.name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
    ])];
    if let Some(description) = command
        .description
        .as_deref()
        .or(command.action.description.as_deref())
    {
        rows.push(field_row("desc", description, Color::White));
    }
    rows.push(field_row("mode", &command.action.mode.to_string(), FG_DIM));
    rows.push(field_row("run", &command.action.command, GOOD));
    if !command.requirements.is_empty() {
        let needs = command
            .requirements
            .iter()
            .map(Requirement::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(field_row("needs", &needs, WARN));
    }

    rows.push(blank_row());
    rows.push(section_row(&format!(
        "matching services ({})",
        group.services.len()
    )));
    if group.services.is_empty() {
        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled(
                "no discovered service matches this command",
                Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
            )),
        ]));
    } else {
        let last = group.services.len().saturating_sub(1);
        for (i, service) in group.services.iter().enumerate() {
            let branch = if i == last { "└─" } else { "├─" };
            rows.push(Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(format!(" {branch} "), Style::default().fg(ACCENT_DIM)),
                    Span::styled("●", Style::default().fg(row_color(service.facts()))),
                ])),
                Cell::from(Line::from(vec![
                    Span::styled(
                        display::text(service.label()),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        display::text(&format!("  {}", host_text(service.facts()))),
                        Style::default().fg(FG_DIM),
                    ),
                ])),
            ]));
        }
        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled(
                "press ⏎ to pick a service and run",
                Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
            )),
        ]));
    }

    Some(rows)
}

// ── footer ───────────────────────────────────────────────────────────────
/// The footer hints for `mode`, spelled with the keys actually bound to each
/// action. An unbound action contributes no hint at all, so the footer never
/// advertises a key that does nothing.
fn footer_hints(app: &App, mode: AppMode) -> Vec<(String, &'static str)> {
    let keys = &app.keybindings;
    let hint = |actions: &[Action], label: &'static str| {
        keys.compact_group(actions).map(|shown| (shown, label))
    };
    match mode {
        AppMode::Search => [
            Some(("type".to_string(), "filter")),
            hint(&[Action::SearchClose], "done"),
            hint(&[Action::SearchClear], "clear"),
        ],
        AppMode::TypeFilter => [
            hint(&[Action::TypeFilterDown, Action::TypeFilterUp], "move"),
            hint(&[Action::TypeFilterToggle], "toggle"),
            hint(&[Action::TypeFilterClose], "close"),
        ],
        AppMode::ActionPicker | AppMode::InstancePicker | AppMode::ServicePicker => [
            hint(&[Action::PickerDown, Action::PickerUp], "move"),
            hint(&[Action::PickerSelect], "run"),
            hint(&[Action::PickerClose], "cancel"),
        ],
        AppMode::Help => [hint(&[Action::HelpClose], "close"), None, None],
        AppMode::Browse => return browse_footer_hints(app),
    }
    .into_iter()
    .flatten()
    .collect()
}

fn browse_footer_hints(app: &App) -> Vec<(String, &'static str)> {
    let keys = &app.keybindings;
    let hint = |actions: &[Action], label: &'static str| {
        keys.compact_group(actions).map(|shown| (shown, label))
    };
    [
        hint(&[Action::MoveDown, Action::MoveUp], "move"),
        hint(&[Action::Invoke], "open"),
        hint(&[Action::OpenSearch], "search"),
        hint(&[Action::OpenTypeFilter], "types"),
        hint(&[Action::TabNext], "view"),
        hint(&[Action::SameHost], "same-host"),
        hint(&[Action::Refresh], "refresh"),
        hint(&[Action::DetailsDown, Action::DetailsUp], "scroll"),
        hint(&[Action::OpenHelp], "help"),
        // Whichever quit survives customization is the one worth showing, and
        // validation guarantees one of them does.
        keys.compact(Action::BrowseQuit)
            .or_else(|| keys.compact(Action::Quit))
            .map(|shown| (shown, "quit")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let hints = footer_hints(app, app.mode);

    let mut spans = vec![Span::raw(" ")];
    for (key, label) in &hints {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(BG_BAR)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}   "),
            Style::default().fg(FG_DIM),
        ));
    }

    // right-aligned status message
    let status = Span::styled(
        display::text(&format!("  {} ", app.status)),
        Style::default().fg(ACCENT_DIM),
    );
    push_right_aligned(&mut spans, vec![status], area.width);

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(BG_BAR)),
        area,
    );
}

// ── modals ───────────────────────────────────────────────────────────────
/// A modal's bottom-border hint, spelled from the keys bound to each action.
/// Each part may name several actions covered by one verb (the down/up pair
/// behind "scrolls"). Parts with nothing bound drop out rather than leaving a
/// stale key on the border.
fn modal_hint(app: &App, parts: &[(&[Action], &str)]) -> String {
    parts
        .iter()
        .filter_map(|(actions, verb)| {
            app.keybindings
                .compact_group(actions)
                .map(|keys| format!("{keys} {verb}"))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The hint shared by the action, instance, and service pickers.
fn picker_hint(app: &App) -> String {
    modal_hint(
        app,
        &[
            (&[Action::PickerSelect], "runs"),
            (&[Action::PickerClose], "closes"),
        ],
    )
}

fn render_type_filter(frame: &mut Frame<'_>, app: &App) {
    render_picker(
        frame,
        PickerSpec {
            title: " service types ",
            hint: &modal_hint(
                app,
                &[
                    (&[Action::TypeFilterToggle], "toggles"),
                    (&[Action::TypeFilterClose], "closes"),
                ],
            ),
            empty: "  no service types discovered yet",
            selected: app.type_filter_index,
            total: app.filter.discovered_types().len(),
            width: 58,
            height: 60,
        },
        |index, selected, base| {
            let service_type = &app.filter.discovered_types()[index];
            let check = if app.filter.is_enabled(service_type) {
                Span::styled(" ✓ ", base.fg(GOOD).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(" ○ ", base.fg(FG_DIM))
            };
            Line::from(vec![
                gutter_span(selected),
                check,
                Span::styled(" ● ", base.fg(category_color(service_type))),
                Span::styled(display::text(service_type), base.fg(Color::White)),
            ])
        },
    );
}

fn render_action_picker(frame: &mut Frame<'_>, app: &App) {
    render_picker(
        frame,
        PickerSpec {
            title: " matching actions ",
            hint: &picker_hint(app),
            empty: "  no matching actions",
            selected: app.action_index,
            total: app.action_matches.len(),
            width: 70,
            height: 50,
        },
        |index, selected, base| {
            let action = &app.action_matches[index];
            let description = action
                .command
                .action
                .description
                .as_deref()
                .or(action.command.description.as_deref())
                .unwrap_or("");
            let mut spans = vec![
                gutter_span(selected),
                Span::styled("★ ", base.fg(STAR)),
                Span::styled(
                    display::text(&action.command.name),
                    base.fg(GOOD).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    display::text(&format!(" — {description}")),
                    base.fg(Color::White),
                ),
            ];
            if action.needs_selection() {
                spans.push(Span::styled("  ⊙ choose instance", base.fg(WARN)));
            }
            Line::from(spans)
        },
    );
}

fn render_instance_picker(frame: &mut Frame<'_>, app: &App) {
    let records = app
        .pending_action
        .as_ref()
        .map(|action| action.targets.as_slice())
        .unwrap_or(&[]);
    render_picker(
        frame,
        PickerSpec {
            title: " select instance ",
            hint: &picker_hint(app),
            empty: "  no instance to choose from",
            selected: app.instance_index,
            total: records.len(),
            width: 72,
            height: 55,
        },
        |index, selected, base| {
            let record = &records[index];
            Line::from(vec![
                gutter_span(selected),
                Span::styled("● ", base.fg(category_color(&record.service_type))),
                Span::styled(
                    display::text(&instance_endpoint(record)),
                    base.fg(Color::White),
                ),
            ])
        },
    );
}

fn render_service_picker(frame: &mut Frame<'_>, app: &App) {
    let group = app.command_groups.get(app.selected);
    let command_name = group.map(|g| g.command.name.as_str()).unwrap_or_default();
    let services = group.map(|g| g.services.as_slice()).unwrap_or(&[]);
    render_picker(
        frame,
        PickerSpec {
            title: &format!(" run {command_name} on "),
            hint: &picker_hint(app),
            empty: "  no service to run this on",
            selected: app.service_picker_index,
            total: services.len(),
            width: 72,
            height: 55,
        },
        |index, selected, base| {
            let service = &services[index];
            Line::from(vec![
                gutter_span(selected),
                Span::styled("● ", base.fg(row_color(service.facts()))),
                Span::styled(display::text(service.label()), base.fg(Color::White)),
                Span::styled(
                    display::text(&format!("  {}", host_text(service.facts()))),
                    base.fg(FG_DIM),
                ),
            ])
        },
    );
}

/// The help overlay is the reference, so it lists every key bound to an action
/// rather than the footer's compact spelling.
fn help_rows(app: &App) -> Vec<(String, &'static str)> {
    let keys = &app.keybindings;
    let row = |actions: &[Action], label: &'static str| {
        keys.describe_group(actions).map(|shown| (shown, label))
    };
    [
        row(&[Action::MoveDown], "move selection down"),
        row(&[Action::MoveUp], "move selection up"),
        row(&[Action::DetailsDown], "details down ½ page"),
        row(&[Action::DetailsUp], "details up ½ page"),
        row(&[Action::Invoke], "run matching action(s)"),
        row(&[Action::OpenSearch], "fuzzy text filter"),
        // Leaving search keeps the query; only `clear` removes it.
        row(&[Action::SearchClose], "leave search, keep filter"),
        row(&[Action::SearchClear], "clear the search filter"),
        row(&[Action::OpenTypeFilter], "service-type checklist"),
        row(
            &[Action::TabNext, Action::TabPrev],
            "switch view tab (services/hosts/types/commands)",
        ),
        row(&[Action::SameHost], "filter to selected host"),
        row(&[Action::Refresh], "refresh: restart service discovery"),
        row(&[Action::PickerClose], "close a modal"),
        row(&[Action::OpenHelp], "toggle this help"),
        row(&[Action::BrowseQuit, Action::Quit], "quit"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Every line of the help overlay, in order: the key rows the bindings generate,
/// then the badges legend. The content exists independently of the popup that
/// shows it, so [`App`] can size a scroll action against the same lines the user
/// is looking at.
pub(crate) fn help_lines(app: &App) -> Vec<Line<'static>> {
    let rows = help_rows(app);
    let width = rows
        .iter()
        .map(|(key, _)| Span::raw(key.as_str()).width())
        .max()
        .unwrap_or(0);
    let mut lines = vec![Line::from("")];
    for (key, label) in &rows {
        let key = display::text(key);
        let padding = (width + 2).saturating_sub(Span::raw(key.as_str()).width());
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {key}{}", " ".repeat(padding)),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(label.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   badges:  ×N occurrences   ★N matching commands",
        Style::default().fg(FG_DIM),
    )));
    lines
}

/// The help popup's share of the screen. Wider and taller than a picker: the
/// key column lists every alias of every action, so the rows are longer than the
/// old hard-coded ones.
const HELP_POPUP: (u16, u16) = (72, 80);

fn help_popup_area(frame_area: Rect) -> Rect {
    centered_rect(HELP_POPUP.0, HELP_POPUP.1, frame_area)
}

/// How many help lines a `frame_area`-sized screen can show at once.
///
/// The scroll bound belongs to whoever handles the key, but the height it
/// depends on is a fact about this layout. Exposing it as a pure function lets
/// [`App`] clamp a scroll against the same geometry the next frame will use,
/// instead of the renderer writing the bound back into the app.
pub(crate) fn help_viewport(frame_area: Rect) -> usize {
    popup_inner(help_popup_area(frame_area)).height as usize
}

/// The help overlay: a window onto [`help_lines`] rather than all of them.
///
/// Help is generated from the user's bindings, so its length is not something
/// the layout can be sized for in advance — a few extra aliases, or a short
/// terminal, and the tail falls off the bottom. Windowing it means the rows the
/// popup cannot show are scrolled to instead of lost.
fn render_help(frame: &mut Frame<'_>, app: &App) {
    let lines = help_lines(app);
    let area = help_popup_area(frame.area());
    let window = Window::at(lines.len(), help_viewport(frame.area()), app.help_scroll);
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(window.offset())
        .take(window.range().len())
        .collect();

    render_popup(
        frame,
        PopupSpec {
            area,
            title: " help ",
            hint: &help_hint(app, window),
            window,
        },
        Paragraph::new(visible).alignment(Alignment::Left),
    );
}

/// The help overlay's bottom-border hint. The scroll keys are named only while
/// there is something to scroll to, and always with the keys bound to the scroll
/// actions, so a rebinding moves the hint with it.
fn help_hint(app: &App, window: Window) -> String {
    let mut parts: Vec<(&[Action], &str)> = Vec::new();
    if window.is_clipped() {
        parts.push((&[Action::HelpDown, Action::HelpUp], "scrolls"));
    }
    parts.push((&[Action::HelpClose], "closes"));
    modal_hint(app, &parts)
}

/// A keyboard-selected modal list.
struct PickerSpec<'a> {
    title: &'a str,
    hint: &'a str,
    /// Shown in place of the list when there is nothing to choose from.
    empty: &'a str,
    selected: usize,
    total: usize,
    width: u16,
    height: u16,
}

/// Render a picker as a window onto its items, never as all of them.
///
/// The window is derived from the popup that is actually being drawn, so the
/// selected item is on screen at any terminal size — including after a resize,
/// since the next frame simply recomputes it. `line_at` is called only for the
/// visible indices, which is what makes it safe for it to index the list
/// directly.
fn render_picker(
    frame: &mut Frame<'_>,
    spec: PickerSpec<'_>,
    mut line_at: impl FnMut(usize, bool, Style) -> Line<'static>,
) {
    let area = centered_rect(spec.width, spec.height, frame.area());
    let inner = popup_inner(area);
    let window = Window::containing(spec.total, inner.height as usize, spec.selected);

    let items: Vec<ListItem> = if spec.total == 0 {
        vec![ListItem::new(Line::from(Span::styled(
            display::text(spec.empty),
            Style::default().fg(FG_DIM),
        )))]
    } else {
        window
            .range()
            .map(|index| {
                let selected = index == spec.selected;
                let base = selection_style(selected);
                ListItem::new(line_at(index, selected, base).style(base))
            })
            .collect()
    };

    render_popup(
        frame,
        PopupSpec {
            area,
            title: spec.title,
            hint: spec.hint,
            window,
        },
        List::new(items),
    );
}

/// A modal popup: where it is, what it says, and how much of its content it is
/// showing.
struct PopupSpec<'a> {
    area: Rect,
    title: &'a str,
    hint: &'a str,
    window: Window,
}

/// The content rectangle inside a popup's border. Kept in step with the block
/// built by [`render_popup`], so the window is computed for the height the
/// content will really get.
fn popup_inner(area: Rect) -> Rect {
    popup_block().inner(area)
}

fn popup_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
}

fn render_popup<W>(frame: &mut Frame<'_>, spec: PopupSpec<'_>, widget: W)
where
    W: ratatui::widgets::Widget,
{
    frame.render_widget(Clear, spec.area);
    let mut title = vec![Span::styled(
        display::text(spec.title),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    // Say where in the content this window is — but only when some of it is out
    // of sight, so a modal that shows everything stays uncluttered.
    if spec.window.is_clipped()
        && let Some(range) = spec.window.range_label()
    {
        title.push(Span::styled(
            format!("{range} "),
            Style::default().fg(FG_DIM),
        ));
    }
    let block = popup_block()
        .title(Line::from(title))
        .title_bottom(Span::styled(
            display::text(&format!(" {} ", spec.hint)),
            Style::default().fg(FG_DIM),
        ));
    let inner = block.inner(spec.area);
    frame.render_widget(block, spec.area);
    frame.render_widget(widget, inner);
    render_scrollbar(
        frame,
        spec.area,
        spec.window.total(),
        spec.window.range().len(),
        spec.window.offset(),
    );
}

// ── helpers ──────────────────────────────────────────────────────────────
fn panel() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT_DIM))
}

fn gutter_span(selected: bool) -> Span<'static> {
    if selected {
        Span::styled("▌", Style::default().fg(ACCENT).bg(BG_SEL))
    } else {
        Span::raw(" ")
    }
}

fn instance_endpoint(record: &Entry) -> String {
    let host = record.hostname.as_deref().unwrap_or("…resolving");
    let addr = if record.addresses.is_empty() {
        "…".to_string()
    } else {
        record
            .addresses
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let port = record
        .port
        .map(|p| p.to_string())
        .unwrap_or_else(|| "…".to_string());
    format!("{host}  {addr}:{port}")
}

fn short_type(service_type: &str) -> String {
    // _https._tcp -> https
    service_type
        .trim_start_matches('_')
        .split('.')
        .next()
        .unwrap_or(service_type)
        .to_string()
}

fn category_color(service_type: &str) -> Color {
    let t = short_type(service_type);
    let t = t.as_str();
    if matches!(
        t,
        "ssh" | "sftp-ssh" | "telnet" | "rfb" | "vnc" | "rdp" | "nx"
    ) {
        GOOD
    } else if matches!(
        t,
        "http" | "https" | "webdav" | "webdavs" | "caldav" | "carddav"
    ) {
        ACCENT
    } else if matches!(
        t,
        "ipp" | "ipps" | "printer" | "pdl-datastream" | "scanner" | "uscan"
    ) {
        Color::Magenta
    } else if matches!(
        t,
        "smb" | "afpovertcp" | "nfs" | "ftp" | "webdav-fs" | "sftp"
    ) {
        WARN
    } else if matches!(
        t,
        "airplay" | "raop" | "googlecast" | "spotify-connect" | "dlna" | "daap" | "sonos"
    ) {
        Color::Rgb(0xbb, 0x9a, 0xf7) // soft purple
    } else if matches!(
        t,
        "workstation" | "device-info" | "homekit" | "hap" | "companion-link"
    ) {
        Color::Cyan
    } else {
        FG_DIM
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

/// Draw a vertical scrollbar on the right border of `area`. Renders nothing when
/// the whole content already fits, so panels without overflow stay uncluttered.
fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    content_len: usize,
    viewport: usize,
    position: usize,
) {
    if content_len <= viewport {
        return;
    }
    let mut state = ScrollbarState::new(content_len)
        .viewport_content_length(viewport)
        .position(position);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_style(Style::default().fg(ACCENT))
            .track_style(Style::default().fg(ACCENT_DIM)),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

/// Where the browse list's window starts. The same calculation the modals use,
/// so a list row and a picker row stay visible for the same reason.
pub(crate) fn scroll_offset(selected: usize, total: usize, view_h: usize) -> usize {
    Window::containing(total, view_h, selected).offset()
}

/// Background highlight for the currently selected row/item.
fn selection_style(selected: bool) -> Style {
    if selected {
        Style::default().bg(BG_SEL)
    } else {
        Style::default()
    }
}

/// Append `right` to `spans` so it sits flush against the right edge of a
/// `width`-wide bar, padding the gap with spaces. Used by the top/filter/footer
/// status bars.
fn push_right_aligned<'a>(spans: &mut Vec<Span<'a>>, right: Vec<Span<'a>>, width: u16) {
    let len = |list: &[Span]| -> usize { list.iter().map(Span::width).sum() };
    let pad = (width as usize)
        .saturating_sub(len(spans))
        .saturating_sub(len(&right));
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);
}

/// Panel title with the `label` plus a `first-last/total` range chip when the
/// list is non-empty. Shared by the service and command list panels.
fn list_title(label: &str, total: usize, selected: usize, inner_h: usize) -> Line<'static> {
    let mut spans = vec![Span::styled(
        display::text(label),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if let Some(range) = Window::containing(total, inner_h, selected).range_label() {
        spans.push(Span::styled(
            format!("{range} "),
            Style::default().fg(FG_DIM),
        ));
    }
    Line::from(spans)
}

/// Render a scrollable left-hand list panel: title, empty-state, the visible row
/// window, and a scrollbar. `row_at` builds the row for a given index.
struct ListPanelSpec<'a> {
    label: &'a str,
    selected: usize,
    total: usize,
    widths: &'a [Constraint],
    empty: Vec<Line<'static>>,
}

fn render_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    spec: ListPanelSpec<'_>,
    mut row_at: impl FnMut(usize) -> Row<'static>,
) {
    let inner_h = area.height.saturating_sub(2) as usize;
    let block = panel().title(list_title(spec.label, spec.total, spec.selected, inner_h));
    if spec.total == 0 {
        frame.render_widget(Paragraph::new(spec.empty).block(block), area);
        return;
    }
    let offset = scroll_offset(spec.selected, spec.total, inner_h);
    let rows: Vec<Row> = (offset..spec.total)
        .take(inner_h)
        .map(&mut row_at)
        .collect();
    frame.render_widget(
        Table::new(rows, spec.widths.iter().copied())
            .column_spacing(1)
            .block(block),
        area,
    );
    render_scrollbar(frame, area, spec.total, inner_h, offset);
}

/// Render the shared tail of the two detail panes: slice the window the layout
/// says is on screen, and draw the table + scrollbar.
///
/// The window comes from the snapshot rather than from this pane's rectangle, so
/// the rows drawn here are the ones the scroll keys were clamped against.
fn render_detail_rows<'a>(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    block: Block<'a>,
    rows: Vec<Row<'a>>,
) {
    let window = app.layout.details_window(app.details_scroll);
    let visible: Vec<Row> = rows
        .into_iter()
        .skip(window.offset())
        .take(window.range().len())
        .collect();

    let widths = [Constraint::Length(8), Constraint::Fill(1)];
    frame.render_widget(
        Table::new(visible, widths).column_spacing(1).block(block),
        area,
    );
    render_scrollbar(
        frame,
        area,
        window.total(),
        app.layout.details_viewport(),
        window.offset(),
    );
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, num::NonZeroU32};

    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use crate::{
        discovery::{
            BrowseMode, DiscoveryBackend, DiscoveryFailure, DiscoverySession, FailureKind,
            OccurrenceId, browse_groups,
        },
        plumber::{ActionMode, CommandAction, CommandConfig, Matcher},
        test_support::{remove, temp_file},
        ui::{
            app::{App, BrowseRow},
            cli::{Cli, CliCommand},
            filter::fuzzy_match,
            keymap::KeyBindings,
        },
    };

    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn test_app(domain: &str) -> App {
        App::new(
            Cli {
                domain: domain.to_string(),
                config_dirs: Vec::new(),
                service_type: None,
                backend: DiscoveryBackend::MdnsSd,
                command: CliCommand::Run,
            },
            Matcher::default(),
            KeyBindings::default(),
            DiscoverySession::inert(),
        )
    }

    /// Draw `app` onto a `width`×`height` terminal exactly as the event loop
    /// does: compute the layout for that size first, then render from it. Taking
    /// `&mut App` is the point — a frame cannot be drawn without the app having
    /// been told what size it is being drawn at.
    fn render_buffer(app: &mut App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        app.update_layout(terminal.get_frame().area());
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Put `count` distinct service types in front of the type filter the only
    /// way it learns any: by observing records that advertise them. Returns the
    /// type names, in the sorted order the filter lists them in.
    fn discover_types(app: &mut App, count: usize, name: impl Fn(usize) -> String) -> Vec<String> {
        let types: Vec<String> = (0..count).map(&name).collect();
        let records: Vec<Entry> = types
            .iter()
            .map(|service_type| Entry::new("svc", service_type, "local"))
            .collect();
        app.filter.observe_types(&records);
        types
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut output = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                output.push_str(buffer[(x, y)].symbol());
            }
            output.push('\n');
        }
        output
    }

    /// An empty list while a browse is running is a quiet network, and saying
    /// so is honest.
    #[test]
    fn an_empty_list_on_a_live_session_says_it_is_listening() {
        let mut app = test_app("local");

        let text = buffer_text(&render_buffer(&mut app, 100, 24));

        assert!(text.contains("listening for mDNS services on local"));
    }

    /// The same empty list after discovery has failed must not claim to be
    /// listening: nothing is browsing, so no service can ever arrive, and the
    /// user would wait forever for one.
    #[test]
    fn an_empty_list_on_a_failed_session_reports_the_failure_not_listening() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = test_app("local");
        app.session = DiscoverySession::detached(rx);
        // The producer goes away; polling is what notices.
        drop(tx);
        app.session.poll();

        let text = buffer_text(&render_buffer(&mut app, 100, 24));

        assert!(
            !text.contains("listening for mDNS services"),
            "a dead browse must not be rendered as a live one: {text}"
        );
        assert!(text.contains("discovery stopped"));
        assert!(text.contains("the browse ended unexpectedly"));
    }

    // ── discovery activity indicator ───────────────────────────────────────

    fn failure(kind: FailureKind, cause: &str) -> SessionState {
        SessionState::Failed(DiscoveryFailure {
            kind,
            cause: cause.to_string(),
        })
    }

    /// An app whose discovery session is in `state`: a live session for
    /// `Listening`, and one that has already ended for either ending.
    fn app_in(state: SessionState) -> App {
        let mut app = test_app("local");
        app.session = match state {
            SessionState::Listening => DiscoverySession::inert(),
            ended => DiscoverySession::ended(ended),
        };
        app
    }

    #[test]
    fn the_activity_of_a_session_follows_its_state() {
        assert_eq!(Activity::of(&SessionState::Listening), Activity::Listening);
        assert_eq!(Activity::of(&SessionState::Complete), Activity::Complete);
        assert_eq!(
            Activity::of(&failure(FailureKind::Startup, "no runtime")),
            Activity::Failed
        );
        assert_eq!(
            Activity::of(&failure(FailureKind::Stopped, "it died")),
            Activity::Failed
        );
    }

    /// Animation is the claim that something is happening. Only a listening
    /// session may make it; the other two draw one frame forever.
    #[test]
    fn only_a_listening_session_animates() {
        let frames = |activity: Activity| {
            (0..40)
                .map(|ticks| activity.symbol(ticks))
                .collect::<BTreeSet<_>>()
        };

        assert_eq!(frames(Activity::Listening).len(), SPINNER.len());
        assert_eq!(frames(Activity::Complete), BTreeSet::from([DONE]));
        assert_eq!(frames(Activity::Failed), BTreeSet::from([STOPPED]));
    }

    /// The three states must be tellable apart at a glance, and a stopped
    /// browse must not borrow the colour of a healthy one.
    #[test]
    fn the_ended_states_are_distinguishable_from_each_other_and_from_listening() {
        assert_ne!(Activity::Complete.symbol(0), Activity::Failed.symbol(0));
        assert_ne!(Activity::Complete.color(), Activity::Failed.color());
        assert_eq!(Activity::Failed.color(), WARN);
        // Whatever the tick, a live spinner is never mistaken for an ending.
        for ticks in 0..40 {
            assert_ne!(Activity::Listening.symbol(ticks), DONE);
            assert_ne!(Activity::Listening.symbol(ticks), STOPPED);
        }
    }

    /// Every indicator occupies one column, so the tab strip beside it sits in
    /// the same place whatever discovery is doing and however it animates.
    #[test]
    fn every_indicator_is_one_column_wide() {
        let symbols = SPINNER
            .iter()
            .copied()
            .chain([DONE, STOPPED])
            .collect::<Vec<_>>();

        for symbol in symbols {
            assert_eq!(Span::raw(symbol).width(), 1, "{symbol:?}");
        }
    }

    /// The frame a user is left looking at must not keep changing once there is
    /// nothing left to happen — byte-for-byte, not just "looks stopped".
    #[test]
    fn an_ended_session_renders_an_identical_frame_at_every_tick() {
        for state in [
            SessionState::Complete,
            failure(FailureKind::Stopped, "the browse ended unexpectedly"),
        ] {
            let mut app = app_in(state.clone());
            app.ticks = 0;
            let first = render_buffer(&mut app, 100, 24);

            for ticks in [1, 2, 7, 21, 99] {
                app.ticks = ticks;
                assert_eq!(
                    render_buffer(&mut app, 100, 24),
                    first,
                    "{state:?} changed at tick {ticks}"
                );
            }
        }
    }

    /// The control: a live browse *does* animate, so the test above is
    /// measuring stillness rather than a spinner that never moved.
    #[test]
    fn a_listening_session_still_animates() {
        let mut app = test_app("local");
        app.ticks = 0;
        let first = render_buffer(&mut app, 100, 24);
        app.ticks = 2;

        assert_ne!(render_buffer(&mut app, 100, 24), first);
    }

    /// The top bar and the body are two renderings of one fact. A finished
    /// sample stream with nothing in it must not be described as a live browse
    /// by either of them.
    #[test]
    fn a_complete_session_reports_completion_rather_than_listening() {
        let mut app = app_in(SessionState::Complete);

        let text = buffer_text(&render_buffer(&mut app, 100, 24));

        assert!(
            !text.contains("listening for mDNS services"),
            "a finished stream must not be rendered as a live one: {text}"
        );
        assert!(text.contains("sample discovery complete"), "{text}");
        assert!(text.contains(DONE), "{text}");
        // Finishing normally is not a failure, and must not be dressed as one.
        assert!(!text.contains("stopped"), "{text}");
        assert!(!text.contains(STOPPED), "{text}");
    }

    /// The failed body already said so; now the top bar agrees with it.
    #[test]
    fn a_failed_session_shows_the_stopped_indicator_beside_its_cause() {
        let mut app = app_in(failure(FailureKind::Stopped, "the browse ended"));

        let text = buffer_text(&render_buffer(&mut app, 100, 24));

        assert!(text.contains(STOPPED), "{text}");
        assert!(text.contains("discovery stopped"), "{text}");
        assert!(text.contains("the browse ended"), "{text}");
        assert!(!text.contains(DONE), "{text}");
    }

    /// A filtered-empty list is not a statement about discovery, so it keeps
    /// its own message — but the indicator above it still tells the truth.
    #[test]
    fn an_active_filter_explains_an_empty_list_without_contradicting_the_indicator() {
        let mut app = app_in(SessionState::Complete);
        app.filter.text_query = "nothing matches this".to_string();

        let text = buffer_text(&render_buffer(&mut app, 100, 24));

        assert!(
            text.contains("no services match the active filters"),
            "{text}"
        );
        assert!(!text.contains("listening for mDNS services"), "{text}");
        // The top bar is still the one that says what discovery is doing.
        assert!(text.contains(DONE), "{text}");
    }

    /// The real thing, end to end: a finite sample stream, drained to its own
    /// natural ending through the real `App`, must leave a still frame that
    /// says it finished — not a spinner over a browse that is already over.
    #[cfg(feature = "fake")]
    #[test]
    fn a_real_finite_fake_stream_ends_on_a_still_complete_frame() {
        let cli = Cli {
            domain: "local".to_string(),
            config_dirs: Vec::new(),
            // Filtering to one type keeps the stream short.
            service_type: Some("_ssh._tcp".to_string()),
            backend: DiscoveryBackend::Fake,
            command: CliCommand::Run,
        };
        let session = crate::discovery::start(
            &cli.discovery_options()
                .expect("valid test discovery options"),
        );
        let mut app = App::new(cli, Matcher::default(), KeyBindings::default(), session);

        while app.session.state().is_listening() {
            app.drain_discovery();
            std::thread::yield_now();
        }
        assert_eq!(*app.session.state(), SessionState::Complete);

        app.ticks = 0;
        let first = render_buffer(&mut app, 100, 24);
        app.ticks = 5;

        assert_eq!(
            render_buffer(&mut app, 100, 24),
            first,
            "a finished sample stream must not go on animating"
        );
        assert!(buffer_text(&first).contains(DONE));
    }

    /// The indicator is a function of whichever session is current, so the
    /// replacement a refresh installs animates again — and then follows its own
    /// ending rather than the one it replaced.
    #[test]
    fn the_indicator_follows_the_session_a_refresh_installs() {
        let mut app = app_in(failure(FailureKind::Stopped, "the browse ended"));
        assert!(buffer_text(&render_buffer(&mut app, 100, 24)).contains(STOPPED));

        // What `refresh_services` does: install a replacement session.
        app.session = DiscoverySession::inert();
        let text = buffer_text(&render_buffer(&mut app, 100, 24));
        assert!(
            !text.contains(STOPPED),
            "a refreshed browse is live: {text}"
        );
        assert!(text.contains("listening for mDNS services"), "{text}");

        // The replacement's own ending is the one that shows.
        app.session = DiscoverySession::ended(SessionState::Complete);
        let text = buffer_text(&render_buffer(&mut app, 100, 24));
        assert!(text.contains(DONE), "{text}");
        assert!(!text.contains(STOPPED), "{text}");
    }

    /// The indicator sits at the far left of the top bar, so it is the last
    /// thing to be squeezed out — and squeezing it must not panic.
    #[test]
    fn the_indicator_is_safe_in_a_terminal_with_no_room_for_it() {
        for state in [
            SessionState::Listening,
            SessionState::Complete,
            failure(FailureKind::Startup, "no runtime"),
        ] {
            let mut app = app_in(state.clone());
            for (width, height) in [(100u16, 24u16), (20, 6), (10, 4), (3, 2), (1, 1)] {
                let buffer = render_buffer(&mut app, width, height);
                assert_eq!(buffer.area.width, width, "{state:?} at {width}x{height}");
            }
        }
    }

    #[test]
    fn short_type_strips_leading_underscore_and_protocol() {
        assert_eq!(short_type("_https._tcp"), "https");
        assert_eq!(short_type("_ssh._tcp"), "ssh");
        assert_eq!(short_type("bare"), "bare");
    }

    #[test]
    fn category_color_groups_related_service_types() {
        assert_eq!(category_color("_ssh._tcp"), GOOD);
        assert_eq!(category_color("_https._tcp"), ACCENT);
        assert_eq!(category_color("_ipp._tcp"), Color::Magenta);
        assert_eq!(category_color("_smb._tcp"), WARN);
        assert_eq!(category_color("_workstation._tcp"), Color::Cyan);
        // Anything uncategorised falls back to the dim foreground.
        assert_eq!(category_color("_unknown._tcp"), FG_DIM);
    }

    #[test]
    fn instance_endpoint_shows_placeholders_until_resolved() {
        let pending = Entry::new("alpha", "_ssh._tcp", "local");
        assert_eq!(instance_endpoint(&pending), "…resolving  …:…");

        let mut resolved = Entry::new("alpha", "_ssh._tcp", "local");
        resolved.hostname = Some("alpha.local".to_string());
        resolved.addresses = vec!["192.0.2.5".parse().unwrap()];
        resolved.port = Some(22);
        assert_eq!(instance_endpoint(&resolved), "alpha.local  192.0.2.5:22");
    }

    #[test]
    fn scroll_offset_keeps_selection_in_view() {
        // Everything fits: no scrolling.
        assert_eq!(scroll_offset(4, 5, 10), 0);
        // Selection within the first window: still pinned to the top.
        assert_eq!(scroll_offset(3, 20, 5), 0);
        // Selection past the window: scroll so the selected row is the last visible.
        assert_eq!(scroll_offset(6, 20, 5), 2);
        // Never scroll past the end of the content.
        assert_eq!(scroll_offset(19, 20, 5), 15);
        // A zero-height viewport never scrolls.
        assert_eq!(scroll_offset(5, 20, 0), 0);
    }

    #[test]
    fn push_right_aligned_pads_to_the_full_width() {
        let mut spans = vec![Span::raw("left")];
        push_right_aligned(&mut spans, vec![Span::raw("right")], 20);

        let line = Line::from(spans);
        assert_eq!(line.width(), 20);
        assert!(line_text(&line).starts_with("left"));
    }

    #[test]
    fn push_right_aligned_uses_terminal_columns_for_unicode() {
        let mut spans = vec![Span::raw(display::text("界e\u{301}🙂\u{1b}"))];
        push_right_aligned(&mut spans, vec![Span::raw(display::text("終\u{7}"))], 24);

        assert_eq!(Line::from(spans).width(), 24);
    }

    #[test]
    fn push_right_aligned_does_not_underflow_when_too_narrow() {
        let mut spans = vec![Span::raw("left")];
        // Width smaller than the combined content must not panic.
        push_right_aligned(&mut spans, vec![Span::raw("right")], 3);

        assert_eq!(line_text(&Line::from(spans)), "leftright");
    }

    #[test]
    fn list_title_adds_a_range_chip_only_when_non_empty() {
        let empty = list_title("Services", 0, 0, 10);
        assert_eq!(line_text(&empty), "Services");

        let populated = list_title("Services", 42, 0, 10);
        let text = line_text(&populated);
        assert!(text.starts_with("Services"));
        assert!(text.contains("1-10/42"));
    }

    #[test]
    fn render_escapes_untrusted_controls_without_mutating_raw_values() {
        let raw_name = "svc\u{1b}[2J\u{7}\r\n\t\u{7f}\u{85}終";
        let raw_domain = "entry\ndomain";
        let raw_host = "host\u{1b}.local";
        let raw_txt_key = "key\t";
        let raw_txt_value = "value\u{7}\r\n\u{7f}\u{85}";

        let mut entry = Entry::new(raw_name, "_ssh._tcp", raw_domain);
        entry.hostname = Some(raw_host.to_string());
        entry.port = Some(22);
        entry
            .txt
            .insert(raw_txt_key.to_string(), raw_txt_value.to_string());

        let mut app = test_app("cli\u{1b}\ndomain");
        app.status = "status\u{7}\r\u{85}".to_string();
        app.mode = AppMode::Search;
        app.filter.text_query = "query\t\u{7f}".to_string();
        app.filter.observe_types(std::slice::from_ref(&entry));
        app.rows =
            rows_without_matches(&browse_groups(&[entry.clone()], BrowseMode::LogicalService));
        app.records.insert(entry.id(), entry.clone());

        let buffer = render_buffer(&mut app, 180, 36);
        let rendered = buffer_text(&buffer);

        assert!(rendered.contains("svc\\x1B[2J\\x07\\x0D\\x0A\\x09\\x7F\\x85終"));
        assert!(rendered.contains("entry\\x0Adomain"));
        assert!(rendered.contains("host\\x1B.local"));
        assert!(rendered.contains("key\\x09"));
        assert!(rendered.contains("value\\x07\\x0D\\x0A\\x7F\\x85"));
        assert!(rendered.contains("cli\\x1B\\x0Adomain"));
        assert!(rendered.contains("status\\x07\\x0D\\x85"));
        assert!(rendered.contains("query\\x09\\x7F"));
        assert!(
            !buffer
                .content()
                .iter()
                .flat_map(|cell| cell.symbol().chars())
                .any(char::is_control)
        );

        let stored = app.records.values().next().unwrap();
        assert_eq!(stored.name, raw_name);
        assert_eq!(stored.domain, raw_domain);
        assert_eq!(stored.hostname.as_deref(), Some(raw_host));
        assert_eq!(
            stored.txt.get(raw_txt_key).map(String::as_str),
            Some(raw_txt_value)
        );
        assert!(fuzzy_match(&stored.searchable_text(), "svc\u{1b}[2J"));
    }

    // ── mode-aware aggregate rendering ─────────────────────────────────────

    fn service_on(
        name: &str,
        service_type: &str,
        host: &str,
        port: u16,
        txt: &[(&str, &str)],
    ) -> Entry {
        let mut record = Entry::new(name, service_type, "local");
        record.hostname = Some(host.to_string());
        record.addresses = vec!["192.0.2.1".parse().unwrap()];
        record.port = Some(port);
        record.txt = txt
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        record
    }

    /// An app showing `records` under `mode`, as a render pass would see it.
    fn browsing(mode: GroupingMode, records: &[Entry]) -> App {
        let mut app = test_app("local");
        app.filter.grouping = mode;
        for record in records {
            app.records.insert(record.id(), record.clone());
        }
        app.filter.observe_types(records);
        app.rows = rows_without_matches(&browse_groups(
            records,
            mode.browse_mode().expect("a browse mode"),
        ));
        app
    }

    /// Browse rows for tests that are about how a group *renders*, not about
    /// which rules match it. Building the rows here rather than assigning two
    /// vectors is what keeps a row's matches its own: there is no length to get
    /// wrong.
    fn rows_without_matches(groups: &[EntryGroup]) -> Vec<BrowseRow> {
        groups
            .iter()
            .map(|group| BrowseRow {
                group: group.clone(),
                matches: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn host_details_list_every_service_and_claim_no_representative_metadata() {
        // One host offering SSH and HTTP on different ports with different TXT.
        let mut app = browsing(
            GroupingMode::Host,
            &[
                service_on("shell", "_ssh._tcp", "nas.local", 22, &[("v", "2")]),
                service_on("site", "_http._tcp", "nas.local", 80, &[("path", "/admin")]),
            ],
        );

        let rendered = buffer_text(&render_buffer(&mut app, 180, 36));

        // Both children are listed with their own type and port.
        assert!(rendered.contains("shell"), "{rendered}");
        assert!(rendered.contains("site"), "{rendered}");
        assert!(rendered.contains(":22"));
        assert!(rendered.contains(":80"));
        assert!(rendered.contains("services (2)"));
        assert!(rendered.contains("2 occurrence(s)"));

        // No child's TXT data is presented as the host's.
        assert!(!rendered.contains("TXT records"), "{rendered}");
        assert!(!rendered.contains("/admin"), "{rendered}");
    }

    #[test]
    fn service_type_details_list_every_host_and_claim_no_representative_metadata() {
        // One type offered by several hosts with different metadata, plus a
        // registration that has not resolved a host yet.
        let mut app = browsing(
            GroupingMode::ServiceType,
            &[
                service_on("alpha", "_ssh._tcp", "alpha.local", 22, &[("os", "linux")]),
                service_on("beta", "_ssh._tcp", "beta.local", 2222, &[("os", "bsd")]),
                Entry::new("ghost", "_ssh._tcp", "local"),
            ],
        );

        let rendered = buffer_text(&render_buffer(&mut app, 180, 36));

        // Every child host is listed against its own service.
        assert!(rendered.contains("alpha.local:22"), "{rendered}");
        assert!(rendered.contains("beta.local:2222"), "{rendered}");
        assert!(rendered.contains("services (3)"));
        // Only the hosts that resolved count as hosts.
        assert!(rendered.contains("2 resolved"));
        assert!(rendered.contains("3 occurrence(s)"));

        // No child's TXT data is presented as the type's.
        assert!(!rendered.contains("TXT records"), "{rendered}");
        assert!(!rendered.contains("linux"), "{rendered}");
    }

    #[test]
    fn logical_service_details_show_shared_txt_and_flag_values_that_differ() {
        // Two occurrences of one service that disagree about their TXT data.
        let mut wired = service_on(
            "alpha",
            "_ssh._tcp",
            "alpha.local",
            22,
            &[("model", "rpi5"), ("iface", "eth0")],
        );
        wired.addresses = vec!["10.0.0.1".parse().unwrap()];
        let wired = wired.with_occurrence(Some(OccurrenceId(NonZeroU32::new(1).unwrap())));
        let mut wireless = service_on(
            "alpha",
            "_ssh._tcp",
            "alpha.local",
            22,
            &[("model", "rpi5"), ("iface", "wlan0")],
        );
        wireless.addresses = vec!["10.0.0.2".parse().unwrap()];
        let wireless = wireless.with_occurrence(Some(OccurrenceId(NonZeroU32::new(2).unwrap())));

        let mut app = browsing(GroupingMode::LogicalService, &[wired, wireless]);
        assert_eq!(app.rows.len(), 1);

        let rendered = buffer_text(&render_buffer(&mut app, 180, 36));

        // The value both occurrences agree on is the service's.
        assert!(rendered.contains("rpi5"), "{rendered}");
        // The value they disagree on is nobody's: neither is shown as the
        // service's, and the disagreement is stated instead.
        assert!(rendered.contains("differs per occurrence"), "{rendered}");
        assert!(!rendered.contains("eth0"), "{rendered}");
        assert!(!rendered.contains("wlan0"), "{rendered}");

        // The addresses that differ appear per occurrence.
        assert!(rendered.contains("occurrences (2)"));
        assert!(rendered.contains("10.0.0.1"), "{rendered}");
        assert!(rendered.contains("10.0.0.2"), "{rendered}");
    }

    #[test]
    fn an_unresolved_host_row_renders_as_unresolved_not_as_a_host() {
        let mut app = browsing(
            GroupingMode::Host,
            &[Entry::new("ghost", "_ipp._tcp", "local")],
        );

        let rendered = buffer_text(&render_buffer(&mut app, 180, 36));

        assert!(rendered.contains("<unresolved host>"), "{rendered}");
        assert!(rendered.contains("not resolved yet"), "{rendered}");
        assert!(rendered.contains("services (1)"));
    }

    #[test]
    fn every_view_renders_with_nothing_discovered() {
        for mode in GroupingMode::TABS {
            let mut app = test_app("local");
            app.filter.grouping = mode;

            let rendered = buffer_text(&render_buffer(&mut app, 120, 24));

            let expected = if mode == GroupingMode::Command {
                "no commands configured"
            } else {
                "listening for mDNS services"
            };
            assert!(rendered.contains(expected), "{mode:?}: {rendered}");
        }
    }

    #[test]
    fn tab_counts_are_rendered_from_the_apps_row_counts() {
        let mut app = browsing(GroupingMode::LogicalService, &[]);
        app.tab_counts = [4, 3, 2, 1];

        let rendered = buffer_text(&render_buffer(&mut app, 120, 24));

        assert!(rendered.contains("services 4"), "{rendered}");
        assert!(rendered.contains("hosts 3"));
        assert!(rendered.contains("types 2"));
        assert!(rendered.contains("commands 1"));
    }

    #[test]
    fn render_escapes_dynamic_command_metadata() {
        let command = CommandConfig {
            name: "inspect\u{1b}".to_string(),
            description: Some("description\nline".to_string()),
            requirements: vec![Requirement {
                command: "binary\tname".to_string(),
                optional: false,
            }],
            predicates: Vec::new(),
            action: CommandAction::compile(
                None,
                "tool\r--flag\u{85}".to_string(),
                ActionMode::Fork,
            )
            .unwrap(),
        };
        let mut app = test_app("local");
        app.filter.grouping = GroupingMode::Command;
        app.command_groups = vec![CommandGroup {
            command,
            services: Vec::new(),
        }];

        let buffer = render_buffer(&mut app, 140, 24);
        let rendered = buffer_text(&buffer);

        assert!(rendered.contains("inspect\\x1B"));
        assert!(rendered.contains("description\\x0Aline"));
        assert!(rendered.contains("binary\\x09name"));
        assert!(rendered.contains("tool\\x0D--flag\\x85"));
        assert!(
            !buffer
                .content()
                .iter()
                .flat_map(|cell| cell.symbol().chars())
                .any(char::is_control)
        );
    }

    #[test]
    fn escaped_text_is_panic_free_in_a_very_narrow_terminal() {
        let mut app = test_app("domain\u{1b}[2J-that-is-wider-than-the-screen");
        app.status = "status\u{7}\nthat-is-also-too-wide".to_string();

        let buffer = render_buffer(&mut app, 1, 4);

        assert_eq!(buffer.area.width, 1);
        assert_eq!(buffer.area.height, 4);
    }

    // ── keybinding hints ───────────────────────────────────────────────────
    /// Build an app whose bindings come from a keybindings file, so the hints
    /// under test are produced the same way a user's customization produces
    /// them.
    fn app_with_bindings(toml: &str) -> App {
        let path = temp_file("render-bindings", toml);
        let bindings = KeyBindings::load(std::slice::from_ref(&path)).unwrap();
        remove(&path);
        let mut app = test_app("local");
        app.keybindings = bindings;
        app
    }

    #[test]
    fn the_footer_shows_the_default_browse_bindings() {
        let mut app = test_app("local");

        let text = buffer_text(&render_buffer(&mut app, 160, 24));

        assert!(text.contains("↓/↑  move"), "{text}");
        assert!(text.contains("⏎  open"), "{text}");
        assert!(text.contains("/  search"), "{text}");
        assert!(text.contains("?  help"), "{text}");
        assert!(text.contains("q  quit"), "{text}");
    }

    /// The whole point of the task: after rebinding, the footer must instruct
    /// the user to press the keys that actually work.
    #[test]
    fn rebound_browse_keys_change_the_footer_hints() {
        let mut app = app_with_bindings(
            r#"
[browse]
down = ["ctrl-n"]
up = ["ctrl-p"]
invoke = ["space"]
help = ["f1"]
search = ["f"]
"#,
        );

        let text = buffer_text(&render_buffer(&mut app, 160, 24));

        assert!(text.contains("^n/^p  move"), "{text}");
        assert!(text.contains("space  open"), "{text}");
        assert!(text.contains("F1  help"), "{text}");
        assert!(text.contains("f  search"), "{text}");
        // The defaults they replaced are gone from the footer.
        assert!(!text.contains("↓/↑  move"), "{text}");
        assert!(!text.contains("⏎  open"), "{text}");
        assert!(!text.contains("?  help"), "{text}");
    }

    /// An unbound action has no key to advertise, so its hint must disappear
    /// rather than name a key that does nothing.
    #[test]
    fn an_unbound_action_has_no_footer_hint() {
        let mut app = app_with_bindings(
            r#"
[browse]
same_host = []
refresh = []
"#,
        );

        let text = buffer_text(&render_buffer(&mut app, 160, 24));

        assert!(!text.contains("same-host"), "{text}");
        assert!(!text.contains("refresh"), "{text}");
        assert!(text.contains("↓/↑  move"), "{text}");
    }

    /// Quit is guaranteed reachable, so the footer always has a quit hint —
    /// even when the browse quit key is the one that was unbound.
    #[test]
    fn the_footer_falls_back_to_the_common_quit_hint() {
        let mut app = app_with_bindings(
            r#"
[browse]
quit = []
"#,
        );

        let text = buffer_text(&render_buffer(&mut app, 160, 24));

        assert!(text.contains("^c  quit"), "{text}");
    }

    #[test]
    fn the_search_placeholder_names_the_configured_search_key() {
        let mut app = app_with_bindings(
            r#"
[browse]
search = ["f"]
"#,
        );

        let text = buffer_text(&render_buffer(&mut app, 160, 24));

        assert!(text.contains("press f to search"), "{text}");
    }

    #[test]
    fn rebound_modal_keys_change_the_popup_hints() {
        let mut app = app_with_bindings(
            r#"
[picker]
select = ["space"]
close = ["ctrl-g"]
"#,
        );
        app.mode = AppMode::ActionPicker;

        let text = buffer_text(&render_buffer(&mut app, 160, 24));

        assert!(text.contains("space runs · ^g closes"), "{text}");
    }

    #[test]
    fn the_help_overlay_lists_every_key_bound_to_an_action() {
        let mut app = test_app("local");
        app.mode = AppMode::Help;

        let text = buffer_text(&render_buffer(&mut app, 160, 40));

        // Help is the reference: it keeps every alias, not just the first.
        assert!(text.contains("↓ / j"), "{text}");
        assert!(text.contains("r / F5"), "{text}");
        assert!(text.contains("d / pgdn / ^d"), "{text}");
        assert!(text.contains("q / ^c"), "{text}");
    }

    #[test]
    fn rebound_keys_change_the_help_overlay() {
        let mut app = app_with_bindings(
            r#"
[browse]
down = ["ctrl-n", "f9"]
help = ["f1"]
"#,
        );
        app.mode = AppMode::Help;

        let text = buffer_text(&render_buffer(&mut app, 160, 40));

        assert!(text.contains("^n / F9"), "{text}");
        assert!(text.contains("F1"), "{text}");
        assert!(!text.contains("↓ / j"), "{text}");
    }

    /// Help used to claim Escape clears the search. It does not: leaving search
    /// keeps the query, and only the clear action removes it. The help text has
    /// to say what the code does.
    #[test]
    fn the_help_overlay_does_not_claim_that_closing_search_clears_it() {
        let mut app = test_app("local");
        app.mode = AppMode::Help;

        let text = buffer_text(&render_buffer(&mut app, 160, 40));

        assert!(!text.contains("clear search"), "{text}");
        assert!(text.contains("leave search, keep filter"), "{text}");
        assert!(text.contains("clear the search filter"), "{text}");
    }

    // ── modal viewports ────────────────────────────────────────────────────
    //
    // The sample backend cannot produce a picker taller than its popup, so the
    // oversized cases live here, where the list length is ours to choose.

    /// A modal so short that only a few of its rows can be on screen at once.
    const SHORT: (u16, u16) = (60, 18);

    fn command_named(name: &str) -> CommandConfig {
        CommandConfig {
            name: name.to_string(),
            description: Some(format!("run {name}")),
            requirements: Vec::new(),
            predicates: Vec::new(),
            action: CommandAction::compile(None, "tool".to_string(), ActionMode::Fork).unwrap(),
        }
    }

    /// An entry whose rendered line names `index` unmistakably.
    fn numbered_entry(index: usize) -> Entry {
        let mut record = Entry::new(format!("svc-{index:02}"), "_ssh._tcp", "local");
        record.hostname = Some(format!("host-{index:02}.local"));
        record.addresses = vec!["192.0.2.1".parse().unwrap()];
        record.port = Some(22);
        record
    }

    /// Which rows of a picker over `count` items are on screen when `selected`
    /// is the cursor, as row labels found in the rendered buffer.
    fn visible_labels(text: &str, count: usize, label: impl Fn(usize) -> String) -> Vec<usize> {
        (0..count)
            .filter(|index| text.contains(&label(*index)))
            .collect()
    }

    /// The safety core of task 011: a picker whose selected row is off-screen
    /// invites the user to run a target they cannot see. At every index of an
    /// oversized list, the selected row must be rendered.
    #[test]
    fn an_oversized_type_filter_shows_the_selected_row_at_every_index() {
        let mut app = test_app("local");
        app.mode = AppMode::TypeFilter;
        let types = discover_types(&mut app, 40, |i| format!("_type{i:02}._tcp"));

        for (index, service_type) in types.iter().enumerate() {
            app.type_filter_index = index;
            let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

            assert!(
                text.contains(service_type),
                "type {index} is selected but not on screen:\n{text}"
            );
        }
    }

    #[test]
    fn an_oversized_action_picker_shows_the_selected_row_at_every_index() {
        let mut app = test_app("local");
        app.mode = AppMode::ActionPicker;
        app.action_matches = (0..30)
            .map(|i| MatchResult {
                command: command_named(&format!("act-{i:02}")),
                targets: vec![numbered_entry(i)],
            })
            .collect();

        for index in 0..app.action_matches.len() {
            app.action_index = index;
            let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

            assert!(
                text.contains(&format!("act-{index:02}")),
                "action {index} is selected but not on screen:\n{text}"
            );
        }
    }

    #[test]
    fn an_oversized_instance_picker_shows_the_selected_row_at_every_index() {
        let mut app = test_app("local");
        app.mode = AppMode::InstancePicker;
        app.pending_action = Some(MatchResult {
            command: command_named("ssh"),
            targets: (0..30).map(numbered_entry).collect(),
        });

        for index in 0..30 {
            app.instance_index = index;
            let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

            assert!(
                text.contains(&format!("host-{index:02}.local")),
                "instance {index} is selected but not on screen:\n{text}"
            );
        }
    }

    #[test]
    fn an_oversized_service_picker_shows_the_selected_row_at_every_index() {
        let mut app = test_app("local");
        app.mode = AppMode::ServicePicker;
        app.filter.grouping = GroupingMode::Command;
        let services: Vec<EntryGroup> = (0..30)
            .map(|i| {
                browse_groups(&[numbered_entry(i)], BrowseMode::LogicalService)
                    .pop()
                    .expect("one group")
            })
            .collect();
        app.command_groups = vec![CommandGroup {
            command: command_named("ssh"),
            services,
        }];
        app.selected = 0;

        for index in 0..30 {
            app.service_picker_index = index;
            let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

            assert!(
                text.contains(&format!("svc-{index:02}")),
                "service {index} is selected but not on screen:\n{text}"
            );
        }
    }

    /// Moving within the visible window must not scroll: the list would jump
    /// under a user who is only stepping down one row.
    #[test]
    fn a_picker_scrolls_only_once_the_selection_leaves_the_window() {
        let mut app = test_app("local");
        app.mode = AppMode::TypeFilter;
        discover_types(&mut app, 40, |i| format!("_type{i:02}._tcp"));
        let label = |index: usize| format!("_type{index:02}._tcp");

        app.type_filter_index = 0;
        let top = visible_labels(
            &buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1)),
            40,
            label,
        );
        // Stepping onto the last visible row shows exactly the same rows.
        app.type_filter_index = *top.last().expect("a visible row");
        let unmoved = visible_labels(
            &buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1)),
            40,
            label,
        );
        assert_eq!(top, unmoved);

        // One row further scrolls by exactly one row.
        app.type_filter_index += 1;
        let scrolled = visible_labels(
            &buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1)),
            40,
            label,
        );
        assert_eq!(scrolled.first(), Some(&1));
        assert_eq!(scrolled.last(), Some(&app.type_filter_index));
    }

    /// The type chip counts what is on screen now.
    ///
    /// The regression: a type the user never switched off stayed in the enabled
    /// set forever, so after the last SSH service went away the chip still read
    /// `1/1` — "one of one type shown" — over a list showing nothing at all.
    #[test]
    fn the_type_chip_counts_only_types_that_are_still_discovered() {
        let ssh = Entry::new("shell", "_ssh._tcp", "local");
        let http = Entry::new("site", "_http._tcp", "local");
        let mut app = test_app("local");

        // Both types discovered, HTTP switched off: one of two shown.
        app.filter.observe_types(&[ssh, http.clone()]);
        app.filter.toggle_service_type("_http._tcp");
        assert!(buffer_text(&render_buffer(&mut app, 100, 24)).contains("types 1/2"));

        // SSH goes away. Only HTTP is left, and it is switched off: nothing is
        // shown, of the one type there is.
        app.filter.observe_types(&[http]);
        let text = buffer_text(&render_buffer(&mut app, 100, 24));

        assert!(text.contains("types 0/1"), "{text}");
        assert!(!text.contains("types 1/1"), "{text}");
    }

    /// Shrinking the terminal until the selection would fall outside the popup
    /// must bring it back into view, not leave it behind.
    #[test]
    fn shrinking_the_terminal_keeps_the_picker_selection_visible() {
        let mut app = test_app("local");
        app.mode = AppMode::TypeFilter;
        discover_types(&mut app, 40, |i| format!("_type{i:02}._tcp"));
        app.type_filter_index = 25;

        for height in 6..40u16 {
            let text = buffer_text(&render_buffer(&mut app, 60, height));

            assert!(
                text.contains("_type25._tcp"),
                "selection lost at height {height}:\n{text}"
            );
        }
    }

    /// A picker that is showing all of its items has nothing to say about
    /// position, and a range chip there would be noise.
    #[test]
    fn a_picker_shows_a_range_chip_only_when_it_is_clipped() {
        let mut app = test_app("local");
        app.mode = AppMode::TypeFilter;

        discover_types(&mut app, 3, |i| format!("_type{i:02}._tcp"));
        let fits = buffer_text(&render_buffer(&mut app, 60, 24));
        assert!(!fits.contains("1-3/3"), "{fits}");

        discover_types(&mut app, 40, |i| format!("_type{i:02}._tcp"));
        app.type_filter_index = 39;
        let clipped = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));
        assert!(clipped.contains("-40/40"), "{clipped}");
    }

    /// Wide glyphs are the list's business, not the window's: the selected row
    /// is still the one on screen.
    #[test]
    fn a_picker_with_unicode_labels_keeps_its_selection_visible() {
        let mut app = test_app("local");
        app.mode = AppMode::TypeFilter;
        discover_types(&mut app, 30, |i| format!("_型{i:02}界._tcp"));
        app.type_filter_index = 29;

        let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

        // A wide glyph owns two cells, the second of which reads back blank, so
        // the label is matched by the part that cannot be split.
        assert!(text.contains("29界"), "{text}");
        assert!(text.contains("-30/30"), "{text}");
    }

    /// An empty picker and a popup with no room for a single row are both
    /// arithmetic edge cases in the window; neither may panic.
    #[test]
    fn empty_and_tiny_modals_render_safely() {
        for mode in [
            AppMode::TypeFilter,
            AppMode::ActionPicker,
            AppMode::InstancePicker,
            AppMode::ServicePicker,
            AppMode::Help,
        ] {
            let mut app = test_app("local");
            app.mode = mode;

            for (width, height) in [(60u16, 18u16), (20, 5), (10, 3), (4, 2), (1, 1)] {
                let buffer = render_buffer(&mut app, width, height);
                assert_eq!(buffer.area.height, height, "{mode:?} at {width}x{height}");
            }
        }
    }

    /// The dangerous half of the same edge: a popup squeezed below one content
    /// row while the list behind it is long and the cursor is at the far end.
    /// The window, the range chip, and the scrollbar all do arithmetic on a
    /// viewport of zero here.
    #[test]
    fn a_long_picker_in_a_terminal_with_no_room_renders_safely() {
        let mut app = test_app("local");
        app.mode = AppMode::TypeFilter;
        discover_types(&mut app, 40, |i| format!("_type{i:02}._tcp"));
        app.type_filter_index = 39;
        app.help_scroll = 999;

        for (width, height) in [(60u16, 6u16), (20, 4), (10, 3), (4, 2), (2, 1), (1, 1)] {
            let buffer = render_buffer(&mut app, width, height);
            assert_eq!(buffer.area.height, height, "at {width}x{height}");
        }

        app.mode = AppMode::Help;
        for (width, height) in [(60u16, 6u16), (10, 3), (1, 1)] {
            let buffer = render_buffer(&mut app, width, height);
            assert_eq!(buffer.area.height, height, "help at {width}x{height}");
        }
    }

    // ── help viewport ──────────────────────────────────────────────────────

    /// The defect reproduced by the midpoint review: at 60×18 the tail of the
    /// help — including the badges legend — was rendered into a popup too short
    /// to show it, and no key could reveal it. Every generated row must now be
    /// reachable by scrolling.
    #[test]
    fn every_help_row_is_reachable_on_a_short_terminal() {
        let mut app = test_app("local");
        app.mode = AppMode::Help;
        let expected: Vec<String> = help_rows(&app)
            .iter()
            .map(|(_, label)| label.to_string())
            .chain(["badges:".to_string()])
            .collect();

        let mut seen = std::collections::BTreeSet::new();
        let max = Window::max_scroll(
            help_lines(&app).len(),
            help_viewport(Rect::new(0, 0, 60, 18)),
        );
        for scroll in 0..=max {
            app.help_scroll = scroll;
            let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));
            for row in &expected {
                // The popup is narrow, so a long label is truncated; its head is
                // what proves the row was drawn.
                let head: String = row.chars().take(18).collect();
                if text.contains(&head) {
                    seen.insert(row.clone());
                }
            }
        }

        for row in &expected {
            assert!(seen.contains(row), "help row `{row}` is never reachable");
        }
    }

    #[test]
    fn the_help_overlay_reports_its_range_and_scrolls_to_the_end() {
        let mut app = test_app("local");
        app.mode = AppMode::Help;
        let total = help_lines(&app).len();

        let top = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));
        assert!(top.contains(&format!("/{total}")), "{top}");

        app.help_scroll = Window::max_scroll(total, help_viewport(Rect::new(0, 0, 60, 18)));
        let bottom = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));
        assert!(bottom.contains(&format!("-{total}/{total}")), "{bottom}");
        assert!(bottom.contains("badges:"), "{bottom}");
    }

    /// Help that fits needs neither a range chip nor a scroll hint: there is
    /// nowhere to scroll to, and saying otherwise would advertise a key that
    /// appears to do nothing.
    #[test]
    fn help_that_fits_advertises_no_scrolling() {
        let mut app = test_app("local");
        app.mode = AppMode::Help;

        let text = buffer_text(&render_buffer(&mut app, 160, 44));

        assert!(text.contains("esc closes"), "{text}");
        assert!(!text.contains("scrolls"), "{text}");
    }

    /// Scrolling to the bottom and then growing the terminal must not leave the
    /// content stranded above a band of blank rows.
    #[test]
    fn help_scrolled_to_the_end_reflows_when_the_terminal_grows() {
        let mut app = test_app("local");
        app.mode = AppMode::Help;
        let total = help_lines(&app).len();
        app.help_scroll = Window::max_scroll(total, help_viewport(Rect::new(0, 0, 60, 18)));

        let grown = buffer_text(&render_buffer(&mut app, 160, 44));

        // The taller popup shows everything, so the window is back at the top.
        assert!(grown.contains("move selection down"), "{grown}");
        assert!(grown.contains("badges:"), "{grown}");
    }

    #[test]
    fn the_help_scroll_hint_follows_the_configured_bindings() {
        let mut app = app_with_bindings(
            r#"
[help]
down = ["ctrl-n"]
up = ["ctrl-p"]
"#,
        );
        app.mode = AppMode::Help;

        let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

        assert!(text.contains("^n/^p scrolls"), "{text}");
        assert!(!text.contains("↓/↑ scrolls"), "{text}");
    }

    /// An unbound scroll action has no key to name, so the hint must go rather
    /// than promise a way to reach the rest of the content.
    #[test]
    fn unbound_help_scrolling_has_no_hint() {
        let mut app = app_with_bindings(
            r#"
[help]
down = []
up = []
"#,
        );
        app.mode = AppMode::Help;

        let text = buffer_text(&render_buffer(&mut app, SHORT.0, SHORT.1));

        assert!(!text.contains("scrolls"), "{text}");
        assert!(text.contains("esc closes"), "{text}");
    }

    /// Every hint the footer offers must name a key the keymap resolves back to
    /// the action the hint describes.
    #[test]
    fn every_browse_footer_hint_names_a_key_that_still_works() {
        let app = test_app("local");

        for (key, label) in browse_footer_hints(&app) {
            assert!(!key.is_empty(), "`{label}` hint has no key");
        }
        assert_eq!(footer_hints(&app, AppMode::Help).len(), 1);
    }
}
