//! The install plan: an ordered list of actions as plain data. Running it is
//! the executor's job, so the same plan can be printed (dry-run), executed, or
//! recorded in tests.

use crate::answers::Answers;
use crate::generate::{GeneratedFiles, Host, DISKO_URL, LUKS_PASSWORD_PATH, NIXPKGS_URL};
use crate::secret::Secret;
use std::fmt;
use std::path::PathBuf;

pub const MOUNT_ROOT: &str = "/mnt";
/// Config dir inside the target system.
pub const CONFIG_DIR: &str = "/mnt/etc/nixos";
/// Where the generated files are staged so disko can run before `/mnt` exists.
pub const STAGING_DIR: &str = "/run/nun-installer";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// Extra environment variables for the process.
    pub env: Vec<(String, String)>,
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
            env: Vec::new(),
            stdin: None,
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), value.into()));
        self
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
    /// Creates a directory (and parents) and sets its mode.
    CreateDir {
        path: PathBuf,
        mode: u32,
    },
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
                write!(f, "$ ")?;
                for (k, v) in &c.env {
                    write!(f, "{k}={v} ")?;
                }
                write!(f, "{}", c.program)?;
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
            ActionKind::CreateDir { path, mode } => {
                writeln!(f, "mkdir -m {mode:o} {}", path.display())
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
/// Why a plan could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("encryption is on but there is no passphrase")]
    MissingPassphrase,
}

pub fn build_plan(a: &Answers, files: &GeneratedFiles, host: &Host) -> Result<Plan, PlanError> {
    let mut actions = Vec::new();
    let staged_disko = format!("{STAGING_DIR}/disko.nix");

    actions.push(action(
        "Create private staging directory",
        ActionKind::CreateDir {
            path: STAGING_DIR.into(),
            mode: 0o700,
        },
    ));

    if a.disk.encrypt {
        // An empty passphrase would silently encrypt the disk with nothing, so a
        // missing one is an error even though validation already catches it.
        let password = a
            .disk
            .encryption_password
            .clone()
            .ok_or(PlanError::MissingPassphrase)?;
        actions.push(action(
            "Stage LUKS passphrase for disko",
            ActionKind::WriteFile {
                path: LUKS_PASSWORD_PATH.into(),
                content: Content::Secret(password),
                mode: 0o600,
            },
        ));
    }

    actions.push(write(
        "Stage disk layout",
        staged_disko.clone(),
        &files.disko_nix,
    ));
    // disko evaluates `<nixpkgs>`, so point it at ours instead of relying on the
    // live system having a channel or NIX_PATH.
    let mut disko = Command::new(
        "disko",
        [
            "--mode",
            "destroy,format,mount",
            "--yes-wipe-all-disks",
            "--root-mountpoint",
            MOUNT_ROOT,
            &staged_disko,
        ],
    );
    if let Some(path) = &host.nixpkgs_path {
        disko = disko.with_env("NIX_PATH", &format!("nixpkgs={}", path.display()));
    }
    actions.push(action(
        &format!("Partition, format and mount {}", a.disk.device),
        ActionKind::Run(disko),
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
    // Pin nixpkgs and disko to what this installer was built with. Only valid
    // while the generated flake uses the default URLs the pins were made for.
    if let Some(lock) = &host.pinned_lock {
        if host.nixpkgs_url == NIXPKGS_URL && host.disko_url == DISKO_URL {
            actions.push(write(
                "Write flake.lock (pinned inputs)",
                format!("{CONFIG_DIR}/flake.lock"),
                lock,
            ));
        }
    }
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
                Secret::new(format!(
                    "{}:{}\n",
                    a.user.name,
                    a.user.password_hash.expose()
                )),
            ),
        ),
    ));

    actions.push(Action {
        description: "Remove staged files".into(),
        kind: ActionKind::Remove(STAGING_DIR.into()),
        cleanup: true,
    });
    // The passphrase file lives inside the staging directory, so this removes it too.
    Ok(Plan { actions })
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
            nixpkgs_path: Some("/nix/store/fake-nixpkgs".into()),
            pinned_lock: None,
        };
        build_plan(a, &render(a, &host).unwrap(), &host).unwrap()
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
                "Create private staging directory",
                "Stage disk layout",
                "Partition, format and mount /dev/vda",
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
    fn encrypted_plan_stages_the_passphrase_inside_the_directory_that_cleanup_removes() {
        let p = plan_for(&answers(true));
        assert_eq!(p.actions[1].description, "Stage LUKS passphrase for disko");
        let last = p.actions.last().unwrap();
        assert!(last.cleanup);
        assert_eq!(last.kind, ActionKind::Remove(STAGING_DIR.into()));
        assert!(
            LUKS_PASSWORD_PATH.starts_with(STAGING_DIR),
            "the passphrase must be inside the directory that cleanup removes"
        );
        assert_eq!(
            p.actions.iter().filter(|a| a.cleanup).count(),
            1,
            "one cleanup action covers everything"
        );
    }

    #[test]
    fn encryption_without_a_passphrase_is_an_error_not_an_empty_passphrase() {
        let mut a = answers(true);
        a.disk.encryption_password = None;
        let host = Host {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: NIXPKGS_URL.into(),
            disko_url: DISKO_URL.into(),
            nixpkgs_path: None,
            pinned_lock: None,
        };
        let files = render(&a, &host).unwrap();
        assert_eq!(
            build_plan(&a, &files, &host).unwrap_err(),
            PlanError::MissingPassphrase
        );
    }

    #[test]
    fn hardware_config_is_generated_before_our_config_is_written() {
        let p = plan_for(&answers(false));
        let pos = |d: &str| p.actions.iter().position(|a| a.description == d).unwrap();
        assert!(pos("Generate hardware configuration") < pos("Write configuration.nix"));
        assert!(pos("Write disko.nix") < pos("Lock flake inputs"));
        assert!(pos("Lock flake inputs") < pos("Install NixOS"));
        assert!(pos("Partition, format and mount /dev/vda") < pos("Install NixOS"));
    }

    fn pinned_plan(mutate: impl FnOnce(&mut Host)) -> Plan {
        let a = answers(false);
        let mut host = Host {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: NIXPKGS_URL.into(),
            disko_url: DISKO_URL.into(),
            nixpkgs_path: None,
            pinned_lock: Some("{\"pinned\": true}\n".into()),
        };
        mutate(&mut host);
        build_plan(&a, &render(&a, &host).unwrap(), &host).unwrap()
    }

    #[test]
    fn pins_the_inputs_before_locking_when_a_lock_is_available() {
        let p = pinned_plan(|_| {});
        let pos = |d: &str| p.actions.iter().position(|a| a.description == d).unwrap();
        let i = pos("Write flake.lock (pinned inputs)");
        assert!(pos("Write flake.nix") < i, "needs the flake it pins");
        assert!(i < pos("Lock flake inputs"));
        let ActionKind::WriteFile {
            path,
            content,
            mode,
        } = &p.actions[i].kind
        else {
            panic!("expected a file write");
        };
        assert_eq!(path, std::path::Path::new("/mnt/etc/nixos/flake.lock"));
        assert_eq!(*content, Content::Plain("{\"pinned\": true}\n".into()));
        assert_eq!(*mode, 0o644);
    }

    #[test]
    fn no_pin_without_a_lock() {
        let p = pinned_plan(|h| h.pinned_lock = None);
        assert!(!descriptions(&p).iter().any(|d| d.contains("flake.lock")));
    }

    #[test]
    fn no_pin_when_the_input_urls_are_overridden() {
        for change in [
            (|h: &mut Host| h.nixpkgs_url = "path:/nix/store/x".into()) as fn(&mut Host),
            |h: &mut Host| h.disko_url = "path:/nix/store/y".into(),
        ] {
            let p = pinned_plan(change);
            assert!(
                !descriptions(&p).iter().any(|d| d.contains("flake.lock")),
                "the pins were made for the default URLs"
            );
        }
    }

    #[test]
    fn staging_dir_is_private_and_first() {
        let p = plan_for(&answers(true));
        assert_eq!(
            p.actions[0].kind,
            ActionKind::CreateDir {
                path: STAGING_DIR.into(),
                mode: 0o700
            }
        );
        assert!(LUKS_PASSWORD_PATH.starts_with(STAGING_DIR));
    }

    #[test]
    fn disko_gets_nix_path_from_host() {
        let p = plan_for(&answers(false));
        let ActionKind::Run(c) = &p.actions[2].kind else {
            panic!("expected command");
        };
        assert_eq!(c.program, "disko");
        assert_eq!(
            c.env,
            [(
                "NIX_PATH".to_owned(),
                "nixpkgs=/nix/store/fake-nixpkgs".to_owned()
            )]
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
        let ActionKind::Run(c) = &p.actions[9].kind else {
            panic!("expected command");
        };
        assert_eq!(c.program, "nixos-enter");
        assert!(c.args.iter().all(|a| !a.contains("$y$")));
        assert_eq!(c.stdin.as_ref().unwrap().expose(), "alice:$y$j9T$abc$def\n");
    }
}
