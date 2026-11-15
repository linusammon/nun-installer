//! The install plan: an ordered list of actions as plain data. Running it is
//! the executor's job, so the same plan can be printed (dry-run), executed, or
//! recorded in tests.

use crate::answers::Answers;
use crate::generate::{GeneratedFiles, LUKS_PASSWORD_PATH};
use std::fmt;
use std::path::PathBuf;

pub const MOUNT_ROOT: &str = "/mnt";
/// Config dir inside the target system.
pub const CONFIG_DIR: &str = "/mnt/etc/nixos";
/// Where the generated files are staged so disko can run before `/mnt` exists.
pub const STAGING_DIR: &str = "/tmp/nun-installer";

/// A string that never shows up in `Debug` or `Display` output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// Written to the process's stdin, if set.
    pub stdin: Option<Secret>,
}

impl Command {
    pub fn new<I, S>(program: &str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            stdin: None,
        }
    }

    pub fn with_stdin(mut self, stdin: Secret) -> Self {
        self.stdin = Some(stdin);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Plain(String),
    Secret(Secret),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    Run(Command),
    WriteFile {
        path: PathBuf,
        content: Content,
        mode: u32,
    },
    /// Removes a file or directory tree. A missing path is not an error.
    Remove(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub description: String,
    pub kind: ActionKind,
    /// Cleanup actions run even if an earlier action failed.
    pub cleanup: bool,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "# {}", self.description)?;
        match &self.kind {
            ActionKind::Run(c) => {
                write!(f, "$ {}", c.program)?;
                for a in &c.args {
                    write!(f, " {a}")?;
                }
                if c.stdin.is_some() {
                    write!(f, "   (stdin: <redacted>)")?;
                }
                writeln!(f)
            }
            ActionKind::WriteFile {
                path,
                content,
                mode,
            } => {
                writeln!(f, "write {} (mode {mode:o})", path.display())?;
                match content {
                    Content::Plain(s) => {
                        for line in s.lines() {
                            writeln!(f, "  | {line}")?;
                        }
                        Ok(())
                    }
                    Content::Secret(_) => writeln!(f, "  | <redacted>"),
                }
            }
            ActionKind::Remove(p) => writeln!(f, "rm -rf {}", p.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub actions: Vec<Action>,
}

fn action(description: &str, kind: ActionKind) -> Action {
    Action {
        description: description.into(),
        kind,
        cleanup: false,
    }
}

fn write(description: &str, path: String, content: &str) -> Action {
    action(
        description,
        ActionKind::WriteFile {
            path: path.into(),
            content: Content::Plain(content.into()),
            mode: 0o644,
        },
    )
}

/// Build the full install plan. Answers must already be validated and
/// the files rendered from them.
pub fn build_plan(a: &Answers, files: &GeneratedFiles) -> Plan {
    let mut actions = Vec::new();
    let staged_disko = format!("{STAGING_DIR}/disko.nix");

    if a.disk.encrypt {
        let password = a.disk.encryption_password.clone().unwrap_or_default();
        actions.push(action(
            "Stage LUKS passphrase for disko",
            ActionKind::WriteFile {
                path: LUKS_PASSWORD_PATH.into(),
                content: Content::Secret(Secret::new(password)),
                mode: 0o600,
            },
        ));
    }

    actions.push(write(
        "Stage disk layout",
        staged_disko.clone(),
        &files.disko_nix,
    ));
    actions.push(action(
        &format!(
            "Partition, format and mount {} (WIPES THE DISK)",
            a.disk.device
        ),
        ActionKind::Run(Command::new(
            "disko",
            [
                "--mode",
                "destroy,format,mount",
                "--yes-wipe-all-disks",
                "--root-mountpoint",
                MOUNT_ROOT,
                &staged_disko,
            ],
        )),
    ));
    actions.push(action(
        "Generate hardware configuration",
        ActionKind::Run(Command::new(
            "nixos-generate-config",
            ["--root", MOUNT_ROOT, "--no-filesystems"],
        )),
    ));
    actions.push(write(
        "Write flake.nix",
        format!("{CONFIG_DIR}/flake.nix"),
        &files.flake_nix,
    ));
    actions.push(write(
        "Write configuration.nix",
        format!("{CONFIG_DIR}/configuration.nix"),
        &files.configuration_nix,
    ));
    actions.push(write(
        "Write disko.nix",
        format!("{CONFIG_DIR}/disko.nix"),
        &files.disko_nix,
    ));
    actions.push(action(
        "Lock flake inputs",
        ActionKind::Run(Command::new(
            "nix",
            [
                "--extra-experimental-features",
                "nix-command flakes",
                "flake",
                "lock",
                CONFIG_DIR,
            ],
        )),
    ));
    actions.push(action(
        "Install NixOS",
        ActionKind::Run(Command::new(
            "nixos-install",
            [
                "--root",
                MOUNT_ROOT,
                "--no-root-passwd",
                "--flake",
                &format!("{CONFIG_DIR}#{}", a.system.hostname),
            ],
        )),
    ));
    actions.push(action(
        &format!("Set password for {}", a.user.name),
        ActionKind::Run(
            Command::new("nixos-enter", ["--root", MOUNT_ROOT, "-c", "chpasswd -e"]).with_stdin(
                Secret::new(format!("{}:{}\n", a.user.name, a.user.password_hash)),
            ),
        ),
    ));

    actions.push(Action {
        description: "Remove staged files".into(),
        kind: ActionKind::Remove(STAGING_DIR.into()),
        cleanup: true,
    });
    if a.disk.encrypt {
        actions.push(Action {
            description: "Remove staged LUKS passphrase".into(),
            kind: ActionKind::Remove(LUKS_PASSWORD_PATH.into()),
            cleanup: true,
        });
    }
    Plan { actions }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, a) in self.actions.iter().enumerate() {
            writeln!(f, "[{}/{}] {a}", i + 1, self.actions.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answers::*;
    use crate::generate::{render, Boot, Host};

    fn answers(encrypt: bool) -> Answers {
        Answers {
            schema_version: SCHEMA_VERSION,
            disk: Disk {
                device: "/dev/vda".into(),
                swap_size_mib: 2048,
                encrypt,
                encryption_password: encrypt.then(|| "correct horse battery".into()),
            },
            user: User {
                name: "alice".into(),
                password_hash: "$y$j9T$abc$def".into(),
            },
            system: System {
                hostname: "nun".into(),
                locale: "en_US.UTF-8".into(),
                timezone: "Europe/Berlin".into(),
                keyboard_layout: "us".into(),
                keyboard_variant: None,
            },
        }
    }

    fn plan_for(a: &Answers) -> Plan {
        let host = Host {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: crate::generate::NIXPKGS_URL.into(),
            disko_url: crate::generate::DISKO_URL.into(),
        };
        build_plan(a, &render(a, &host).unwrap())
    }

    fn descriptions(p: &Plan) -> Vec<&str> {
        p.actions.iter().map(|a| a.description.as_str()).collect()
    }

    #[test]
    fn plain_plan_order() {
        let p = plan_for(&answers(false));
        assert_eq!(
            descriptions(&p),
            [
                "Stage disk layout",
                "Partition, format and mount /dev/vda (WIPES THE DISK)",
                "Generate hardware configuration",
                "Write flake.nix",
                "Write configuration.nix",
                "Write disko.nix",
                "Lock flake inputs",
                "Install NixOS",
                "Set password for alice",
                "Remove staged files",
            ]
        );
    }

    #[test]
    fn encrypted_plan_stages_and_removes_passphrase() {
        let p = plan_for(&answers(true));
        assert_eq!(p.actions[0].description, "Stage LUKS passphrase for disko");
        let last = p.actions.last().unwrap();
        assert!(last.cleanup);
        assert_eq!(last.kind, ActionKind::Remove(LUKS_PASSWORD_PATH.into()));
    }

    #[test]
    fn hardware_config_is_generated_before_our_config_is_written() {
        let p = plan_for(&answers(false));
        let pos = |d: &str| p.actions.iter().position(|a| a.description == d).unwrap();
        assert!(pos("Generate hardware configuration") < pos("Write configuration.nix"));
        assert!(pos("Write disko.nix") < pos("Lock flake inputs"));
        assert!(pos("Lock flake inputs") < pos("Install NixOS"));
        assert!(
            pos("Partition, format and mount /dev/vda (WIPES THE DISK)") < pos("Install NixOS")
        );
    }

    #[test]
    fn secrets_never_printed() {
        let text = plan_for(&answers(true)).to_string();
        assert!(!text.contains("correct horse"));
        assert!(!text.contains("$y$j9T$abc$def"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn password_goes_through_stdin_not_args() {
        let p = plan_for(&answers(false));
        let ActionKind::Run(c) = &p.actions[8].kind else {
            panic!("expected command");
        };
        assert_eq!(c.program, "nixos-enter");
        assert!(c.args.iter().all(|a| !a.contains("$y$")));
        assert_eq!(c.stdin.as_ref().unwrap().expose(), "alice:$y$j9T$abc$def\n");
    }
}
