//! Terminal setup and the event loop.

use crate::app::{App, Effect};
use crate::{install, ui};
use crossterm::event::{self, Event, KeyEventKind};
use installer_core::{list_disks, Catalog, Host};
use std::io;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// The timezone the live system is already set to, if it is a real one.
fn current_timezone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    target.split_once("zoneinfo/").map(|(_, tz)| tz.to_owned())
}

/// Run the interactive installer. Returns when the user quits or reboots.
pub fn run(catalog: Catalog, host: Host) -> io::Result<()> {
    let mut app = App::new(catalog, host, list_disks);
    if let Some(tz) = current_timezone() {
        app.set_default_timezone(&tz);
    }

    // Restores the terminal on panic as well.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();

    if result? {
        let _ = std::process::Command::new("systemctl")
            .arg("reboot")
            .status();
    }
    Ok(())
}

/// Returns whether to reboot.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<bool> {
    let mut installing: Option<Receiver<_>> = None;
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        if let Some(rx) = &installing {
            while let Ok(ev) = rx.try_recv() {
                app.on_install_event(ev);
            }
        }

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match app.handle_key(key) {
                Effect::None => {}
                Effect::Quit => return Ok(false),
                Effect::Reboot => return Ok(true),
                Effect::Install(plan) => installing = Some(install::spawn(plan)),
            }
        }
    }
}
