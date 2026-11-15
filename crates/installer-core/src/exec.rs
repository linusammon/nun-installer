//! Running a [`Plan`]. The [`Executor`] trait is the seam: the real one touches
//! the system, the dry-run one prints, tests use their own recorder.

use crate::plan::{Action, ActionKind, Command, Content, Plan};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("{description}: `{program}` was not found in PATH")]
    CommandNotFound {
        description: String,
        program: String,
    },
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

/// Performs actions on the live system. Output of spawned commands is captured
/// line by line and handed to the sink, never written to the terminal directly,
/// so a full-screen UI stays intact.
pub struct Live {
    sink: Box<dyn FnMut(&str)>,
}

impl Live {
    pub fn new(sink: impl FnMut(&str) + 'static) -> Self {
        Self {
            sink: Box::new(sink),
        }
    }

    /// Forward command output to stderr.
    pub fn stderr() -> Self {
        Self::new(|line| eprintln!("    {line}"))
    }
}

impl Executor for Live {
    fn execute(&mut self, action: &Action) -> Result<(), ExecError> {
        let io_err = |source| ExecError::Io {
            description: action.description.clone(),
            source,
        };
        match &action.kind {
            ActionKind::Run(cmd) => run_command(action, cmd, &mut *self.sink),
            ActionKind::CreateDir { path, mode } => create_dir(path, *mode).map_err(io_err),
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

fn run_command(
    action: &Action,
    cmd: &Command,
    sink: &mut dyn FnMut(&str),
) -> Result<(), ExecError> {
    let io_err = |source| ExecError::Io {
        description: action.description.clone(),
        source,
    };
    let mut child = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .envs(cmd.env.iter().map(|(k, v)| (k, v)))
        .stdin(if cmd.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                ExecError::CommandNotFound {
                    description: action.description.clone(),
                    program: cmd.program.clone(),
                }
            } else {
                ExecError::Io {
                    description: action.description.clone(),
                    source,
                }
            }
        })?;

    // Written on its own thread, started before anything is read: a child that
    // produces a lot of output before it reads its stdin would otherwise block
    // both sides forever. The copy of the secret is wiped when the thread ends.
    let writer = match (&cmd.stdin, child.stdin.take()) {
        (Some(secret), Some(mut pipe)) => {
            let data = Zeroizing::new(secret.expose().as_bytes().to_vec());
            Some(std::thread::spawn(move || {
                // The child may exit without reading everything; that is not our error.
                let _ = pipe.write_all(&data);
            }))
        }
        _ => None,
    };

    let (tx, rx) = mpsc::channel::<String>();
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|stream| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
                if tx
                    .send(String::from_utf8_lossy(&line).into_owned())
                    .is_err()
                {
                    break;
                }
            }
        })
    })
    .collect();
    drop(tx);
    for line in rx {
        sink(&line);
    }
    for r in readers.into_iter().chain(writer) {
        let _ = r.join();
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

fn create_dir(path: &Path, mode: u32) -> io::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
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
    // `mode` only applies when the file is created; enforce it for existing files too.
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
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
    use crate::plan::Content;
    use crate::secret::Secret;

    fn live() -> Live {
        Live::new(|_| {})
    }

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
        live().execute(&write).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi");
        let rm = Action {
            description: "rm".into(),
            kind: ActionKind::Remove(dir.clone()),
            cleanup: true,
        };
        live().execute(&rm).unwrap();
        assert!(!dir.exists());
        live().execute(&rm).unwrap();
    }

    #[test]
    fn live_captures_stdout_and_stderr_and_env() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let lines = Rc::new(RefCell::new(Vec::new()));
        let sink = lines.clone();
        let mut exec = Live::new(move |l| sink.borrow_mut().push(l.to_owned()));
        let a = Action {
            description: "echo".into(),
            kind: ActionKind::Run(
                Command::new("sh", ["-c", "echo out; echo err >&2; echo \"$NUN_T\""])
                    .with_env("NUN_T", "env-ok"),
            ),
            cleanup: false,
        };
        exec.execute(&a).unwrap();
        let mut got = lines.borrow().clone();
        got.sort();
        assert_eq!(got, ["env-ok", "err", "out"]);
    }

    #[test]
    fn a_chatty_child_that_reads_stdin_late_does_not_deadlock() {
        // The child writes far more than a pipe holds before it reads its stdin,
        // so writing stdin and draining output must happen at the same time.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let big = Secret::new("s".repeat(300_000));
            let mut exec = Live::new(|_| {});
            let a = Action {
                description: "chatty".into(),
                kind: ActionKind::Run(
                    Command::new(
                        "sh",
                        [
                            "-c",
                            "head -c 300000 /dev/zero | tr '\\0' x; cat > /dev/null",
                        ],
                    )
                    .with_stdin(big),
                ),
                cleanup: false,
            };
            let _ = tx.send(exec.execute(&a).is_ok());
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(10)),
            Ok(true),
            "deadlocked"
        );
    }

    #[test]
    fn live_feeds_stdin_to_command() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let lines = Rc::new(RefCell::new(Vec::new()));
        let sink = lines.clone();
        let mut exec = Live::new(move |l| sink.borrow_mut().push(l.to_owned()));
        let a = Action {
            description: "cat".into(),
            kind: ActionKind::Run(
                Command::new("cat", Vec::<String>::new()).with_stdin("hi\n".into()),
            ),
            cleanup: false,
        };
        exec.execute(&a).unwrap();
        assert_eq!(*lines.borrow(), ["hi"]);
    }

    #[test]
    fn secret_files_are_private_even_if_they_existed() {
        let dir = std::env::temp_dir().join(format!("nun-mode-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let a = Action {
            description: "w".into(),
            kind: ActionKind::WriteFile {
                path: file.clone(),
                content: Content::Secret(Secret::new("new")),
                mode: 0o600,
            },
            cleanup: false,
        };
        live().execute(&a).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn create_dir_sets_mode() {
        let dir = std::env::temp_dir().join(format!("nun-dir-test-{}", std::process::id()));
        let a = Action {
            description: "d".into(),
            kind: ActionKind::CreateDir {
                path: dir.clone(),
                mode: 0o700,
            },
            cleanup: false,
        };
        live().execute(&a).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn a_missing_program_is_named_not_a_bare_os_error() {
        let a = Action {
            description: "Partition".into(),
            kind: ActionKind::Run(Command::new(
                "definitely-not-installed",
                Vec::<String>::new(),
            )),
            cleanup: false,
        };
        let err = live().execute(&a).unwrap_err();
        assert!(matches!(err, ExecError::CommandNotFound { .. }));
        assert_eq!(
            err.to_string(),
            "Partition: `definitely-not-installed` was not found in PATH"
        );
    }

    #[test]
    fn system_executor_reports_command_failure() {
        let a = Action {
            description: "fail".into(),
            kind: ActionKind::Run(Command::new("false", Vec::<String>::new())),
            cleanup: false,
        };
        assert!(matches!(
            live().execute(&a),
            Err(ExecError::CommandFailed { .. })
        ));
    }
}
