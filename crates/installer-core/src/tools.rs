//! The programs an install shells out to. Checked before anything is wiped, so
//! a missing one is reported up front instead of halfway through.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub const REQUIRED_TOOLS: &[&str] = &[
    "disko",
    "nix",
    "nixos-generate-config",
    "nixos-install",
    "nixos-enter",
];

fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Tools from [`REQUIRED_TOOLS`] not found as executables in `path`
/// (a `PATH`-style list of directories).
pub fn missing_tools_in(path: &OsStr) -> Vec<String> {
    let dirs: Vec<_> = std::env::split_paths(path).collect();
    REQUIRED_TOOLS
        .iter()
        .filter(|tool| !dirs.iter().any(|d| is_executable(&d.join(tool))))
        .map(|t| (*t).to_owned())
        .collect()
}

/// Missing tools according to the current `PATH`.
pub fn missing_tools() -> Vec<String> {
    match std::env::var_os("PATH") {
        Some(path) => missing_tools_in(&path),
        None => REQUIRED_TOOLS.iter().map(|t| (*t).to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make(d: &Path, name: &str, mode: u32) {
        let p = d.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn reports_only_what_is_missing() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        make(a.path(), "disko", 0o755);
        make(a.path(), "nix", 0o755);
        make(b.path(), "nixos-install", 0o755);
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(
            missing_tools_in(&path),
            ["nixos-generate-config", "nixos-enter"]
        );
    }

    #[test]
    fn a_non_executable_file_does_not_count() {
        let d = TempDir::new().unwrap();
        make(d.path(), "disko", 0o644);
        assert!(missing_tools_in(d.path().as_os_str()).contains(&"disko".to_owned()));
    }

    #[test]
    fn everything_is_missing_from_an_empty_path() {
        assert_eq!(missing_tools_in(OsStr::new("")).len(), REQUIRED_TOOLS.len());
    }
}
