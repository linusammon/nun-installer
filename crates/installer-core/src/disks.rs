//! Finding disks the user can pick as the install target.

use crate::safety::{check_disk, SafetyError};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    /// e.g. `/dev/nvme0n1`.
    pub path: PathBuf,
    pub name: String,
    pub size_bytes: u64,
    /// May be empty (virtual disks).
    pub model: String,
    pub removable: bool,
    /// Why this disk must not be used. Empty means it is selectable.
    pub rejected: Vec<SafetyError>,
}

impl DiskInfo {
    pub fn selectable(&self) -> bool {
        self.rejected.is_empty()
    }
}

/// Virtual and optical devices that are never an install target.
const IGNORED_PREFIXES: &[&str] = &["loop", "ram", "zram", "sr", "fd", "md", "dm-", "nbd"];

/// Real disks on this machine, each with the reasons it can't be used, if any.
pub fn list_disks() -> Vec<DiskInfo> {
    let mut disks = scan(Path::new("/sys/block"));
    for d in &mut disks {
        d.rejected = check_disk(&d.path);
    }
    disks
}

/// Read disks from a sysfs `block` directory. Does not check safety.
pub fn scan(sys_block: &Path) -> Vec<DiskInfo> {
    let Ok(entries) = fs::read_dir(sys_block) else {
        return Vec::new();
    };
    let mut disks: Vec<DiskInfo> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if IGNORED_PREFIXES.iter().any(|p| name.starts_with(p)) {
                return None;
            }
            let dir = e.path();
            let sectors: u64 = read(&dir.join("size"))?.parse().ok()?;
            if sectors == 0 {
                return None;
            }
            Some(DiskInfo {
                path: Path::new("/dev").join(&name),
                size_bytes: sectors * 512,
                model: read(&dir.join("device/model")).unwrap_or_default(),
                removable: read(&dir.join("removable")).as_deref() == Some("1"),
                rejected: Vec::new(),
                name,
            })
        })
        .collect();
    disks.sort_by(|a, b| a.name.cmp(&b.name));
    disks
}

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_owned())
}

/// `476.9 GiB`, `1.0 TiB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_sys(root: &Path, name: &str, sectors: u64, model: Option<&str>, removable: bool) {
        let d = root.join(name);
        fs::create_dir_all(d.join("device")).unwrap();
        fs::write(d.join("size"), format!("{sectors}\n")).unwrap();
        fs::write(d.join("removable"), if removable { "1\n" } else { "0\n" }).unwrap();
        if let Some(m) = model {
            fs::write(d.join("device/model"), format!("{m}   \n")).unwrap();
        }
    }

    #[test]
    fn scans_real_disks_and_skips_virtual_ones() {
        let root = std::env::temp_dir().join(format!("nun-sys-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fake_sys(
            &root,
            "nvme0n1",
            1_000_215_216,
            Some("Samsung SSD 970"),
            false,
        );
        fake_sys(&root, "sda", 61_000_000, Some("USB Stick"), true);
        fake_sys(&root, "vda", 2048, None, false);
        fake_sys(&root, "loop0", 4096, None, false);
        fake_sys(&root, "zram0", 4096, None, false);
        fake_sys(&root, "sr0", 4096, None, true);
        fake_sys(&root, "sdb", 0, None, false);

        let disks = scan(&root);
        fs::remove_dir_all(&root).unwrap();

        let names: Vec<_> = disks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["nvme0n1", "sda", "vda"]);
        assert_eq!(disks[0].path, Path::new("/dev/nvme0n1"));
        assert_eq!(disks[0].model, "Samsung SSD 970");
        assert_eq!(disks[0].size_bytes, 1_000_215_216 * 512);
        assert!(disks[1].removable);
        assert_eq!(disks[2].model, "");
        assert!(disks.iter().all(DiskInfo::selectable));
    }

    #[test]
    fn missing_sysfs_is_empty() {
        assert!(scan(Path::new("/definitely/not/here")).is_empty());
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GiB");
        assert_eq!(human_size(512_110_190_592), "476.9 GiB");
        assert_eq!(human_size(2 * 1024u64.pow(4)), "2.0 TiB");
    }
}
