//! Background work. Each effect runs on its own thread and reports back over
//! the channel, so the UI keeps drawing while commands run.

use crate::app::{AppEvent, ChecksResult, Effect, InstallEvent};
use installer_core::{
    apply_console_keymap, check_online, run_plan, system_clock_note, system_on_ac_power,
    KeymapTools, Live, Reachability,
};
use std::sync::mpsc::Sender;

/// Start the thread for an effect that needs one. `Quit` and `Reboot` are the
/// event loop's own business and are ignored here.
pub fn spawn(effect: Effect, tx: Sender<AppEvent>) {
    match effect {
        Effect::Quit | Effect::Reboot => {}
        Effect::Install(plan) => {
            std::thread::spawn(move || {
                let output = tx.clone();
                let mut live = Live::new(move |line| {
                    let _ = output.send(AppEvent::Install(InstallEvent::Output(line.to_owned())));
                });
                let progress = tx.clone();
                let result = run_plan(&plan, &mut live, |i, _| {
                    let _ = progress.send(AppEvent::Install(InstallEvent::Step(i)));
                });
                let _ = tx.send(AppEvent::Install(InstallEvent::Finished(
                    result.map_err(|e| e.to_string()),
                )));
            });
        }
        Effect::ApplyKeymap { layout, variant } => {
            std::thread::spawn(move || {
                let described = match &variant {
                    Some(v) => format!("{layout} ({v})"),
                    None => layout.clone(),
                };
                let result =
                    apply_console_keymap(&KeymapTools::from_env(), &layout, variant.as_deref())
                        .map(|()| described)
                        .map_err(|e| e.to_string());
                let _ = tx.send(AppEvent::Keymap(result));
            });
        }
        Effect::RunChecks { internet } => {
            std::thread::spawn(move || {
                let internet = if internet {
                    check_online()
                } else {
                    Reachability {
                        github: true,
                        cache: true,
                    }
                };
                let _ = tx.send(AppEvent::Checks(ChecksResult {
                    internet,
                    clock_note: system_clock_note(),
                    on_ac: system_on_ac_power(),
                }));
            });
        }
    }
}
