//! Terminal setup and the event loop.

use crate::app::{App, AppEvent, Effect};
use crate::{ui, workers};
use crossterm::event::{self, Event, KeyEventKind};
use installer_core::{list_disks, Catalog, Host};
use std::io;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

/// The timezone the live system is already set to, if it is a real one.
fn current_timezone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    target.split_once("zoneinfo/").map(|(_, tz)| tz.to_owned())
}

/// `NUN_OFFLINE=1` marks media that carries everything the install needs.
fn offline() -> bool {
    std::env::var("NUN_OFFLINE").is_ok_and(|v| v == "1")
}

/// Run the interactive installer. Returns when the user quits or reboots.
pub fn run(catalog: Catalog, host: Host) -> io::Result<()> {
    let mut app = App::new(catalog, host, offline(), list_disks);
    if let Some(tz) = current_timezone() {
        app.set_default_timezone(&tz);
    }

    // Restores the terminal on panic as well.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();

    if result? {
        let rebooted = std::process::Command::new("systemctl")
            .arg("reboot")
            .status()
            .is_ok_and(|s| s.success());
        if !rebooted {
            return Err(io::Error::other(
                "the installation finished, but `systemctl reboot` failed; reboot by hand",
            ));
        }
    }
    Ok(())
}

/// Run effects; returns `Some(reboot)` when the loop should end.
fn run_effects(effects: Vec<Effect>, tx: &Sender<AppEvent>) -> Option<bool> {
    for effect in effects {
        match effect {
            Effect::Quit => return Some(false),
            Effect::Reboot => return Some(true),
            other => workers::spawn(other, tx.clone()),
        }
    }
    None
}

/// Returns whether to reboot.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<bool> {
    let (tx, rx) = mpsc::channel();
    let start = app.start();
    run_effects(start, &tx);

    loop {
        terminal.draw(|f| ui::render(f, app))?;

        while let Ok(ev) = rx.try_recv() {
            match ev {
                AppEvent::Install(e) => app.on_install_event(e),
                AppEvent::Keymap(r) => app.on_keymap_result(r),
                AppEvent::Checks(r) => app.on_checks(r),
            }
        }

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if let Some(reboot) = run_effects(app.handle_key(key), &tx) {
                return Ok(reboot);
            }
        }
    }
}
