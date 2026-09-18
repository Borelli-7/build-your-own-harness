//! Terminal rendering and input handling built on `ratatui` + `crossterm`.

use std::io::{stdout, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};

use crate::state::AppState;

pub enum InputEvent {
    Char(char),
    Backspace,
    Submit,
    Quit,
    Tick,
}

pub fn setup() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

pub fn restore(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Restores the terminal on unwind so a panic never leaves the user's shell in raw/alt-screen mode.
pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

pub fn poll_input(timeout: Duration) -> anyhow::Result<InputEvent> {
    if !event::poll(timeout)? {
        return Ok(InputEvent::Tick);
    }
    if let CEvent::Key(key) = event::read()?
        && key.kind == KeyEventKind::Press {
            return Ok(match key.code {
                KeyCode::Enter => InputEvent::Submit,
                KeyCode::Backspace => InputEvent::Backspace,
                KeyCode::Esc => InputEvent::Quit,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => InputEvent::Quit,
                KeyCode::Char(c) => InputEvent::Char(c),
                _ => InputEvent::Tick,
            });
        }
    Ok(InputEvent::Tick)
}

pub fn render(f: &mut Frame, history: &[(String, String)], input: &str, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(f.area());

    let lines: Vec<Line> = history
        .iter()
        .map(|(role, content)| Line::from(format!("{role}: {content}")))
        .collect();
    let history_widget = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(format!("Harness [{state:?}]")));
    f.render_widget(history_widget, chunks[0]);

    let input_widget =
        Paragraph::new(input).block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input_widget, chunks[1]);
}
