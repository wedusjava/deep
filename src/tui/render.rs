use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use super::app::{
    ActivityItem, AgentStatus, App, CredentialItem, FormKind, InvestigationPhase, Screen,
};

pub(super) fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(frame.area());

    match app.screen {
        Screen::Home => home(frame, app, chunks[0]),
        Screen::Credentials => credentials(frame, app, chunks[0]),
        Screen::CredentialForm => credential_form(frame, app, chunks[0]),
        Screen::Objective => objective(frame, app, chunks[0]),
        Screen::Running => running(frame, app, chunks[0]),
        Screen::Report => report(frame, app, chunks[0]),
    }
    footer(frame, app, chunks[1]);
}

fn home(frame: &mut Frame, app: &App, area: Rect) {
    let llm = app
        .credentials
        .active_llm()
        .map(|profile| format!("{} · {}", profile.name, profile.model))
        .unwrap_or_else(|| "not configured".into());
    let firecrawl = app
        .credentials
        .active_firecrawl()
        .map(|profile| profile.name)
        .unwrap_or_else(|| "not configured".into());
    let pulse = pulse(app.animation_tick);

    let body = vec![
        Line::from(vec![
            Span::styled(
                "DEEP",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {pulse}"), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(Span::styled(
            "Public-source OSINT agent swarm console",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("LLM         ", Style::default().fg(Color::DarkGray)),
            Span::styled(llm, Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("FIRECRAWL   ", Style::default().fg(Color::DarkGray)),
            Span::styled(firecrawl, Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("n", Style::default().fg(Color::Cyan)),
            Span::raw(" new OSINT case    "),
            Span::styled("c", Style::default().fg(Color::Cyan)),
            Span::raw(" credentials    "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" quit"),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(body).alignment(Alignment::Left).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" DEEP / OSINT SWARM READY "),
        ),
        centered(area, 82, 17),
    );
}

fn credentials(frame: &mut Frame, app: &App, area: Rect) {
    let items = app.credential_items();
    let rows = if items.is_empty() {
        vec![ListItem::new(Line::from(vec![
            Span::styled("· ", Style::default().fg(Color::DarkGray)),
            Span::raw("No saved profiles. Press l or f to add one."),
        ]))]
    } else {
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let (kind, name, active) = match item {
                    CredentialItem::Llm { name, active } => ("LLM", name, *active),
                    CredentialItem::Firecrawl { name, active } => ("FIRECRAWL", name, *active),
                };
                let cursor = if index == app.credential_selection {
                    "›"
                } else {
                    " "
                };
                let marker = if active { "●" } else { "○" };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{cursor} {marker} "),
                        Style::default().fg(if active {
                            Color::Green
                        } else {
                            Color::DarkGray
                        }),
                    ),
                    Span::styled(format!("{kind:<11}"), Style::default().fg(Color::Magenta)),
                    Span::raw(name.clone()),
                ]))
            })
            .collect()
    };

    frame.render_widget(
        List::new(rows).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" CREDENTIAL STACK · Enter activate · l LLM · f Firecrawl "),
        ),
        area,
    );
}

fn credential_form(frame: &mut Frame, app: &App, area: Rect) {
    let Some(form) = &app.form else {
        return;
    };
    let lines = form
        .labels()
        .iter()
        .enumerate()
        .flat_map(|(index, label)| {
            let value = if index == form.secret_index() {
                "•".repeat(form.values[index].chars().count())
            } else {
                form.values[index].clone()
            };
            let active = index == form.field;
            let color = if active { Color::Cyan } else { Color::White };
            [
                Line::from(Span::styled(*label, Style::default().fg(Color::DarkGray))),
                Line::from(vec![
                    Span::styled(if active { "› " } else { "  " }, Style::default().fg(color)),
                    Span::styled(value, Style::default().fg(color)),
                ]),
            ]
        })
        .collect::<Vec<_>>();
    let title = match form.kind {
        FormKind::Llm => " ADD OPENAI-COMPATIBLE LLM PROFILE ",
        FormKind::Firecrawl => " ADD FIRECRAWL PROFILE ",
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(title),
            )
            .wrap(Wrap { trim: false }),
        centered(area, 88, 20),
    );
}

fn objective(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.objective.is_empty() {
        "Type a public-source OSINT objective..."
    } else {
        &app.objective
    };
    let body = vec![
        Line::from(Span::styled(
            "What should the OSINT swarm establish from public evidence?",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "Public sources only · no intrusion, access-control bypass, or non-public data collection",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("› ", Style::default().fg(Color::Cyan)),
            Span::raw(text),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(" NEW OSINT CASE · Enter launch swarm · Esc cancel "),
            )
            .wrap(Wrap { trim: false }),
        centered(area, 110, 13),
    );
}

fn running(frame: &mut Frame, app: &App, area: Rect) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(8)])
        .split(area);
    running_header(frame, app, vertical[0]);

    if vertical[1].width >= 96 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(vertical[1]);
        activity(frame, app, columns[0]);
        state_stack(frame, app, columns[1]);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(vertical[1]);
        activity(frame, app, rows[0]);
        state_stack(frame, app, rows[1]);
    }
}

fn running_header(frame: &mut Frame, app: &App, area: Rect) {
    let spinner = spinner(app.animation_tick);
    let elapsed = format_duration(app.elapsed());
    let phase_color = phase_color(app.phase);
    let width = area.width.saturating_sub(4) as usize;
    let objective_width = width.saturating_sub(11);
    let operation_width = width.saturating_sub(16);
    let (operation_kind, operation_message) = app
        .current_operation
        .as_ref()
        .map(|(kind, message)| (kind.as_str(), message.as_str()))
        .unwrap_or(("WAIT", "Waiting for swarm orchestration"));

    let running_agents = app
        .agents
        .iter()
        .filter(|agent| agent.status == AgentStatus::Running)
        .count();
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{spinner} OSINT SWARM ACTIVE"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {elapsed}"), Style::default().fg(Color::White)),
            Span::styled(
                format!("   {}", app.phase.label()),
                Style::default()
                    .fg(phase_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "   {running_agents} workers live   idle {}s",
                    app.idle_for().as_secs()
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("OBJECTIVE  ", Style::default().fg(Color::DarkGray)),
            Span::raw(fit(&app.active_objective, objective_width)),
        ]),
        Line::from(vec![
            Span::styled("NOW        ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{operation_kind:<11}"),
                Style::default().fg(event_color(operation_kind)),
            ),
            Span::raw(fit(operation_message, operation_width)),
        ]),
        phase_rail(app.phase),
        Line::from(vec![
            metric("AGENTS", app.agents.len() as u64, Color::Magenta),
            Span::raw("   "),
            metric("EVENTS", app.event_count, Color::Cyan),
            Span::raw("   "),
            metric("CLAIMS", app.claims.len() as u64, Color::Yellow),
            Span::raw("   "),
            metric("LEADS", app.leads.len() as u64, Color::Magenta),
            Span::raw("   "),
            metric("SOURCES", app.sources.len() as u64, Color::Green),
            Span::raw("   "),
            metric(
                "ERRORS",
                app.tool_error_count,
                if app.tool_error_count == 0 {
                    Color::DarkGray
                } else {
                    Color::Red
                },
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" DEEP / LIVE OSINT SWARM "),
        ),
        area,
    );
}

fn activity(frame: &mut Frame, app: &App, area: Rect) {
    let visible = area.height.saturating_sub(2) as usize;
    let skip = app.activity.len().saturating_sub(visible);
    let message_width = area.width.saturating_sub(29) as usize;
    let last_index = app.activity.len().saturating_sub(1);
    let lines = app
        .activity
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(index, item)| activity_line(app, item, index == last_index, message_width))
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" LIVE SWARM TRACE · {} events ", app.event_count)),
        ),
        area,
    );
}

fn activity_line(
    app: &App,
    item: &ActivityItem,
    latest: bool,
    message_width: usize,
) -> Line<'static> {
    let marker = if latest {
        spinner(app.animation_tick)
    } else {
        event_icon(&item.kind)
    };
    let elapsed = format_duration(item.elapsed);
    let message_style = if latest {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Line::from(vec![
        Span::styled(format!("{elapsed} "), Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{marker} "),
            Style::default().fg(event_color(&item.kind)),
        ),
        Span::styled(
            format!("{:<11}", item.kind),
            Style::default().fg(event_color(&item.kind)),
        ),
        Span::styled(fit(&item.message, message_width), message_style),
    ])
}

fn state_stack(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(27),
            Constraint::Percentage(30),
            Constraint::Percentage(19),
            Constraint::Percentage(24),
        ])
        .split(area);
    agents(frame, app, rows[0]);
    claims(frame, app, rows[1]);
    leads(frame, app, rows[2]);
    sources(frame, app, rows[3]);
}

fn agents(frame: &mut Frame, app: &App, area: Rect) {
    let mission_width = area.width.saturating_sub(31) as usize;
    let items = if app.agents.is_empty() {
        vec![empty_item("Workers are starting")]
    } else {
        app.agents
            .iter()
            .map(|agent| {
                let marker = if agent.status == AgentStatus::Running {
                    spinner(app.animation_tick)
                } else {
                    agent_status_icon(agent.status)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{marker} {:<10}", agent.name),
                        Style::default().fg(agent_status_color(agent.status)),
                    ),
                    Span::styled(
                        format!("{:<9}", agent.status.label()),
                        Style::default().fg(agent_status_color(agent.status)),
                    ),
                    Span::styled(
                        format!("e{} x{} ", agent.events, agent.errors),
                        Style::default().fg(if agent.errors == 0 {
                            Color::DarkGray
                        } else {
                            Color::Red
                        }),
                    ),
                    Span::raw(fit(&agent.mission, mission_width)),
                ]))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" AGENT SWARM · {} ", app.agents.len())),
        ),
        area,
    );
}

fn claims(frame: &mut Frame, app: &App, area: Rect) {
    let message_width = area.width.saturating_sub(24) as usize;
    let items = if app.claims.is_empty() {
        vec![empty_item("No material claims yet")]
    } else {
        app.claims
            .iter()
            .map(|(id, statement, status)| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{} C{id:<3} ", status_icon(status)),
                        Style::default().fg(status_color(status)),
                    ),
                    Span::styled(
                        format!("{:<18}", short_status(status)),
                        Style::default().fg(status_color(status)),
                    ),
                    Span::raw(fit(statement, message_width)),
                ]))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" CLAIMS · {} ", app.claims.len())),
        ),
        area,
    );
}

fn leads(frame: &mut Frame, app: &App, area: Rect) {
    let message_width = area.width.saturating_sub(18) as usize;
    let items = if app.leads.is_empty() {
        vec![empty_item("No open leads yet")]
    } else {
        app.leads
            .iter()
            .map(|(id, description, status)| {
                let marker = if status == "ACTIVE" {
                    spinner(app.animation_tick)
                } else {
                    lead_icon(status)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{marker} L{id:<3} {status:<10}"),
                        Style::default().fg(lead_color(status)),
                    ),
                    Span::raw(fit(description, message_width)),
                ]))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" LEADS · {} ", app.leads.len())),
        ),
        area,
    );
}

fn sources(frame: &mut Frame, app: &App, area: Rect) {
    let message_width = area.width.saturating_sub(23) as usize;
    let items = if app.sources.is_empty() {
        vec![empty_item("No scraped sources yet")]
    } else {
        app.sources
            .iter()
            .map(|(id, title, class, duplicate_of)| {
                let class = if class.eq_ignore_ascii_case("unknown") {
                    "UNCLASSIFIED"
                } else {
                    class.as_str()
                };
                let duplicate = duplicate_of
                    .map(|source| format!(" ↳ S{source}"))
                    .unwrap_or_default();
                let color = if duplicate_of.is_some() {
                    Color::DarkGray
                } else {
                    source_color(class)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("◆ S{id:<3} {class:<13}"),
                        Style::default().fg(color),
                    ),
                    Span::raw(fit(&format!("{title}{duplicate}"), message_width)),
                ]))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(format!(" SOURCES · {} ", app.sources.len())),
        ),
        area,
    );
}

fn report(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(
        Paragraph::new(app.report.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green))
                    .title(" OSINT SWARM / EVIDENCE REPORT "),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.report_scroll, 0)),
        area,
    );
}

fn footer(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.screen {
        Screen::Home => "READY",
        Screen::Credentials | Screen::CredentialForm => "CREDENTIALS",
        Screen::Objective => "OBJECTIVE",
        Screen::Running => app.phase.label(),
        Screen::Report => "REPORT",
    };
    let heartbeat = if app.screen == Screen::Running {
        spinner(app.animation_tick)
    } else {
        pulse(app.animation_tick)
    };
    let status = if let Some(error) = &app.error {
        Line::from(vec![
            Span::styled(format!(" {mode} "), Style::default().bg(Color::DarkGray)),
            Span::styled(format!("  × {error}"), Style::default().fg(Color::Red)),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!(" {mode} "),
                Style::default().bg(Color::DarkGray).fg(Color::White),
            ),
            Span::styled(format!("  {heartbeat}  "), Style::default().fg(Color::Cyan)),
            Span::raw("Public-source OSINT · evidence first · uncertainty preserved."),
        ])
    };
    let hints = match app.screen {
        Screen::Running => {
            "q quit  ·  parallel worker state  ·  transient upstream errors retry automatically"
        }
        Screen::Report => "↑↓ scroll  ·  PgUp/PgDn  ·  n new case  ·  h home  ·  q quit",
        Screen::Credentials => {
            "↑↓ select  ·  Enter activate  ·  l add LLM  ·  f add Firecrawl  ·  h home"
        }
        Screen::CredentialForm => "Tab next field  ·  Enter continue/save  ·  Esc cancel",
        Screen::Objective => "Enter launch OSINT swarm  ·  Esc cancel",
        Screen::Home => "n new OSINT case  ·  c credentials  ·  q quit",
    };
    frame.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(Span::styled(hints, Style::default().fg(Color::DarkGray))),
        ]),
        area,
    );
}

fn phase_rail(active: InvestigationPhase) -> Line<'static> {
    let phases = [
        (InvestigationPhase::Discover, "DISCOVER"),
        (InvestigationPhase::Read, "READ"),
        (InvestigationPhase::Analyze, "ANALYZE"),
        (InvestigationPhase::Verify, "VERIFY"),
        (InvestigationPhase::Synthesize, "REPORT"),
    ];
    let mut spans = vec![Span::styled(
        "PIPELINE   ",
        Style::default().fg(Color::DarkGray),
    )];
    for (index, (phase, label)) in phases.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ─ ", Style::default().fg(Color::DarkGray)));
        }
        let selected = active == *phase
            || (active == InvestigationPhase::Admission && *phase == InvestigationPhase::Discover)
            || (active == InvestigationPhase::Complete && *phase == InvestigationPhase::Synthesize);
        spans.push(Span::styled(
            *label,
            if selected {
                Style::default()
                    .fg(phase_color(active))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ));
    }
    Line::from(spans)
}

fn metric(label: &'static str, value: u64, color: Color) -> Span<'static> {
    Span::styled(
        format!("{label} {value}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn empty_item(message: &str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled("· ", Style::default().fg(Color::DarkGray)),
        Span::styled(message.to_owned(), Style::default().fg(Color::DarkGray)),
    ]))
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn spinner(tick: u64) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[(tick as usize) % FRAMES.len()]
}

fn pulse(tick: u64) -> &'static str {
    const FRAMES: [&str; 4] = ["·", "∙", "●", "∙"];
    FRAMES[(tick as usize) % FRAMES.len()]
}

fn format_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn fit(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn event_icon(kind: &str) -> &'static str {
    match kind {
        "SWARM_START" => "◎",
        "AGENT_START" => "▶",
        "AGENT_DONE" => "✓",
        "AGENT_ERROR" => "!",
        "SYNTHESIZE" => "Σ",
        "SEARCH" => "⌕",
        "FOUND" => "+",
        "SCRAPE" | "OPEN" => "↓",
        "MAP" | "CRAWL" => "◇",
        "CLAIM" => "◆",
        "FOLLOW" => "→",
        "EVIDENCE" | "LINK" => "⊕",
        "VERIFY" => "✓",
        "ENTITY" | "RELATIONSHIP" => "◈",
        "TOOL_ERROR" | "ERROR" | "DEAD_END" => "×",
        "DUPLICATE" => "≡",
        "STOP" => "■",
        _ => "·",
    }
}

fn event_color(kind: &str) -> Color {
    match kind {
        "SWARM_START" | "AGENT_START" | "SYNTHESIZE" => Color::Magenta,
        "AGENT_DONE" => Color::Green,
        "AGENT_ERROR" => Color::Red,
        "SEARCH" | "SCRAPE" | "MAP" | "CRAWL" | "INTERACT" | "OPEN" => Color::Cyan,
        "VERIFY" | "EVIDENCE" | "LINK" | "STOP" => Color::Green,
        "CLAIM" | "FOLLOW" | "FOUND" | "ADMISSION" => Color::Yellow,
        "ENTITY" | "RELATIONSHIP" => Color::Magenta,
        "REJECT" | "TOOL_ERROR" | "ERROR" | "DEAD_END" => Color::Red,
        "DUPLICATE" => Color::DarkGray,
        "STATISTICS" | "DIFF" | "CALCULATE" | "DATE_MATH" | "NOTE" => Color::Blue,
        _ => Color::White,
    }
}

fn phase_color(phase: InvestigationPhase) -> Color {
    match phase {
        InvestigationPhase::Admission => Color::Yellow,
        InvestigationPhase::Discover | InvestigationPhase::Read => Color::Cyan,
        InvestigationPhase::Analyze => Color::Magenta,
        InvestigationPhase::Verify
        | InvestigationPhase::Synthesize
        | InvestigationPhase::Complete => Color::Green,
        InvestigationPhase::Failed => Color::Red,
    }
}

fn agent_status_color(status: AgentStatus) -> Color {
    match status {
        AgentStatus::Running => Color::Cyan,
        AgentStatus::Complete => Color::Green,
        AgentStatus::Error => Color::Red,
    }
}

fn agent_status_icon(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "●",
        AgentStatus::Complete => "✓",
        AgentStatus::Error => "!",
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "VERIFIED" => Color::Green,
        "SUPPORTED" | "UNRESOLVED" | "INSUFFICIENT_EVIDENCE" => Color::Yellow,
        "CONFLICTING" | "DISPROVEN" | "DEAD_END" => Color::Red,
        _ => Color::White,
    }
}

fn status_icon(status: &str) -> &'static str {
    match status {
        "VERIFIED" => "✓",
        "SUPPORTED" => "~",
        "UNRESOLVED" | "INSUFFICIENT_EVIDENCE" => "?",
        "CONFLICTING" => "!",
        "DISPROVEN" | "DEAD_END" => "×",
        _ => "·",
    }
}

fn short_status(status: &str) -> &str {
    match status {
        "INSUFFICIENT_EVIDENCE" => "INSUFFICIENT",
        other => other,
    }
}

fn lead_color(status: &str) -> Color {
    match status {
        "ACTIVE" => Color::Cyan,
        "OPEN" => Color::Yellow,
        "EXHAUSTED" => Color::Red,
        "DISCARDED" => Color::DarkGray,
        _ => Color::White,
    }
}

fn lead_icon(status: &str) -> &'static str {
    match status {
        "OPEN" => "○",
        "ACTIVE" => "●",
        "EXHAUSTED" => "×",
        "DISCARDED" => "·",
        _ => "·",
    }
}

fn source_color(class: &str) -> Color {
    match class {
        "PRIMARY" => Color::Green,
        "HIGH_QUALITY_SECONDARY" => Color::Cyan,
        "SECONDARY" => Color::Yellow,
        "WEAK" => Color::Red,
        _ => Color::Magenta,
    }
}
