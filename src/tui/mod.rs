mod app;
mod render;

use std::{io, path::PathBuf, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::credentials::CredentialStore;
use app::App;

const FRAME_TIME: Duration = Duration::from_millis(80);

pub async fn run(db_path: PathBuf, credentials: CredentialStore) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, App::new(db_path, credentials)).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
) -> Result<()> {
    while !app.should_quit {
        app.tick();
        app.drain_agent_events();
        terminal.draw(|frame| render::draw(frame, &app))?;

        if event::poll(FRAME_TIME)?
            && let Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
        {
            app.handle_key(key).await?;
        }
    }
    Ok(())
}
