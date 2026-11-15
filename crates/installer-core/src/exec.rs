//! Running a [`Plan`]. The [`Executor`] trait is the seam: the real one touches
//! the system, the dry-run one prints, tests use their own recorder.

use crate::plan::{Action, ActionKind, Command, Content, Plan};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Stdio;

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("{description}: {source}")]
    Io {
        description: String,
        source: io::Error,
    },
    #[error("{description}: `{program}` exited with {status}")]
    CommandFailed {
        description: String,
        program: String,
        status: std::process::ExitStatus,
    },
}

pub trait Executor {
    fn execute(&mut self, action: &Action) -> Result<(), ExecError>;
}

/// Run all actions in order. On the first failure, stop, still run the cleanup
/// actions, and return that failure. `progress` is called before each action.
pub fn run_plan(
    plan: &Plan,
    executor: &mut dyn Executor,
    mut progress: impl FnMut(usize, &Action),
) -> Result<(), ExecError> {
    let mut result = Ok(());
    for (i, action) in plan.actions.iter().enumerate() {
        if result.is_err() && !action.cleanup {
            continue;
        }
        progress(i, action);
        let outcome = executor.execute(action);
        if result.is_ok() {
            result = outcome;
        }
    }
    result
}

/// Prints each action instead of performing it.
pub struct DryRun<W: Write>(pub W);

impl<W: Write> Executor for DryRun<W> {
    fn execute(&mut self, action: &Action) -> Result<(), ExecError> {
        write!(self.0, "{action}").map_err(|source| ExecError::Io {
            description: action.description.clone(),
            source,
        })
    }
}

/// Performs actions on the live system.
pub struct Live;

impl Executor for Live {
    fn execute(&mut self, action: &Action) -> Result<(), ExecError> {
        let io_err = |source| ExecError::Io {
            description: action.description.clone(),
            source,
        };
        match &action.kind {
            ActionKind::Run(cmd) => run_command(action, cmd),
            ActionKind::WriteFile {
                path,
                content,
                mode,
            } => {
                let data = match content {
                    Content::Plain(s) => s.as_str(),
                    Content::Secret(s) => s.expose(),
                };
                write_file(path, data, *mode).map_err(io_err)
            }
            ActionKind::Remove(path) => remove(path).map_err(io_err),
        }
    }
}

fn run_command(action: &Action, cmd: &Command) -> Result<(), ExecError> {
    let io_err = |source| ExecError::Io {
        description: action.description.clone(),
        source,
    };
    let mut child = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .stdin(if cmd.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(io_err)?;
    if let (Some(secret), Some(mut stdin)) = (&cmd.stdin, child.stdin.take()) {
        stdin
            .write_all(secret.expose().as_bytes())
            .map_err(io_err)?;
    }
    let status = child.wait().map_err(io_err)?;
    if status.success() {
        Ok(())
    } else {
        Err(ExecError::CommandFailed {
            description: action.description.clone(),
            program: cmd.program.clone(),
            status,
        })
    }
}

fn write_file(path: &Path, data: &str, mode: u32) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)?;
    file.write_all(data.as_bytes())
}

fn remove(path: &Path) -> io::Result<()> {
    let result = match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) => Err(e),
    };
    match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Content, Secret};

    #[derive(Default)]
    struct Recorder {
        seen: Vec<String>,
        fail_on: Option<String>,
    }

    impl Executor for Recorder {
        fn execute(&mut self, action: &Action) -> Result<(), ExecError> {
            self.seen.push(action.description.clone());
            if self.fail_on.as_deref() == Some(&action.description) {
                return Err(ExecError::Io {
                    description: action.description.clone(),
                    source: io::Error::other("boom"),
                });
            }
            Ok(())
        }
    }

    fn act(description: &str, cleanup: bool) -> Action {
        Action {
            description: description.into(),
            kind: ActionKind::Remove("/nonexistent".into()),
            cleanup,
        }
    }

    fn plan() -> Plan {
        Plan {
            actions: vec![act("a", false), act("b", false), act("c", true)],
        }
    }

    #[test]
    fn runs_everything_in_order() {
        let mut r = Recorder::default();
        run_plan(&plan(), &mut r, |_, _| {}).unwrap();
        assert_eq!(r.seen, ["a", "b", "c"]);
    }

    #[test]
    fn failure_skips_rest_but_runs_cleanup() {
        let mut r = Recorder {
            fail_on: Some("a".into()),
            ..Default::default()
        };
        let err = run_plan(&plan(), &mut r, |_, _| {}).unwrap_err();
        assert!(err.to_string().starts_with("a:"));
        assert_eq!(r.seen, ["a", "c"]);
    }

    #[test]
    fn cleanup_failure_does_not_mask_original_error() {
        let mut r = Recorder {
            fail_on: Some("c".into()),
            ..Default::default()
        };
        // all normal actions succeed, so the cleanup error is the first one
        assert!(run_plan(&plan(), &mut r, |_, _| {}).is_err());
        let mut r = Recorder {
            fail_on: Some("a".into()),
            ..Default::default()
        };
        let err = run_plan(&plan(), &mut r, |_, _| {}).unwrap_err();
        assert!(err.to_string().starts_with("a:"));
    }

    #[test]
    fn dry_run_redacts_secrets() {
        let secret = Action {
            description: "secret".into(),
            kind: ActionKind::WriteFile {
                path: "/tmp/x".into(),
                content: Content::Secret(Secret::new("hunter2hunter2")),
                mode: 0o600,
            },
            cleanup: false,
        };
        let mut out = Vec::new();
        DryRun(&mut out).execute(&secret).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("<redacted>"));
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn system_executor_writes_and_removes() {
        let dir = std::env::temp_dir().join(format!("nun-exec-test-{}", std::process::id()));
        let file = dir.join("sub/f.txt");
        let write = Action {
            description: "w".into(),
            kind: ActionKind::WriteFile {
                path: file.clone(),
                content: Content::Plain("hi".into()),
                mode: 0o600,
            },
            cleanup: false,
        };
        Live.execute(&write).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi");
        let rm = Action {
            description: "rm".into(),
            kind: ActionKind::Remove(dir.clone()),
            cleanup: true,
        };
        Live.execute(&rm).unwrap();
        assert!(!dir.exists());
        Live.execute(&rm).unwrap();
    }

    #[test]
    fn system_executor_reports_command_failure() {
        let a = Action {
            description: "fail".into(),
            kind: ActionKind::Run(Command::new("false", Vec::<String>::new())),
            cleanup: false,
        };
        assert!(matches!(
            Live.execute(&a),
            Err(ExecError::CommandFailed { .. })
        ));
    }
}
