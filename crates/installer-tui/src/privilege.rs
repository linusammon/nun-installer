//! Installing needs root. When started as a normal user (for example with
//! `nix run` on the official ISO, where you are `nixos`), re-run through sudo
//! before the terminal is touched, so sudo's password prompt is plain I/O.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// Environment the installer relies on. sudo resets the environment, so these
/// are passed through explicitly with `env`.
fn keep(name: &str) -> bool {
    name.starts_with("NUN_") || name == "PATH" || name == "TERM"
}

/// `sudo env KEY=VALUE... <exe> <args...>`
pub fn escalation_command(
    exe: &Path,
    args: &[String],
    env: impl IntoIterator<Item = (String, String)>,
) -> Vec<String> {
    let mut cmd = vec!["sudo".to_owned(), "env".to_owned()];
    let mut vars: Vec<_> = env.into_iter().filter(|(k, _)| keep(k)).collect();
    vars.sort();
    cmd.extend(vars.into_iter().map(|(k, v)| format!("{k}={v}")));
    cmd.push(exe.display().to_string());
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
        let args: Vec<String> = std::env::args().skip(1).collect();
        let cmd = escalation_command(&exe, &args, std::env::vars());
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
    fn passes_only_the_environment_the_installer_needs() {
        let env = [
            ("NUN_STATE_VERSION", "26.11"),
            ("PATH", "/bin"),
            ("TERM", "linux"),
            ("HOME", "/home/x"),
            ("SECRET_TOKEN", "x"),
            ("NUN_NIXPKGS_PATH", "/nix/store/p"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
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
