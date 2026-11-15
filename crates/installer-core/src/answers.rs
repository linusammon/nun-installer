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
    #[serde(default)]
    pub nun: Nun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    /// Block device to wipe, e.g. `/dev/nvme0n1`.
    pub device: String,
    #[serde(default)]
    pub filesystem: Filesystem,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filesystem {
    #[default]
    Ext4,
    Btrfs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub name: String,
    /// Crypt-style hash (yescrypt, sha512, ...). Plaintext is never stored.
    pub password_hash: String,
    #[serde(default = "default_true")]
    pub sudo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub hostname: String,
    /// e.g. `en_US.UTF-8`.
    pub locale: String,
    /// e.g. `Europe/Berlin`.
    pub timezone: String,
    /// XKB layout, e.g. `us` or `de`.
    pub keyboard_layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nun {
    #[serde(default = "default_true")]
    pub noctalia: bool,
    #[serde(default = "default_true")]
    pub greeter: bool,
    #[serde(default = "default_true")]
    pub umbriel: bool,
}

impl Default for Nun {
    fn default() -> Self {
        Self {
            noctalia: true,
            greeter: true,
            umbriel: true,
        }
    }
}

fn default_true() -> bool {
    true
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
                "keyboard_layout": "de"
            }
        }"#;
        let a: Answers = serde_json::from_str(json).unwrap();
        assert_eq!(a.disk.filesystem, Filesystem::Ext4);
        assert!(a.user.sudo);
        assert_eq!(a.nun, Nun::default());
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"{ "schema_version": 1, "bogus": 1 }"#;
        assert!(serde_json::from_str::<Answers>(json).is_err());
    }
}
