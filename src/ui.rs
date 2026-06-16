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
    app::{App, AppMode, CommandGroup},
    plumber::MatchResult,
    service::{GroupingMode, ServiceGroup, ServiceRecord},
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
        AppMode::Grouping => render_grouping(frame, app),
        AppMode::ActionPicker => render_action_picker(frame, app),
        AppMode::InstancePicker => render_instance_picker(frame, app),
        AppMode::ServicePicker => render_service_picker(frame, app),
        AppMode::Help => render_help(frame),
        AppMode::Browse | AppMode::Search => {}
    }
}

// ── top bar ──────────────────────────────────────────────────────────────
fn render_top_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let spinner = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
    let hosts = app
        .records
        .values()
        .filter_map(|r| r.hostname.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let types = app.service_types().len();

    let sep = || Span::styled("  •  ", Style::default().fg(ACCENT_DIM));
    let num = |value: usize| {
        Span::styled(
            format!("{value}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
    };
    let label = |text: &str| Span::styled(format!(" {text}"), Style::default().fg(FG_DIM));

    let mut spans = vec![
        Span::styled(format!(" {spinner} "), Style::default().fg(GOOD)),
        Span::styled(
            "avahi-tui",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        sep(),
        num(app.records.len()),
        label("services   "),
        num(hosts),
        label("hosts   "),
        num(types),
        label("types"),
        sep(),
        num(app.matcher.command_count()),
        label("commands"),
    ];

    // right-aligned domain chip
    let left_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let right = format!("  {}  ", app.cli.domain);
    let pad = (area.width as usize)
        .saturating_sub(left_text.chars().count())
        .saturating_sub(right.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(
        right,
        Style::default()
            .fg(BG_BAR)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    ));

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
    chips.push(Span::raw(" "));
    chips.push(chip(&format!(" group:{} ", app.filter.grouping), ACCENT));

    let left_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let right_text: String = chips.iter().map(|s| s.content.as_ref()).collect();
    let pad = (area.width as usize)
        .saturating_sub(left_text.chars().count())
        .saturating_sub(right_text.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(chips);

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
    let inner_h = area.height.saturating_sub(2) as usize;

    let title = if total == 0 {
        Line::from(vec![Span::styled(
            " services ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )])
    } else {
        let first = scroll_offset(app.selected, total, inner_h) + 1;
        let last = (first + inner_h - 1).min(total);
        Line::from(vec![
            Span::styled(
                " services ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{first}-{last}/{total} "),
                Style::default().fg(FG_DIM),
            ),
        ])
    };

    let block = panel().title(title);

    if total == 0 {
        let spinner = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
        let msg = if app.filter.is_active() {
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
        frame.render_widget(Paragraph::new(msg).block(block), area);
        return;
    }

    let offset = scroll_offset(app.selected, total, inner_h);
    let rows: Vec<Row> = app
        .visible_groups
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner_h)
        .map(|(index, group)| {
            let selected = index == app.selected;
            let matches = app.group_match_counts.get(index).copied().unwrap_or(0);
            service_row(group, selected, matches)
        })
        .collect();

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

    frame.render_widget(
        Table::new(rows, widths).column_spacing(1).block(block),
        area,
    );
    render_scrollbar(frame, area, total, inner_h, offset);
}

fn service_row(group: &ServiceGroup, selected: bool, matches: usize) -> Row<'static> {
    let color = category_color(&group.service_type);
    let base = if selected {
        Style::default().bg(BG_SEL)
    } else {
        Style::default()
    };

    let gutter = if selected {
        Span::styled("▌", Style::default().fg(ACCENT).bg(BG_SEL))
    } else {
        Span::styled(" ", base)
    };

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
        Cell::from(Span::styled(short_type(&group.service_type), base.fg(color))),
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
    rows.push(section_row(&format!("instances ({})", group.instances.len())));
    let last = group.instances.len().saturating_sub(1);
    for (i, record) in group.instances.iter().enumerate() {
        let branch = if i == last { "└─" } else { "├─" };
        rows.push(Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(format!(" {branch} "), Style::default().fg(ACCENT_DIM)),
                Span::styled("●", Style::default().fg(category_color(&record.service_type))),
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

    // Clamp the requested scroll to what the content actually needs, then slice
    // out the visible window. `details_max_scroll` is written back so the key
    // handler can clamp the next scroll request against the real content height.
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

fn field_row(label: &str, value: &str, value_color: Color) -> Row<'static> {
    Row::new(vec![
        Cell::from(Span::styled(
            format!("{label:>7} "),
            Style::default().fg(FG_DIM),
        )),
        Cell::from(Span::styled(value.to_string(), Style::default().fg(value_color))),
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
    let inner_h = area.height.saturating_sub(2) as usize;

    let title = if total == 0 {
        Line::from(vec![Span::styled(
            " commands ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )])
    } else {
        let first = scroll_offset(app.selected, total, inner_h) + 1;
        let last = (first + inner_h - 1).min(total);
        Line::from(vec![
            Span::styled(
                " commands ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{first}-{last}/{total} "),
                Style::default().fg(FG_DIM),
            ),
        ])
    };

    let block = panel().title(title);

    if total == 0 {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  no commands configured",
                    Style::default().fg(FG_DIM),
                )),
            ])
            .block(block),
            area,
        );
        return;
    }

    let offset = scroll_offset(app.selected, total, inner_h);
    let rows: Vec<Row> = app
        .command_groups
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner_h)
        .map(|(index, group)| command_row(group, index == app.selected))
        .collect();

    let widths = [
        Constraint::Length(2),  // selection gutter + dot
        Constraint::Fill(5),    // command name
        Constraint::Length(11), // matching-services badge
        Constraint::Fill(6),    // description
    ];
    frame.render_widget(
        Table::new(rows, widths).column_spacing(1).block(block),
        area,
    );
    render_scrollbar(frame, area, total, inner_h, offset);
}

fn command_row(group: &CommandGroup, selected: bool) -> Row<'static> {
    let base = if selected {
        Style::default().bg(BG_SEL)
    } else {
        Style::default()
    };
    let count = group.services.len();
    let active = count > 0;

    let gutter = if selected {
        Span::styled("▌", Style::default().fg(ACCENT).bg(BG_SEL))
    } else {
        Span::styled(" ", base)
    };

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
                    Span::styled("●", Style::default().fg(category_color(&service.service_type))),
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

// ── footer ───────────────────────────────────────────────────────────────
fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let hints: &[(&str, &str)] = match app.mode {
        AppMode::Search => &[("type", "filter"), ("⏎/esc", "done"), ("^U", "clear")],
        AppMode::TypeFilter => &[("jk", "move"), ("space", "toggle"), ("esc", "close")],
        AppMode::Grouping => &[("jk", "move"), ("⏎", "select"), ("esc", "close")],
        AppMode::ActionPicker | AppMode::InstancePicker | AppMode::ServicePicker => {
            &[("jk", "move"), ("⏎", "run"), ("esc", "cancel")]
        }
        AppMode::Help => &[("esc", "close")],
        AppMode::Browse => &[
            ("jk", "move"),
            ("⏎", "open"),
            ("/", "search"),
            ("t", "types"),
            ("g", "group"),
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
    let status = format!("  {} ", app.status);
    let left_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let pad = (area.width as usize)
        .saturating_sub(left_text.chars().count())
        .saturating_sub(status.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(status, Style::default().fg(ACCENT_DIM)));

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
        service_types
            .iter()
            .enumerate()
            .map(|(index, service_type)| {
                let enabled = app.filter.enabled_service_types.contains(service_type);
                let selected = index == app.type_filter_index;
                let base = if selected {
                    Style::default().bg(BG_SEL)
                } else {
                    Style::default()
                };
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
                .style(base)
                .into()
            })
            .collect()
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

fn render_grouping(frame: &mut Frame<'_>, app: &App) {
    let items: Vec<ListItem> = GroupingMode::ALL
        .iter()
        .enumerate()
        .map(|(index, mode)| {
            let selected = index == app.grouping_index;
            let active = *mode == app.filter.grouping;
            let base = if selected {
                Style::default().bg(BG_SEL)
            } else {
                Style::default()
            };
            let marker = if active {
                Span::styled(" ● ", base.fg(GOOD))
            } else {
                Span::styled(" ○ ", base.fg(FG_DIM))
            };
            Line::from(vec![
                gutter_span(selected),
                marker,
                Span::styled(
                    mode.to_string(),
                    base.fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ])
            .style(base)
            .into()
        })
        .collect();
    render_popup(
        frame,
        " group by ",
        "⏎ selects · esc closes",
        List::new(items),
        46,
        40,
    );
}

fn render_action_picker(frame: &mut Frame<'_>, app: &App) {
    let items: Vec<ListItem> = app
        .action_matches
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let selected = index == app.action_index;
            let base = if selected {
                Style::default().bg(BG_SEL)
            } else {
                Style::default()
            };
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
            Line::from(spans).style(base).into()
        })
        .collect();
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
    let items: Vec<ListItem> = app
        .pending_action
        .as_ref()
        .map(|action| {
            action
                .matching_records
                .iter()
                .enumerate()
                .map(|(index, record)| {
                    let selected = index == app.instance_index;
                    let base = if selected {
                        Style::default().bg(BG_SEL)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        gutter_span(selected),
                        Span::styled("● ", base.fg(category_color(&record.service_type))),
                        Span::styled(instance_endpoint(record), base.fg(Color::White)),
                    ])
                    .style(base)
                    .into()
                })
                .collect()
        })
        .unwrap_or_default();
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
    let (command_name, items): (String, Vec<ListItem>) = app
        .command_groups
        .get(app.selected)
        .map(|group| {
            let items = group
                .services
                .iter()
                .enumerate()
                .map(|(index, service)| {
                    let selected = index == app.service_picker_index;
                    let base = if selected {
                        Style::default().bg(BG_SEL)
                    } else {
                        Style::default()
                    };
                    let host = service.hostname.as_deref().unwrap_or("…resolving");
                    Line::from(vec![
                        gutter_span(selected),
                        Span::styled("● ", base.fg(category_color(&service.service_type))),
                        Span::styled(service.label.clone(), base.fg(Color::White)),
                        Span::styled(format!("  {host}"), base.fg(FG_DIM)),
                    ])
                    .style(base)
                    .into()
                })
                .collect();
            (group.command.name.clone(), items)
        })
        .unwrap_or_else(|| (String::new(), Vec::new()));

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
        ("g", "change grouping (incl. by command)"),
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

fn instance_endpoint(record: &ServiceRecord) -> String {
    let host = record.hostname.as_deref().unwrap_or("…resolving");
    let addr = record
        .address
        .map(|a| a.to_string())
        .unwrap_or_else(|| "…".to_string());
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
