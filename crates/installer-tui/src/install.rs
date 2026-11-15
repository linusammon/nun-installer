//! Runs the install plan on a worker thread and reports back over a channel,
//! so the UI keeps drawing while commands run.

use crate::app::InstallEvent;
use installer_core::{run_plan, Live, Plan};
use std::sync::mpsc;

pub fn spawn(plan: Plan) -> mpsc::Receiver<InstallEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let output = tx.clone();
        let mut live = Live::new(move |line| {
            let _ = output.send(InstallEvent::Output(line.to_owned()));
        });
        let progress = tx.clone();
        let result = run_plan(&plan, &mut live, |i, _| {
            let _ = progress.send(InstallEvent::Step(i));
        });
        let _ = tx.send(InstallEvent::Finished(result.map_err(|e| e.to_string())));
    });
    rx
}
