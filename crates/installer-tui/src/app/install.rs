//! Progress of the running installation.

use installer_core::Plan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug)]
pub enum InstallEvent {
    Step(usize),
    Output(String),
    Finished(Result<(), String>),
}

const LOG_LINES: usize = 1000;

#[derive(Debug, Default)]
pub struct InstallState {
    steps: Vec<(String, StepStatus)>,
    log: Vec<String>,
    result: Option<Result<(), String>>,
}

impl InstallState {
    pub fn for_plan(plan: &Plan) -> Self {
        Self {
            steps: plan
                .actions
                .iter()
                .filter(|a| a.is_step())
                .map(|a| (a.description.clone(), StepStatus::Pending))
                .collect(),
            ..Self::default()
        }
    }

    pub fn steps(&self) -> &[(String, StepStatus)] {
        &self.steps
    }

    pub fn log(&self) -> &[String] {
        &self.log
    }

    pub fn result(&self) -> Option<&Result<(), String>> {
        self.result.as_ref()
    }

    pub fn succeeded(&self) -> bool {
        matches!(self.result, Some(Ok(())))
    }

    /// Returns whether the installation is over.
    pub fn on_event(&mut self, ev: InstallEvent) -> bool {
        match ev {
            InstallEvent::Step(i) => {
                for (n, (_, status)) in self.steps.iter_mut().enumerate() {
                    if n < i && *status != StepStatus::Failed {
                        *status = StepStatus::Done;
                    } else if n == i {
                        *status = StepStatus::Running;
                    }
                }
                false
            }
            InstallEvent::Output(line) => {
                self.log.push(line);
                let extra = self.log.len().saturating_sub(LOG_LINES);
                self.log.drain(..extra);
                false
            }
            InstallEvent::Finished(result) => {
                for (_, status) in &mut self.steps {
                    match (&result, *status) {
                        (Ok(()), _) => *status = StepStatus::Done,
                        (Err(_), StepStatus::Running) => *status = StepStatus::Failed,
                        _ => {}
                    }
                }
                self.result = Some(result);
                true
            }
        }
    }

    #[cfg(test)]
    pub fn with_steps(steps: Vec<(String, StepStatus)>) -> Self {
        Self {
            steps,
            ..Self::default()
        }
    }
}
