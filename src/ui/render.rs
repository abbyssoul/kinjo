use std::collections::BTreeSet;

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
    discovery::{Entry, EntryGroup, GroupingMode},
    plumber::MatchResult,
};

use super::app::{App, AppMode, CommandGroup};

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

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top stats bar
            Constraint::Length(1), // filter / search bar
            Constraint::Min(6),    // body
            Constraint::Length(1), // footer hints
        ])
        .split(area);

    render_top_bar(frame, app, chunks[0]);
    render_filter_bar(frame, app, chunks[1]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(chunks[2]);
    if app.filter.grouping == GroupingMode::Command {
        render_commands(frame, app, body[0]);
        render_command_details(frame, app, body[1]);
    } else {
        render_services(frame, app, body[0]);
        render_details(frame, app, body[1]);
    }

    render_footer(frame, app, chunks[3]);

    match app.mode {
        AppMode::TypeFilter => render_type_filter(frame, app),
        AppMode::ActionPicker => render_action_picker(frame, app),
        AppMode::InstancePicker => render_instance_picker(frame, app),
        AppMode::ServicePicker => render_service_picker(frame, app),
        AppMode::Help => render_help(frame),
        AppMode::Browse | AppMode::Search => {}
    }
}

// ── top bar (view tabs) ────────────────────────────────────────────────────
fn render_top_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let spinner = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
    let hosts = app
        .records
        .values()
        .filter_map(|r| r.hostname.clone())
        .collect::<BTreeSet<_>>()
        .len();

    // Per-tab count shown alongside the title so the bar still surfaces the
    // discovery totals it used to.
    let tab_count = |mode: GroupingMode| match mode {
        GroupingMode::LogicalService => app.records.len(),
        GroupingMode::Host => hosts,
        GroupingMode::ServiceType => app.service_types().len(),
        GroupingMode::Command => app.matcher.command_count(),
    };

    let mut spans = vec![
        Span::styled(format!(" {spinner} "), Style::default().fg(GOOD)),
        Span::styled(
            "avahi-tui  ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
    ];

    for mode in GroupingMode::TABS {
        let active = mode == app.filter.grouping;
        let text = format!(" {} {} ", mode.tab_title(), tab_count(mode));
        let style = if active {
            Style::default()
                .fg(BG_BAR)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG_DIM)
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }

    // right-aligned domain chip
    let domain = Span::styled(
        format!("  {}  ", app.cli.domain),
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
        spans.push(Span::styled(
            "fuzzy filter — press / to search",
            Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
        ));
    } else {
        spans.push(Span::styled(
            app.filter.text_query.clone(),
            Style::default().fg(Color::White),
        ));
        if searching && (app.ticks / 4).is_multiple_of(2) {
            spans.push(Span::styled("▌", Style::default().fg(ACCENT)));
        }
    }

    // right-side chips: type filter + host filter
    let mut chips: Vec<Span> = Vec::new();
    let total_types = app.service_types().len();
    let enabled = app.filter.enabled_service_types.len().min(total_types);
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
        text.to_string(),
        Style::default()
            .fg(BG_BAR)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

// ── service list ─────────────────────────────────────────────────────────
fn render_services(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let total = app.visible_groups.len();

    let spinner = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
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
        vec![
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "  {spinner} listening for mDNS services on {}…",
                    app.cli.domain
                ),
                Style::default().fg(FG_DIM),
            )),
        ]
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

    render_list_panel(
        frame,
        area,
        ListPanelSpec {
            label: " services ",
            selected: app.selected,
            total,
            widths: &widths,
            empty,
        },
        |index| {
            service_row(
                &app.visible_groups[index],
                index == app.selected,
                app.group_match_counts.get(index).copied().unwrap_or(0),
            )
        },
    );
}

fn service_row(group: &EntryGroup, selected: bool, matches: usize) -> Row<'static> {
    let color = category_color(&group.service_type);
    let base = selection_style(selected);
    let gutter = gutter_span(selected);

    let name_style = if selected {
        base.fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        base.fg(Color::White)
    };

    // instance count badge
    let n = group.instances.len();
    let count = if n > 1 {
        Span::styled(
            format!("×{n}"),
            base.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("", base)
    };

    // matching-commands badge
    let matches_cell = if matches > 0 {
        Span::styled(
            format!("★{matches}"),
            base.fg(STAR).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("·", base.fg(ACCENT_DIM))
    };

    let host = group.hostname.as_deref().unwrap_or("…resolving");

    Row::new(vec![
        Cell::from(Line::from(vec![gutter, Span::styled("●", base.fg(color))])),
        Cell::from(Span::styled(group.label.clone(), name_style)),
        Cell::from(Span::styled(
            short_type(&group.service_type),
            base.fg(color),
        )),
        Cell::from(count),
        Cell::from(matches_cell),
        Cell::from(Span::styled(host.to_string(), base.fg(FG_DIM))),
    ])
    .style(base)
}

// ── details / preview ────────────────────────────────────────────────────
fn render_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = panel().title(Line::from(Span::styled(
        " details ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));

    let Some(group) = app.visible_groups.get(app.selected) else {
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

    let color = category_color(&group.service_type);
    // header — name spans the value column, dot sits in the label column
    let mut rows: Vec<Row> = vec![Row::new(vec![
        Cell::from(Span::styled(" ●", Style::default().fg(color))),
        Cell::from(Span::styled(
            group.label.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
    ])];
    rows.push(field_row("type", &group.service_type, color));
    rows.push(field_row("domain", &group.domain, FG_DIM));
    rows.push(field_row(
        "host",
        group.hostname.as_deref().unwrap_or("…resolving"),
        FG_DIM,
    ));
    rows.push(field_row(
        "port",
        &group
            .port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "…".to_string()),
        FG_DIM,
    ));

    if !group.txt.is_empty() {
        rows.push(blank_row());
        rows.push(section_row("TXT records"));
        for (key, value) in group.txt.iter() {
            rows.push(Row::new(vec![
                Cell::from(Span::styled(format!(" {key}"), Style::default().fg(WARN))),
                Cell::from(Line::from(vec![
                    Span::styled("= ", Style::default().fg(ACCENT_DIM)),
                    Span::styled(value.clone(), Style::default().fg(Color::White)),
                ])),
            ]));
        }
    }

    // instance tree
    rows.push(blank_row());
    rows.push(section_row(&format!(
        "instances ({})",
        group.instances.len()
    )));
    let last = group.instances.len().saturating_sub(1);
    for (i, record) in group.instances.iter().enumerate() {
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
                Span::styled(instance_endpoint(record), Style::default().fg(Color::White)),
                Span::styled(
                    format!("  {}s", record.last_seen.elapsed().as_secs()),
                    Style::default().fg(FG_DIM),
                ),
            ])),
        ]));
    }

    // matching actions
    rows.push(blank_row());
    let actions = app.matcher.matches_group(group);
    rows.push(section_row(&format!("actions ({})", actions.len())));
    if actions.is_empty() {
        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled(
                "no configured commands match this service",
                Style::default().fg(FG_DIM).add_modifier(Modifier::ITALIC),
            )),
        ]));
    } else {
        for action in &actions {
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

    render_detail_rows(frame, app, area, block, rows);
}

fn field_row(label: &str, value: &str, value_color: Color) -> Row<'static> {
    Row::new(vec![
        Cell::from(Span::styled(
            format!("{label:>7} "),
            Style::default().fg(FG_DIM),
        )),
        Cell::from(Span::styled(
            value.to_string(),
            Style::default().fg(value_color),
        )),
    ])
}

fn section_row(title: &str) -> Row<'static> {
    Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled(
            title.to_string(),
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
        action.command.name.clone(),
        Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
    )];
    if !description.is_empty() {
        spans.push(Span::styled(
            format!(" — {description}"),
            Style::default().fg(Color::White),
        ));
    }
    spans.push(Span::styled(
        format!("  [{mode}]"),
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
            format!("★{count} svc"),
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
        Cell::from(Span::styled(group.command.name.clone(), name_style)),
        Cell::from(count_cell),
        Cell::from(Span::styled(description.to_string(), base.fg(FG_DIM))),
    ])
    .style(base)
}

fn render_command_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = panel().title(Line::from(Span::styled(
        " command ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));

    let Some(group) = app.command_groups.get(app.selected) else {
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

    let command = &group.command;
    let mut rows: Vec<Row> = vec![Row::new(vec![
        Cell::from(Span::styled(" ★", Style::default().fg(STAR))),
        Cell::from(Span::styled(
            command.name.clone(),
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
        rows.push(field_row("needs", &command.requirements.join(", "), WARN));
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
            let host = service.hostname.as_deref().unwrap_or("…resolving");
            rows.push(Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(format!(" {branch} "), Style::default().fg(ACCENT_DIM)),
                    Span::styled(
                        "●",
                        Style::default().fg(category_color(&service.service_type)),
                    ),
                ])),
                Cell::from(Line::from(vec![
                    Span::styled(service.label.clone(), Style::default().fg(Color::White)),
                    Span::styled(format!("  {host}"), Style::default().fg(FG_DIM)),
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

    render_detail_rows(frame, app, area, block, rows);
}

// ── footer ───────────────────────────────────────────────────────────────
fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let hints: &[(&str, &str)] = match app.mode {
        AppMode::Search => &[("type", "filter"), ("⏎/esc", "done"), ("^U", "clear")],
        AppMode::TypeFilter => &[("jk", "move"), ("space", "toggle"), ("esc", "close")],
        AppMode::ActionPicker | AppMode::InstancePicker | AppMode::ServicePicker => {
            &[("jk", "move"), ("⏎", "run"), ("esc", "cancel")]
        }
        AppMode::Help => &[("esc", "close")],
        AppMode::Browse => &[
            ("jk", "move"),
            ("⏎", "open"),
            ("/", "search"),
            ("t", "types"),
            ("⇥", "view"),
            ("s", "same-host"),
            ("u/d", "scroll"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };

    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints {
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
        format!("  {} ", app.status),
        Style::default().fg(ACCENT_DIM),
    );
    push_right_aligned(&mut spans, vec![status], area.width);

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(BG_BAR)),
        area,
    );
}

// ── modals ───────────────────────────────────────────────────────────────
fn render_type_filter(frame: &mut Frame<'_>, app: &App) {
    let service_types = app.service_types();
    let items: Vec<ListItem> = if service_types.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no service types discovered yet",
            Style::default().fg(FG_DIM),
        )))]
    } else {
        build_list_items(
            &service_types,
            app.type_filter_index,
            |service_type, selected, base| {
                let enabled = app.filter.enabled_service_types.contains(service_type);
                let check = if enabled {
                    Span::styled(" ✓ ", base.fg(GOOD).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled(" ○ ", base.fg(FG_DIM))
                };
                Line::from(vec![
                    gutter_span(selected),
                    check,
                    Span::styled(" ● ", base.fg(category_color(service_type))),
                    Span::styled(service_type.clone(), base.fg(Color::White)),
                ])
            },
        )
    };
    render_popup(
        frame,
        " service types ",
        "space toggles · esc closes",
        List::new(items),
        58,
        60,
    );
}

fn render_action_picker(frame: &mut Frame<'_>, app: &App) {
    let items = build_list_items(
        &app.action_matches,
        app.action_index,
        |action, selected, base| {
            let needs = action.needs_instance && action.matching_records.len() > 1;
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
                    action.command.name.clone(),
                    base.fg(GOOD).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" — {description}"), base.fg(Color::White)),
            ];
            if needs {
                spans.push(Span::styled("  ⊙ choose instance", base.fg(WARN)));
            }
            Line::from(spans)
        },
    );
    render_popup(
        frame,
        " matching actions ",
        "⏎ runs · esc closes",
        List::new(items),
        70,
        50,
    );
}

fn render_instance_picker(frame: &mut Frame<'_>, app: &App) {
    let records = app
        .pending_action
        .as_ref()
        .map(|action| action.matching_records.as_slice())
        .unwrap_or(&[]);
    let items = build_list_items(records, app.instance_index, |record, selected, base| {
        Line::from(vec![
            gutter_span(selected),
            Span::styled("● ", base.fg(category_color(&record.service_type))),
            Span::styled(instance_endpoint(record), base.fg(Color::White)),
        ])
    });
    render_popup(
        frame,
        " select instance ",
        "⏎ runs · esc closes",
        List::new(items),
        72,
        55,
    );
}

fn render_service_picker(frame: &mut Frame<'_>, app: &App) {
    let group = app.command_groups.get(app.selected);
    let command_name = group.map(|g| g.command.name.clone()).unwrap_or_default();
    let services = group.map(|g| g.services.as_slice()).unwrap_or(&[]);
    let items = build_list_items(
        services,
        app.service_picker_index,
        |service, selected, base| {
            let host = service.hostname.as_deref().unwrap_or("…resolving");
            Line::from(vec![
                gutter_span(selected),
                Span::styled("● ", base.fg(category_color(&service.service_type))),
                Span::styled(service.label.clone(), base.fg(Color::White)),
                Span::styled(format!("  {host}"), base.fg(FG_DIM)),
            ])
        },
    );

    render_popup(
        frame,
        &format!(" run {command_name} on "),
        "⏎ runs · esc closes",
        List::new(items),
        72,
        55,
    );
}

fn render_help(frame: &mut Frame<'_>) {
    let rows = [
        ("j / ↓", "move selection down"),
        ("k / ↑", "move selection up"),
        ("d / ^d", "details down ½ page"),
        ("u / ^u", "details up ½ page"),
        ("enter", "run matching action(s)"),
        ("/", "fuzzy text filter"),
        ("t", "service-type checklist"),
        ("⇥ / ←→", "switch view tab (services/hosts/types/commands)"),
        ("s", "filter to selected host"),
        ("esc", "close modal / clear search"),
        ("?", "toggle this help"),
        ("q", "quit"),
    ];
    let mut lines = vec![Line::from("")];
    for (key, label) in rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {key:<8}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(label.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "   badges:  ×N instances   ★N matching commands",
        Style::default().fg(FG_DIM),
    )));
    render_popup(
        frame,
        " help ",
        "esc closes",
        Paragraph::new(lines).alignment(Alignment::Left),
        58,
        70,
    );
}

fn render_popup<W>(
    frame: &mut Frame<'_>,
    title: &str,
    hint: &str,
    widget: W,
    width: u16,
    height: u16,
) where
    W: ratatui::widgets::Widget,
{
    let area = centered_rect(width, height, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            format!(" {hint} "),
            Style::default().fg(FG_DIM),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(widget, inner);
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

fn scroll_offset(selected: usize, total: usize, view_h: usize) -> usize {
    if view_h == 0 || total <= view_h {
        return 0;
    }
    if selected < view_h {
        0
    } else {
        (selected + 1 - view_h).min(total - view_h)
    }
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
    let len = |list: &[Span]| -> usize { list.iter().map(|s| s.content.chars().count()).sum() };
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
        label.to_string(),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if total > 0 {
        let first = scroll_offset(selected, total, inner_h) + 1;
        let last = (first + inner_h - 1).min(total);
        spans.push(Span::styled(
            format!("{first}-{last}/{total} "),
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

/// Render the shared tail of the two detail panes: clamp the requested scroll to
/// the content height (writing the bounds back for the key handler), slice the
/// visible window, and draw the table + scrollbar.
fn render_detail_rows<'a>(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    block: Block<'a>,
    rows: Vec<Row<'a>>,
) {
    let total = rows.len();
    let inner_h = area.height.saturating_sub(2) as usize;
    let max_scroll = total.saturating_sub(inner_h);
    app.details_max_scroll.set(max_scroll);
    app.details_viewport.set(inner_h);
    let offset = app.details_scroll.min(max_scroll);
    let visible: Vec<Row> = rows.into_iter().skip(offset).take(inner_h).collect();

    let widths = [Constraint::Length(8), Constraint::Fill(1)];
    frame.render_widget(
        Table::new(visible, widths).column_spacing(1).block(block),
        area,
    );
    render_scrollbar(frame, area, total, inner_h, offset);
}

/// Build modal list items: compute the per-item `selected`/`base` style, let
/// `line` produce the row content, and apply the selection background.
fn build_list_items<T>(
    items: &[T],
    selected_index: usize,
    mut line: impl FnMut(&T, bool, Style) -> Line<'static>,
) -> Vec<ListItem<'static>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let selected = index == selected_index;
            let base = selection_style(selected);
            ListItem::new(line(item, selected, base).style(base))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
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

        let width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(width, 20);
        assert!(line_text(&Line::from(spans)).starts_with("left"));
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
}
