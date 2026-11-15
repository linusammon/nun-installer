//! Example values for tests. Compiled for this crate's own tests and, with the
//! `test-util` feature, for the tests of crates that depend on it. They live in
//! one place so that adding a field to `Host` or `Answers` is one edit here and
//! not one per test module.

use crate::generate::{DISKO_URL, NIXPKGS_URL};
use crate::secret::Secret;
use crate::{Answers, Boot, Catalog, Disk, DiskInfo, Host, System, User, SCHEMA_VERSION};

impl Host {
    /// A UEFI x86_64 machine with the default inputs and no pinned lock.
    pub fn example() -> Self {
        Self {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: NIXPKGS_URL.into(),
            disko_url: DISKO_URL.into(),
            nixpkgs_path: None,
            pinned_lock: None,
        }
    }
}

impl Answers {
    /// Valid against [`Catalog::example`]: plain disk, no swap, no encryption.
    pub fn example() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            disk: Disk {
                device: "/dev/nvme0n1".into(),
                swap_size_mib: 0,
                encrypt: false,
                encryption_password: None,
            },
            user: User {
                name: "alice".into(),
                password_hash: Secret::new("$y$j9T$abc$def"),
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
}

impl Catalog {
    /// Two timezones, two locales, layouts `us` (no variants) and `ch` (variant `fr`).
    pub fn example() -> Self {
        Self::from_sources(
            "DE\t+5230+01322\tEurope/Berlin\nCH\t+4723+00832\tEurope/Zurich\n",
            "SUPPORTED-LOCALES=\\\nde_CH.UTF-8/UTF-8 \\\nen_US.UTF-8/UTF-8 \\\n",
            "! layout\n  us  English (US)\n  ch  German (Switzerland)\n\n! variant\n  fr  ch: French (Switzerland)\n",
        )
    }
}

impl DiskInfo {
    /// A 20 GiB disk called `name` that can be used.
    pub fn example(name: &str) -> Self {
        Self {
            path: format!("/dev/{name}").into(),
            name: name.into(),
            size_bytes: 20 << 30,
            model: "Test Disk".into(),
            removable: false,
            rejected: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate;

    #[test]
    fn the_examples_fit_together() {
        let errors = validate(&Answers::example(), &Catalog::example());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(DiskInfo::example("vda").selectable());
        assert_eq!(Host::example().boot, Boot::Uefi);
    }
}
