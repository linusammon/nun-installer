use crate::answers::Answers;
use minijinja::{context, Environment, Value};

/// Default nixpkgs ref written into the generated flake.
pub const NIXPKGS_URL: &str = "github:NixOS/nixpkgs/nixos-unstable";

/// Default disko ref written into the generated flake.
pub const DISKO_URL: &str = "github:nix-community/disko";

/// Where the LUKS passphrase is staged for disko. Removed again after install.
pub const LUKS_PASSWORD_PATH: &str = "/tmp/nun-luks-password";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    Uefi,
    Bios,
}

/// Facts about the machine and release being installed, detected at runtime
/// rather than asked from the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub boot: Boot,
    /// Nix system double, e.g. `x86_64-linux`.
    pub system: String,
    /// `system.stateVersion`, e.g. `26.05`.
    pub state_version: String,
    /// Flake input URLs for the generated flake.
    pub nixpkgs_url: String,
    pub disko_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("unsupported architecture: {0}")]
    UnsupportedArch(String),
    #[error("NUN_STATE_VERSION is not set (the Nix package sets it to the nixpkgs release)")]
    MissingStateVersion,
}

impl Host {
    /// Detect boot mode and architecture of the running machine. The state
    /// version comes from `NUN_STATE_VERSION`, set by the Nix package. Input
    /// URLs can be overridden with `NUN_NIXPKGS_URL` and `NUN_DISKO_URL` (tests,
    /// mirrors, offline media).
    pub fn detect() -> Result<Self, HostError> {
        let system = match std::env::consts::ARCH {
            "x86_64" => "x86_64-linux",
            "aarch64" => "aarch64-linux",
            other => return Err(HostError::UnsupportedArch(other.into())),
        };
        let state_version = std::env::var("NUN_STATE_VERSION")
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or(HostError::MissingStateVersion)?;
        let url = |var: &str, default: &str| {
            std::env::var(var)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.into())
        };
        let boot = if std::path::Path::new("/sys/firmware/efi").exists() {
            Boot::Uefi
        } else {
            Boot::Bios
        };
        Ok(Self {
            boot,
            system: system.into(),
            state_version,
            nixpkgs_url: url("NUN_NIXPKGS_URL", NIXPKGS_URL),
            disko_url: url("NUN_DISKO_URL", DISKO_URL),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFiles {
    pub flake_nix: String,
    pub configuration_nix: String,
    pub disko_nix: String,
}

#[derive(Debug, thiserror::Error)]
#[error("template error: {0}")]
pub struct RenderError(#[from] minijinja::Error);

/// Escape a value as a double-quoted Nix string literal.
fn nix_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    env.add_filter("nix", |v: Value| nix_string(&v.to_string()));
    env
}

/// Render `flake.nix`, `configuration.nix` and `disko.nix`. Answers must already be validated.
pub fn render(a: &Answers, host: &Host) -> Result<GeneratedFiles, RenderError> {
    let env = env();
    let ctx = context! {
        nixpkgs_url => &host.nixpkgs_url,
        disko_url => &host.disko_url,
        luks_password_path => LUKS_PASSWORD_PATH,
        system => &host.system,
        state_version => &host.state_version,
        uefi => host.boot == Boot::Uefi,
        hostname => &a.system.hostname,
        locale => &a.system.locale,
        timezone => &a.system.timezone,
        keyboard_layout => &a.system.keyboard_layout,
        keyboard_variant => &a.system.keyboard_variant,
        user_name => &a.user.name,
        device => &a.disk.device,
        encrypt => a.disk.encrypt,
        swap_size_mib => a.disk.swap_size_mib,
    };
    let render = |name: &str, src: &str| env.render_named_str(name, src, &ctx);
    Ok(GeneratedFiles {
        flake_nix: render("flake.nix", include_str!("../templates/flake.nix.j2"))?,
        configuration_nix: render(
            "configuration.nix",
            include_str!("../templates/configuration.nix.j2"),
        )?,
        disko_nix: render("disko.nix", include_str!("../templates/disko.nix.j2"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answers::*;

    fn answers() -> Answers {
        Answers {
            schema_version: SCHEMA_VERSION,
            disk: Disk {
                device: "/dev/nvme0n1".into(),
                swap_size_mib: 0,
                encrypt: false,
                encryption_password: None,
            },
            user: User {
                name: "alice".into(),
                password_hash: "$y$j9T$abc$def".into(),
            },
            system: System {
                hostname: "nun-box".into(),
                locale: "en_US.UTF-8".into(),
                timezone: "Europe/Berlin".into(),
                keyboard_layout: "ch".into(),
                keyboard_variant: Some("fr".into()),
            },
        }
    }

    fn host(boot: Boot) -> Host {
        Host {
            boot,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: NIXPKGS_URL.into(),
            disko_url: DISKO_URL.into(),
        }
    }

    fn all(files: &GeneratedFiles) -> String {
        format!(
            "--- flake.nix\n{}\n--- configuration.nix\n{}\n--- disko.nix\n{}",
            files.flake_nix, files.configuration_nix, files.disko_nix
        )
    }

    #[test]
    fn escapes_nix_strings() {
        assert_eq!(nix_string(r#"a"b\c${d}$e"#), r#""a\"b\\c\${d}$e""#);
    }

    #[test]
    fn uefi_plain() {
        let out = render(&answers(), &host(Boot::Uefi)).unwrap();
        insta::assert_snapshot!(all(&out));
    }

    #[test]
    fn bios_encrypted_swap() {
        let mut a = answers();
        a.disk.encrypt = true;
        a.disk.encryption_password = Some("long-enough-pass".into());
        a.disk.swap_size_mib = 4096;
        a.system.keyboard_layout = "us".into();
        a.system.keyboard_variant = None;
        let out = render(&a, &host(Boot::Bios)).unwrap();
        insta::assert_snapshot!(all(&out));
    }

    #[test]
    fn luks_password_never_in_output() {
        let mut a = answers();
        a.disk.encrypt = true;
        a.disk.encryption_password = Some("super-secret-pass".into());
        let out = render(&a, &host(Boot::Uefi)).unwrap();
        assert!(!all(&out).contains("super-secret-pass"));
    }
}
