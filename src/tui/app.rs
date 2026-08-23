use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::{
    agent::{ResearchEvent, run_investigation},
    credentials::{CredentialStore, FirecrawlProfile, LlmProfile},
};

const MAX_ACTIVITY: usize = 500;
const MAX_UI_MESSAGE_CHARS: usize = 220;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Screen {
    Home,
    Credentials,
    CredentialForm,
    Objective,
    Running,
    Report,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InvestigationPhase {
    Admission,
    Discover,
    Read,
    Analyze,
    Verify,
    Synthesize,
    Complete,
    Failed,
}

impl InvestigationPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Admission => "ADMISSION",
            Self::Discover => "DISCOVER",
            Self::Read => "READ",
            Self::Analyze => "ANALYZE",
            Self::Verify => "VERIFY",
            Self::Synthesize => "SYNTHESIZE",
            Self::Complete => "COMPLETE",
            Self::Failed => "FAILED",
        }
    }
}

#[derive(Clone)]
pub(super) struct ActivityItem {
    pub kind: String,
    pub message: String,
    pub elapsed: Duration,
}

#[derive(Clone, Copy)]
pub(super) enum FormKind {
    Llm,
    Firecrawl,
}

pub(super) struct CredentialForm {
    pub kind: FormKind,
    pub field: usize,
    pub values: Vec<String>,
}

impl CredentialForm {
    fn llm() -> Self {
        Self {
            kind: FormKind::Llm,
            field: 0,
            values: vec![
                String::new(),
                "https://api.openai.com/v1".into(),
                String::new(),
                String::new(),
            ],
        }
    }

    fn firecrawl() -> Self {
        Self {
            kind: FormKind::Firecrawl,
            field: 0,
            values: vec![
                String::new(),
                "https://api.firecrawl.dev".into(),
                String::new(),
            ],
        }
    }

    pub fn labels(&self) -> &'static [&'static str] {
        match self.kind {
            FormKind::Llm => &["Profile name", "Base URL", "Model", "API key"],
            FormKind::Firecrawl => &["Profile name", "Base URL", "API key"],
        }
    }

    pub fn secret_index(&self) -> usize {
        self.values.len() - 1
    }
}

#[derive(Clone)]
pub(super) enum CredentialItem {
    Llm { name: String, active: bool },
    Firecrawl { name: String, active: bool },
}

pub(super) struct App {
    pub db_path: PathBuf,
    pub credentials: CredentialStore,
    pub screen: Screen,
    pub should_quit: bool,
    pub credential_selection: usize,
    pub form: Option<CredentialForm>,
    pub objective: String,
    pub active_objective: String,
    pub activity: VecDeque<ActivityItem>,
    pub claims: Vec<(i64, String, String)>,
    pub leads: Vec<(i64, String, String)>,
    pub sources: Vec<(i64, String, String, Option<i64>)>,
    pub report: String,
    pub report_scroll: u16,
    pub receiver: Option<UnboundedReceiver<ResearchEvent>>,
    pub error: Option<String>,
    pub phase: InvestigationPhase,
    pub animation_tick: u64,
    pub event_count: u64,
    pub tool_error_count: u64,
    pub current_operation: Option<(String, String)>,
    started_at: Option<Instant>,
    last_event_at: Option<Instant>,
}

impl App {
    pub fn new(db_path: PathBuf, credentials: CredentialStore) -> Self {
        let screen = if credentials.has_required_profiles() {
            Screen::Home
        } else {
            Screen::Credentials
        };
        Self {
            db_path,
            credentials,
            screen,
            should_quit: false,
            credential_selection: 0,
            form: None,
            objective: String::new(),
            active_objective: String::new(),
            activity: VecDeque::new(),
            claims: Vec::new(),
            leads: Vec::new(),
            sources: Vec::new(),
            report: String::new(),
            report_scroll: 0,
            receiver: None,
            error: None,
            phase: InvestigationPhase::Admission,
            animation_tick: 0,
            event_count: 0,
            tool_error_count: 0,
            current_operation: None,
            started_at: None,
            last_event_at: None,
        }
    }

    pub fn tick(&mut self) {
        self.animation_tick = self.animation_tick.wrapping_add(1);
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at
            .map_or(Duration::ZERO, |started| started.elapsed())
    }

    pub fn idle_for(&self) -> Duration {
        self.last_event_at
            .map_or(Duration::ZERO, |last| last.elapsed())
    }

    pub async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.screen {
            Screen::Home => self.handle_home(key),
            Screen::Credentials => self.handle_credentials(key)?,
            Screen::CredentialForm => self.handle_form(key)?,
            Screen::Objective => self.handle_objective(key)?,
            Screen::Running => self.handle_running(key),
            Screen::Report => self.handle_report(key),
        }
        Ok(())
    }

    fn handle_home(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') => self.screen = Screen::Credentials,
            KeyCode::Char('n') if self.credentials.has_required_profiles() => {
                self.objective.clear();
                self.error = None;
                self.screen = Screen::Objective;
            }
            KeyCode::Char('n') => {
                self.error = Some("Configure an active LLM and Firecrawl profile first.".into());
                self.screen = Screen::Credentials;
            }
            _ => {}
        }
    }

    fn handle_credentials(&mut self, key: KeyEvent) -> Result<()> {
        let items = self.credential_items();
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') => self.screen = Screen::Home,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('l') => {
                self.form = Some(CredentialForm::llm());
                self.screen = Screen::CredentialForm;
            }
            KeyCode::Char('f') => {
                self.form = Some(CredentialForm::firecrawl());
                self.screen = Screen::CredentialForm;
            }
            KeyCode::Up => {
                self.credential_selection = self.credential_selection.saturating_sub(1);
            }
            KeyCode::Down => {
                if !items.is_empty() {
                    self.credential_selection =
                        (self.credential_selection + 1).min(items.len() - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(item) = items.get(self.credential_selection) {
                    match item {
                        CredentialItem::Llm { name, .. } => {
                            self.credentials.set_active_llm(name)?
                        }
                        CredentialItem::Firecrawl { name, .. } => {
                            self.credentials.set_active_firecrawl(name)?
                        }
                    }
                    self.credentials.save()?;
                    self.error = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.form.as_mut() else {
            self.screen = Screen::Credentials;
            return Ok(());
        };

        match key.code {
            KeyCode::Esc => {
                self.form = None;
                self.screen = Screen::Credentials;
            }
            KeyCode::Backspace => {
                form.values[form.field].pop();
            }
            KeyCode::Tab => form.field = (form.field + 1) % form.values.len(),
            KeyCode::BackTab => {
                form.field = if form.field == 0 {
                    form.values.len() - 1
                } else {
                    form.field - 1
                };
            }
            KeyCode::Enter => {
                if form.field + 1 < form.values.len() {
                    form.field += 1;
                } else {
                    self.save_form()?;
                }
            }
            KeyCode::Char(character) => form.values[form.field].push(character),
            _ => {}
        }
        Ok(())
    }

    fn save_form(&mut self) -> Result<()> {
        let form = self
            .form
            .take()
            .ok_or_else(|| anyhow!("missing credential form"))?;
        if form.values.iter().any(|value| value.trim().is_empty()) {
            self.error = Some("Every credential field is required.".into());
            self.form = Some(form);
            return Ok(());
        }

        match form.kind {
            FormKind::Llm => self.credentials.add_llm(LlmProfile {
                name: form.values[0].trim().to_owned(),
                base_url: form.values[1].trim().trim_end_matches('/').to_owned(),
                model: form.values[2].trim().to_owned(),
                api_key: form.values[3].trim().to_owned(),
            }),
            FormKind::Firecrawl => self.credentials.add_firecrawl(FirecrawlProfile {
                name: form.values[0].trim().to_owned(),
                base_url: form.values[1].trim().trim_end_matches('/').to_owned(),
                api_key: form.values[2].trim().to_owned(),
            }),
        }
        self.credentials.save()?;
        self.credential_selection = 0;
        self.error = None;
        self.screen = Screen::Credentials;
        Ok(())
    }

    fn handle_objective(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Backspace => {
                self.objective.pop();
            }
            KeyCode::Enter if !self.objective.trim().is_empty() => self.start_investigation()?,
            KeyCode::Char(character) => self.objective.push(character),
            _ => {}
        }
        Ok(())
    }

    fn start_investigation(&mut self) -> Result<()> {
        let llm = self
            .credentials
            .active_llm()
            .ok_or_else(|| anyhow!("no active LLM credential"))?;
        let firecrawl = self
            .credentials
            .active_firecrawl()
            .ok_or_else(|| anyhow!("no active Firecrawl credential"))?;
        let objective = self.objective.trim().to_owned();
        let db_path = self.db_path.clone();
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(run_investigation(
            objective.clone(),
            llm,
            firecrawl,
            db_path,
            tx,
        ));

        let now = Instant::now();
        self.active_objective = objective;
        self.activity.clear();
        self.claims.clear();
        self.leads.clear();
        self.sources.clear();
        self.report.clear();
        self.report_scroll = 0;
        self.receiver = Some(rx);
        self.error = None;
        self.phase = InvestigationPhase::Admission;
        self.event_count = 0;
        self.tool_error_count = 0;
        self.current_operation = None;
        self.started_at = Some(now);
        self.last_event_at = Some(now);
        self.screen = Screen::Running;
        Ok(())
    }

    fn handle_running(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Char('q')) {
            self.should_quit = true;
        }
    }

    fn handle_report(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('h') | KeyCode::Esc => self.screen = Screen::Home,
            KeyCode::Char('n') => {
                self.objective.clear();
                self.screen = Screen::Objective;
            }
            KeyCode::Up => self.report_scroll = self.report_scroll.saturating_sub(1),
            KeyCode::Down => self.report_scroll = self.report_scroll.saturating_add(1),
            KeyCode::PageUp => self.report_scroll = self.report_scroll.saturating_sub(10),
            KeyCode::PageDown => self.report_scroll = self.report_scroll.saturating_add(10),
            _ => {}
        }
    }

    pub fn drain_agent_events(&mut self) {
        let mut pending = Vec::new();
        if let Some(receiver) = self.receiver.as_mut() {
            while let Ok(event) = receiver.try_recv() {
                pending.push(event);
            }
        }

        for event in pending {
            match event {
                ResearchEvent::Activity { kind, message } => {
                    let elapsed = self.elapsed();
                    self.event_count = self.event_count.saturating_add(1);
                    if matches!(kind.as_str(), "TOOL_ERROR" | "ERROR") {
                        self.tool_error_count = self.tool_error_count.saturating_add(1);
                    }
                    self.phase = phase_for_event(&kind, self.phase);
                    let message = compact_activity_message(&kind, &message);
                    self.current_operation = Some((kind.clone(), message.clone()));
                    self.last_event_at = Some(Instant::now());
                    if self.activity.len() == MAX_ACTIVITY {
                        self.activity.pop_front();
                    }
                    self.activity.push_back(ActivityItem {
                        kind,
                        message,
                        elapsed,
                    });
                }
                ResearchEvent::Claim {
                    id,
                    statement,
                    status,
                } => {
                    if let Some(item) = self.claims.iter_mut().find(|item| item.0 == id) {
                        *item = (id, statement, status);
                    } else {
                        self.claims.push((id, statement, status));
                    }
                }
                ResearchEvent::Lead {
                    id,
                    description,
                    status,
                } => {
                    if let Some(item) = self.leads.iter_mut().find(|item| item.0 == id) {
                        *item = (id, description, status);
                    } else {
                        self.leads.push((id, description, status));
                    }
                }
                ResearchEvent::Source {
                    id,
                    title,
                    url,
                    source_class,
                    duplicate_of,
                } => {
                    let label = if title.trim().is_empty() { url } else { title };
                    if let Some(item) = self.sources.iter_mut().find(|item| item.0 == id) {
                        *item = (id, label, source_class, duplicate_of);
                    } else {
                        self.sources.push((id, label, source_class, duplicate_of));
                    }
                }
                ResearchEvent::Finished { case_id, report } => {
                    self.phase = InvestigationPhase::Complete;
                    self.current_operation = None;
                    self.report = format!("CASE {case_id}\n\n{report}");
                    self.receiver = None;
                    self.screen = Screen::Report;
                }
                ResearchEvent::Failed { message } => {
                    self.phase = InvestigationPhase::Failed;
                    self.tool_error_count = self.tool_error_count.saturating_add(1);
                    let compact = compact_activity_message("ERROR", &message);
                    self.error = Some(compact.clone());
                    self.activity.push_back(ActivityItem {
                        kind: "ERROR".into(),
                        message: compact,
                        elapsed: self.elapsed(),
                    });
                }
            }
        }
    }

    pub fn credential_items(&self) -> Vec<CredentialItem> {
        let mut items = Vec::new();
        for profile in &self.credentials.llm {
            items.push(CredentialItem::Llm {
                name: profile.name.clone(),
                active: self.credentials.active_llm.as_deref() == Some(profile.name.as_str()),
            });
        }
        for profile in &self.credentials.firecrawl {
            items.push(CredentialItem::Firecrawl {
                name: profile.name.clone(),
                active: self.credentials.active_firecrawl.as_deref() == Some(profile.name.as_str()),
            });
        }
        items
    }
}

fn phase_for_event(kind: &str, current: InvestigationPhase) -> InvestigationPhase {
    match kind {
        "ADMISSION" => InvestigationPhase::Admission,
        "SEARCH" | "MAP" | "CRAWL" | "FOLLOW" | "FOUND" => InvestigationPhase::Discover,
        "SCRAPE" | "OPEN" | "INTERACT" | "DUPLICATE" => InvestigationPhase::Read,
        "CLAIM" | "ENTITY" | "RELATIONSHIP" | "NOTE" | "CALCULATE" | "DATE_MATH" | "STATISTICS"
        | "DIFF" => InvestigationPhase::Analyze,
        "EVIDENCE" | "LINK" | "VERIFY" => InvestigationPhase::Verify,
        "STOP" => InvestigationPhase::Synthesize,
        "ERROR" => InvestigationPhase::Failed,
        _ => current,
    }
}

fn compact_activity_message(kind: &str, message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = if kind == "TOOL_ERROR" {
        compact_tool_error(&normalized)
    } else {
        match normalized.as_str() {
            "Evaluating whether the objective requires substantive research." => {
                "Checking whether this objective requires evidence-based research".to_owned()
            }
            "Search results received; snippets remain discovery-only." => {
                "Results received · snippets remain discovery-only".to_owned()
            }
            other => other.to_owned(),
        }
    };
    truncate_chars(&compact, MAX_UI_MESSAGE_CHARS)
}

fn compact_tool_error(message: &str) -> String {
    let tool = message.split(':').next().unwrap_or("tool").trim();
    if let Some(index) = message.find("returned HTTP") {
        let status = message[index + "returned ".len()..]
            .split(':')
            .next()
            .unwrap_or("HTTP error")
            .trim();
        return format!("{tool} · {status} · pivoting to another source");
    }
    if let Some(index) = message.find("HTTP ") {
        let status = message[index..]
            .split(['{', '['])
            .next()
            .unwrap_or("HTTP error")
            .trim_matches(|character: char| character == ':' || character.is_whitespace());
        return format!("{tool} · {status}");
    }
    truncate_chars(message, 150)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
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
