//! Interactive mode: session list + tabs with embedded agent terminals.

pub mod app;
pub mod keys;
pub mod term;
pub mod ui;

use crate::config::Config;
use anyhow::Result;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;
use resume_ai_core::i18n::I18n;
use std::sync::mpsc;
use std::time::Duration;

pub fn run(cfg: Config, t: I18n) -> Result<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let mut app = app::App::new(cfg, t, tx);

    let result = (|| -> Result<()> {
        let mut dirty = true;
        while !app.quit {
            if dirty {
                terminal.draw(|f| ui::draw(f, &mut app))?;
                dirty = false;
            }
            // Wake up for keys, or for agent output (signalled through the channel)
            if event::poll(Duration::from_millis(25))? {
                match event::read()? {
                    // Windows reports key releases too; only presses matter
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                    Event::Paste(text) => app.on_paste(text),
                    Event::Resize(..) => {}
                    _ => continue,
                }
                dirty = true;
            }
            while rx.try_recv().is_ok() {
                dirty = true;
            }
            let had_status = app.status.is_some();
            app.tick();
            if had_status != app.status.is_some() || app.tabs.iter().any(|t| t.term.as_ref().is_some_and(|x| x.pending.is_some()))
            {
                dirty = true;
            }
        }
        Ok(())
    })();

    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    drop(app); // closes every tab (kills the agents)
    result
}
