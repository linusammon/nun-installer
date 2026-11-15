//! Background work. Each effect runs on its own thread and reports back over
//! the channel, so the UI keeps drawing while commands run.

use crate::app::{AppEvent, ChecksResult, Effect, InstallEvent};
use installer_core::{
    apply_console_keymap, check_online, missing_tools, run_plan, system_clock_note,
    system_on_ac_power, Action, KeymapTools, Live, Reachability,
};
use std::sync::mpsc::Sender;

/// Numbers the actions that are shown as steps, in the order they run. Cleanup
/// actions run even after a failure and are not shown, so a step's number is not
/// its position in the plan, whatever the plan's order.
#[derive(Default)]
struct StepCounter(usize);

impl StepCounter {
    fn number(&mut self, action: &Action) -> Option<usize> {
        if action.cleanup {
            return None;
        }
        let n = self.0;
        self.0 += 1;
        Some(n)
    }
}

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
                let mut steps = StepCounter::default();
                let result = run_plan(&plan, &mut live, |_, action| {
                    if let Some(n) = steps.number(action) {
                        let _ = progress.send(AppEvent::Install(InstallEvent::Step(n)));
                    }
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
                    missing_tools: missing_tools(),
                }));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use installer_core::{run_plan, ActionKind, ExecError, Executor, Plan};

    fn action(description: &str, cleanup: bool) -> Action {
        Action {
            description: description.into(),
            kind: ActionKind::Remove("/nonexistent".into()),
            cleanup,
        }
    }

    struct Fails(&'static str);

    impl Executor for Fails {
        fn execute(&mut self, a: &Action) -> Result<(), ExecError> {
            if a.description == self.0 {
                Err(ExecError::Io {
                    description: a.description.clone(),
                    source: std::io::Error::other("boom"),
                })
            } else {
                Ok(())
            }
        }
    }

    fn numbers(plan: &Plan, fail_on: &'static str) -> Vec<(String, Option<usize>)> {
        let mut counter = StepCounter::default();
        let mut seen = Vec::new();
        let _ = run_plan(plan, &mut Fails(fail_on), |_, a| {
            seen.push((a.description.clone(), counter.number(a)));
        });
        seen
    }

    #[test]
    fn steps_are_numbered_among_the_shown_ones_wherever_cleanup_sits() {
        // Cleanup in the middle, not only at the end.
        let plan = Plan {
            actions: vec![
                action("a", false),
                action("cleanup 1", true),
                action("b", false),
                action("c", false),
                action("cleanup 2", true),
            ],
        };
        let got: Vec<_> = numbers(&plan, "never")
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        assert_eq!(got, [Some(0), None, Some(1), Some(2), None]);
    }

    #[test]
    fn after_a_failure_only_the_failed_step_was_numbered() {
        let plan = Plan {
            actions: vec![
                action("a", false),
                action("b", false),
                action("c", false),
                action("cleanup", true),
            ],
        };
        let got = numbers(&plan, "b");
        assert_eq!(
            got,
            [
                ("a".to_owned(), Some(0)),
                ("b".to_owned(), Some(1)),
                ("cleanup".to_owned(), None),
            ],
            "c never ran, so it was never numbered"
        );
    }
}
