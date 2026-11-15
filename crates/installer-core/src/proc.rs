//! Small helpers shared by the modules that run external programs.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// A path from an environment variable, or a default (usually a bare program
/// name resolved through `PATH`).
pub(crate) fn env_path(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

/// Run `cmd` with `input` on its stdin and collect its output. The input is
/// written on its own thread, so a child that produces a lot of output before
/// it reads its stdin cannot deadlock us.
pub(crate) fn output_with_stdin(cmd: &mut Command, input: &[u8]) -> io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let writer = child.stdin.take().map(|mut pipe| {
        let data = input.to_vec();
        std::thread::spawn(move || {
            // The child may exit without reading everything; that is its call.
            let _ = pipe.write_all(&data);
        })
    });
    let out = child.wait_with_output();
    if let Some(w) = writer {
        let _ = w.join();
    }
    out
}

/// Why a program failed, in the words it used: stderr, else stdout, else its
/// exit status.
pub(crate) fn failure_message(out: &Output) -> String {
    [&out.stderr, &out.stdout]
        .into_iter()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| out.status.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    #[test]
    fn feeds_stdin_and_collects_output() {
        let out = output_with_stdin(&mut sh("cat"), b"hello").unwrap();
        assert_eq!(out.stdout, b"hello");
        assert!(out.status.success());
    }

    #[test]
    fn a_chatty_child_that_reads_late_does_not_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let input = vec![b's'; 300_000];
            let out = output_with_stdin(
                &mut sh("head -c 300000 /dev/zero | tr '\\0' x; cat > /dev/null"),
                &input,
            );
            let _ = tx.send(out.map(|o| o.stdout.len()).ok());
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(10)),
            Ok(Some(300_000)),
            "deadlocked"
        );
    }

    #[test]
    fn a_child_that_ignores_its_stdin_is_not_an_error() {
        let out = output_with_stdin(&mut sh("exit 0"), &vec![b'x'; 300_000]).unwrap();
        assert!(out.status.success());
    }

    #[test]
    fn a_missing_program_is_an_io_error() {
        let err =
            output_with_stdin(&mut Command::new("definitely-not-installed"), b"").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn failure_message_prefers_stderr_then_stdout_then_status() {
        let out = |script| sh(script).output().unwrap();
        assert_eq!(
            failure_message(&out("echo bad >&2; echo ok; exit 3")),
            "bad"
        );
        assert_eq!(
            failure_message(&out("echo only stdout; exit 3")),
            "only stdout"
        );
        assert!(failure_message(&out("exit 3")).contains("3"));
    }

    #[test]
    fn env_path_falls_back_to_the_default() {
        assert_eq!(
            env_path("NUN_DEFINITELY_NOT_SET_ANYWHERE", "/some/default"),
            PathBuf::from("/some/default")
        );
    }
}
