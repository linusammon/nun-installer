//! Installing needs root. When started as a normal user (for example with
//! `nix run` on the official ISO, where you are `nixos`), re-run through sudo
//! before the terminal is touched, so sudo's password prompt is plain I/O.

use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// Environment the installer relies on. sudo resets the environment, so these
/// are passed through explicitly with `env`.
///
/// Passing the caller's `PATH` means the escalated installer runs `disko`, `nix`
/// and `nixos-install` from a directory list the caller controls, instead of
/// sudo's `secure_path`. That is not an escalation: whoever can run this through
/// sudo can already run anything as root. It is needed because the Nix package
/// supplies `disko` through `PATH`. Resolve them to absolute paths if the
/// installer is ever run where the caller should not be trusted with that.
fn keep(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|n| n.starts_with("NUN_") || n == "PATH" || n == "TERM")
}

/// `sudo env KEY=VALUE... <exe> <args...>`
pub fn escalation_command(
    exe: &Path,
    args: &[OsString],
    env: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = vec!["sudo".into(), "env".into()];
    let mut vars: Vec<_> = env.into_iter().filter(|(k, _)| keep(k)).collect();
    vars.sort();
    cmd.extend(vars.into_iter().map(|(mut k, v)| {
        k.push("=");
        k.push(v);
        k
    }));
    cmd.push(exe.as_os_str().to_owned());
    cmd.extend(args.iter().cloned());
    cmd
}

fn have_sudo() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("sudo").is_file()))
}

/// Return if root, otherwise replace this process with a sudo'd copy of
/// itself. Exits with a message when that is not possible.
pub fn require_root() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    if have_sudo() {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nun-installer"));
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        let cmd = escalation_command(&exe, &args, std::env::vars_os());
        // exec never returns on success; the new process keeps our terminal.
        let err = std::process::Command::new(&cmd[0]).args(&cmd[1..]).exec();
        eprintln!("failed to re-run through sudo: {err}");
    }
    eprintln!("nun-installer must run as root (try `sudo nun-installer`).");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_bytes_in_unrelated_variables_are_dropped_not_a_panic() {
        use std::os::unix::ffi::OsStringExt;
        let env = [
            (OsString::from("LATIN1"), OsString::from_vec(vec![0xe9])),
            (OsString::from("PATH"), OsString::from("/bin")),
        ];
        let cmd = escalation_command(Path::new("/x"), &[OsString::from_vec(vec![0xff])], env);
        assert_eq!(cmd[2], "PATH=/bin");
        assert_eq!(cmd.len(), 5);
    }

    #[test]
    fn passes_only_the_environment_the_installer_needs() {
        let env = [
            ("NUN_STATE_VERSION", "26.11"),
            ("PATH", "/bin"),
            ("TERM", "linux"),
            ("HOME", "/home/x"),
            ("SECRET_TOKEN", "x"),
            ("NUN_NIXPKGS_PATH", "/nix/store/p"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let cmd = escalation_command(
            Path::new("/bin/nun-installer"),
            &["--config".into(), "a.json".into(), "--yes".into()],
            env,
        );
        assert_eq!(
            cmd,
            [
                "sudo",
                "env",
                "NUN_NIXPKGS_PATH=/nix/store/p",
                "NUN_STATE_VERSION=26.11",
                "PATH=/bin",
                "TERM=linux",
                "/bin/nun-installer",
                "--config",
                "a.json",
                "--yes",
            ]
        );
    }
}
