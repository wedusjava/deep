use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use super::app::{App, CredentialItem, FormKind, Screen};

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

    let body = vec![
        Line::from(Span::styled(
            "DEEP",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from("Evidence-first research agent harness"),
        Line::from(""),
        Line::from(vec![
            Span::raw("LLM         "),
            Span::styled(llm, Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("Firecrawl   "),
            Span::styled(firecrawl, Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from("n  new investigation"),
        Line::from("c  credential profiles"),
        Line::from("q  quit"),
    ];

    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" INVESTIGATION CONSOLE "),
        ),
        centered(area, 76, 18),
    );
}

fn credentials(frame: &mut Frame, app: &App, area: Rect) {
    let items = app.credential_items();
    let rows = if items.is_empty() {
        vec![ListItem::new("No saved profiles. Press l or f to add one.")]
    } else {
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let (kind, name, active) = match item {
                    CredentialItem::Llm { name, active } => ("LLM", name, *active),
                    CredentialItem::Firecrawl { name, active } => ("FIRECRAWL", name, *active),
                };
                let cursor = if index == app.credential_selection { ">" } else { " " };
                let marker = if active { "●" } else { "○" };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{cursor} {marker} "),
                        Style::default().fg(if active { Color::Green } else { Color::DarkGray }),
                    ),
                    Span::styled(format!("{kind:<10}"), Style::default().fg(Color::Magenta)),
                    Span::raw(name.clone()),
                ]))
            })
            .collect()
    };

    frame.render_widget(
        List::new(rows).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" CREDENTIAL STACK · Enter activates · l add LLM · f add Firecrawl "),
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
            let color = if index == form.field { Color::Cyan } else { Color::White };
            [
                Line::from(Span::styled(*label, Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled(format!("> {value}"), Style::default().fg(color))),
            ]
        })
        .collect::<Vec<_>>();
    let title = match form.kind {
        FormKind::Llm => " ADD OPENAI-COMPATIBLE LLM PROFILE ",
        FormKind::Firecrawl => " ADD FIRECRAWL PROFILE ",
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        centered(area, 86, 20),
    );
}

fn objective(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.objective.is_empty() {
        "Type a substantive research objective..."
    } else {
        &app.objective
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" NEW INVESTIGATION · Enter starts · Esc cancels "),
            )
            .wrap(Wrap { trim: false }),
        centered(area, 100, 14),
    );
}

fn running(frame: &mut Frame, app: &App, area: Rect) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(area);
    frame.render_widget(
        Paragraph::new(app.active_objective.as_str())
            .block(Block::default().borders(Borders::ALL).title(" CASE / OBJECTIVE "))
            .wrap(Wrap { trim: true }),
        vertical[0],
    );

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(vertical[1]);
    activity(frame, app, columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(columns[1]);
    claims(frame, app, right[0]);
    sources(frame, app, right[1]);
}

fn activity(frame: &mut Frame, app: &App, area: Rect) {
    let visible = area.height.saturating_sub(2) as usize;
    let skip = app.activity.len().saturating_sub(visible);
    let lines = app
        .activity
        .iter()
        .skip(skip)
        .map(|(kind, message)| {
            Line::from(vec![
                Span::styled(
                    format!("{kind:<12}"),
                    Style::default()
                        .fg(event_color(kind))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(message.clone()),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" ACTIVITY "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn claims(frame: &mut Frame, app: &App, area: Rect) {
    let items = app
        .claims
        .iter()
        .map(|(id, statement, status)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("C{id:<3} {status:<22}"),
                    Style::default().fg(status_color(status)),
                ),
                Span::raw(statement.clone()),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" CLAIMS ")),
        area,
    );
}

fn sources(frame: &mut Frame, app: &App, area: Rect) {
    let items = app
        .sources
        .iter()
        .map(|(id, title, class, duplicate_of)| {
            let duplicate = duplicate_of
                .map(|source| format!(" ↳ S{source}"))
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("S{id:<3} {class:<10}"),
                    Style::default().fg(if duplicate_of.is_some() {
                        Color::DarkGray
                    } else {
                        Color::Magenta
                    }),
                ),
                Span::raw(format!("{title}{duplicate}")),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" EVIDENCE SOURCES "),
        ),
        area,
    );
}

fn report(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(
        Paragraph::new(app.report.as_str())
            .block(Block::default().borders(Borders::ALL).title(" RESULT "))
            .wrap(Wrap { trim: false })
            .scroll((app.report_scroll, 0)),
        area,
    );
}

fn footer(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.screen {
        Screen::Home => " HOME ",
        Screen::Credentials | Screen::CredentialForm => " CREDENTIALS ",
        Screen::Objective => " OBJECTIVE ",
        Screen::Running => " INVESTIGATING ",
        Screen::Report => " REPORT ",
    };
    let mut spans = vec![Span::styled(
        mode,
        Style::default().bg(Color::DarkGray).fg(Color::White),
    )];
    if let Some(error) = &app.error {
        spans.push(Span::styled(
            format!("  {error}"),
            Style::default().fg(Color::Red),
        ));
    } else {
        spans.push(Span::raw("  Evidence first. Conclusions second."));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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

fn event_color(kind: &str) -> Color {
    match kind {
        "SEARCH" | "SCRAPE" | "MAP" | "CRAWL" | "INTERACT" | "OPEN" => Color::Cyan,
        "VERIFY" | "EVIDENCE" | "LINK" => Color::Green,
        "CLAIM" | "FOLLOW" | "FOUND" | "ADMISSION" => Color::Yellow,
        "ENTITY" | "RELATIONSHIP" => Color::Magenta,
        "REJECT" | "TOOL_ERROR" | "ERROR" => Color::Red,
        "DUPLICATE" => Color::DarkGray,
        _ => Color::White,
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
