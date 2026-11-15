use serde::{Deserialize, Serialize};

/// Current version of the answers file format.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answers {
    pub schema_version: u32,
    pub disk: Disk,
    pub user: User,
    pub system: System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    /// Block device to wipe, e.g. `/dev/nvme0n1`. Layout is always GPT + btrfs.
    pub device: String,
    /// Swapfile size in MiB, created by disko on the btrfs volume. `0` disables swap.
    #[serde(default)]
    pub swap_size_mib: u32,
    #[serde(default)]
    pub encrypt: bool,
    /// LUKS passphrase. Required if and only if `encrypt` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_password: Option<String>,
}

/// The user is always in `wheel`; sudo uses the user password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub name: String,
    /// Crypt-style hash (yescrypt, sha512, ...). Applied with `chpasswd -e` after
    /// install and never written to the generated config.
    pub password_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub hostname: String,
    /// e.g. `en_US.UTF-8`.
    pub locale: String,
    /// e.g. `Europe/Berlin`.
    pub timezone: String,
    /// XKB layout, e.g. `us`, `de` or `ch`.
    pub keyboard_layout: String,
    /// XKB variant, e.g. `fr` for Swiss French (`ch` + `fr`). Must exist for the layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_variant: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_json_with_defaults() {
        let json = r#"{
            "schema_version": 1,
            "disk": { "device": "/dev/vda" },
            "user": { "name": "alice", "password_hash": "$y$j9T$abc$def" },
            "system": {
                "hostname": "nun",
                "locale": "en_US.UTF-8",
                "timezone": "Europe/Berlin",
                "keyboard_layout": "ch",
                "keyboard_variant": "fr"
            }
        }"#;
        let a: Answers = serde_json::from_str(json).unwrap();

        assert_eq!(a.disk.swap_size_mib, 0);
        assert!(!a.disk.encrypt);
        assert_eq!(a.system.keyboard_variant.as_deref(), Some("fr"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"{ "schema_version": 1, "bogus": 1 }"#;
        assert!(serde_json::from_str::<Answers>(json).is_err());
    }
}
