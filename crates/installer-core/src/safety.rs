//! Pre-flight checks for the destructive part of the install. Gathering facts
//! from the system is separate from judging them, so the judging is pure.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SafetyError {
    #[error("{0} does not exist")]
    NotFound(PathBuf),
    #[error("{0} is not a block device")]
    NotBlockDevice(PathBuf),
    #[error("{0} is a partition, select the whole disk")]
    NotWholeDisk(PathBuf),
    #[error("{device} holds the running system's root filesystem")]
    HoldsRoot { device: PathBuf },
    #[error("{device} is the installer's boot media (mounted at {target})")]
    LiveMedia { device: PathBuf, target: PathBuf },
    #[error("{device} is mounted at {target}")]
    Mounted { device: PathBuf, target: PathBuf },
    #[error("{0} is in use as swap")]
    SwapActive(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Canonicalized when the source is a `/dev` path.
    pub source: PathBuf,
    pub target: PathBuf,
}

/// What we know about the target disk and the system around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// As given by the user.
    pub device: PathBuf,
    pub exists: bool,
    pub is_block: bool,
    pub is_whole_disk: bool,
    /// The disk, its partitions, and device-mapper devices stacked on them.
    pub related: BTreeSet<PathBuf>,
    pub mounts: Vec<Mount>,
    pub swaps: Vec<PathBuf>,
}

/// Where the NixOS installer ISO mounts its own medium.
const LIVE_MEDIA_TARGET: &str = "/iso";

pub fn check(f: &Facts) -> Vec<SafetyError> {
    if !f.exists {
        return vec![SafetyError::NotFound(f.device.clone())];
    }
    if !f.is_block {
        return vec![SafetyError::NotBlockDevice(f.device.clone())];
    }
    let mut errs = Vec::new();
    if !f.is_whole_disk {
        errs.push(SafetyError::NotWholeDisk(f.device.clone()));
    }
    // One error per device: the most telling mount wins (root, then live media, then the rest).
    let mut worst: BTreeMap<&PathBuf, (u8, SafetyError)> = BTreeMap::new();
    for m in f.mounts.iter().filter(|m| f.related.contains(&m.source)) {
        let device = m.source.clone();
        let target = m.target.clone();
        let (rank, err) = if target == Path::new("/") {
            (2, SafetyError::HoldsRoot { device })
        } else if target == Path::new(LIVE_MEDIA_TARGET) {
            (1, SafetyError::LiveMedia { device, target })
        } else {
            (0, SafetyError::Mounted { device, target })
        };
        match worst.get(&m.source) {
            Some((r, _)) if *r >= rank => {}
            _ => {
                worst.insert(&m.source, (rank, err));
            }
        }
    }
    let mut worst: Vec<_> = worst.into_values().collect();
    worst.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
    errs.extend(worst.into_iter().map(|(_, e)| e));
    for s in f.swaps.iter().filter(|s| f.related.contains(*s)) {
        errs.push(SafetyError::SwapActive(s.clone()));
    }
    errs
}

/// Gather facts about `device` and check them.
pub fn check_disk(device: &str) -> Vec<SafetyError> {
    check(&gather(Path::new(device)))
}

pub fn gather(device: &Path) -> Facts {
    let mut facts = Facts {
        device: device.to_path_buf(),
        exists: false,
        is_block: false,
        is_whole_disk: false,
        related: BTreeSet::new(),
        mounts: Vec::new(),
        swaps: Vec::new(),
    };
    let Ok(canonical) = fs::canonicalize(device) else {
        return facts;
    };
    facts.exists = true;
    let Ok(meta) = fs::metadata(&canonical) else {
        return facts;
    };
    facts.is_block = meta.file_type().is_block_device();
    let Some(name) = canonical.file_name().and_then(|n| n.to_str()) else {
        return facts;
    };
    if !facts.is_block {
        return facts;
    }
    facts.is_whole_disk = !Path::new("/sys/class/block")
        .join(name)
        .join("partition")
        .exists();
    facts.related = related_devices(name);
    facts.related.insert(canonical);
    facts.mounts = parse_mounts(&fs::read_to_string("/proc/self/mounts").unwrap_or_default())
        .into_iter()
        .map(|m| Mount {
            source: canonicalize_dev(m.source),
            target: m.target,
        })
        .collect();
    facts.swaps = parse_swaps(&fs::read_to_string("/proc/swaps").unwrap_or_default())
        .into_iter()
        .map(canonicalize_dev)
        .collect();
    facts
}

fn canonicalize_dev(p: PathBuf) -> PathBuf {
    if p.starts_with("/dev") {
        fs::canonicalize(&p).unwrap_or(p)
    } else {
        p
    }
}

/// The disk's partitions plus everything stacked on them (LUKS, LVM, ...),
/// found through sysfs `holders`.
fn related_devices(disk: &str) -> BTreeSet<PathBuf> {
    let sys = Path::new("/sys/class/block");
    let mut names = vec![disk.to_owned()];
    if let Ok(entries) = fs::read_dir(sys.join(disk)) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(disk) && e.path().join("partition").exists() {
                names.push(n);
            }
        }
    }
    let mut seen = BTreeSet::new();
    while let Some(n) = names.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Ok(holders) = fs::read_dir(sys.join(&n).join("holders")) {
            names.extend(
                holders
                    .flatten()
                    .map(|h| h.file_name().to_string_lossy().into_owned()),
            );
        }
    }
    seen.into_iter()
        .map(|n| Path::new("/dev").join(n))
        .collect()
}

fn parse_mounts(src: &str) -> Vec<Mount> {
    src.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let source = it.next()?;
            let target = it.next()?;
            Some(Mount {
                source: PathBuf::from(unescape(source)),
                target: PathBuf::from(unescape(target)),
            })
        })
        .collect()
}

fn parse_swaps(src: &str) -> Vec<PathBuf> {
    src.lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .map(|s| PathBuf::from(unescape(s)))
        .collect()
}

/// Decode the `\040`-style octal escapes used in `/proc/self/mounts`.
fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..=i + 3]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let digit = |k: usize| u32::from(bytes[i + k] - b'0');
            let v = digit(1) * 64 + digit(2) * 8 + digit(3);
            // The kernel only emits \000 to \377; anything bigger is not an escape.
            if let Ok(byte) = u8::try_from(v) {
                out.push(byte);
                i += 4;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn facts() -> Facts {
        Facts {
            device: p("/dev/sdb"),
            exists: true,
            is_block: true,
            is_whole_disk: true,
            related: [p("/dev/sdb"), p("/dev/sdb1"), p("/dev/dm-0")].into(),
            mounts: vec![Mount {
                source: p("/dev/sda1"),
                target: p("/"),
            }],
            swaps: vec![],
        }
    }

    #[test]
    fn clean_disk_passes() {
        assert!(check(&facts()).is_empty());
    }

    #[test]
    fn missing_and_non_block() {
        let mut f = facts();
        f.exists = false;
        assert_eq!(check(&f), [SafetyError::NotFound(p("/dev/sdb"))]);
        let mut f = facts();
        f.is_block = false;
        assert_eq!(check(&f), [SafetyError::NotBlockDevice(p("/dev/sdb"))]);
    }

    #[test]
    fn partition_rejected() {
        let mut f = facts();
        f.is_whole_disk = false;
        assert_eq!(check(&f), [SafetyError::NotWholeDisk(p("/dev/sdb"))]);
    }

    #[test]
    fn mounted_partition_or_stacked_device() {
        let mut f = facts();
        f.mounts.push(Mount {
            source: p("/dev/sdb1"),
            target: p("/data"),
        });
        f.mounts.push(Mount {
            source: p("/dev/dm-0"),
            target: p("/home"),
        });
        assert_eq!(
            check(&f),
            [
                SafetyError::Mounted {
                    device: p("/dev/dm-0"),
                    target: p("/home")
                },
                SafetyError::Mounted {
                    device: p("/dev/sdb1"),
                    target: p("/data")
                },
            ]
        );
    }

    #[test]
    fn one_error_per_device_worst_wins() {
        let mut f = facts();
        f.mounts = vec![
            Mount {
                source: p("/dev/dm-0"),
                target: p("/nix"),
            },
            Mount {
                source: p("/dev/dm-0"),
                target: p("/"),
            },
            Mount {
                source: p("/dev/dm-0"),
                target: p("/home"),
            },
        ];
        assert_eq!(
            check(&f),
            [SafetyError::HoldsRoot {
                device: p("/dev/dm-0")
            }]
        );
    }

    #[test]
    fn root_and_live_media_are_called_out() {
        let mut f = facts();
        f.mounts = vec![
            Mount {
                source: p("/dev/sdb2"),
                target: p("/"),
            },
            Mount {
                source: p("/dev/sdb1"),
                target: p("/iso"),
            },
        ];
        f.related.insert(p("/dev/sdb2"));
        let errs = check(&f);
        assert!(matches!(errs[0], SafetyError::HoldsRoot { .. }));
        assert!(matches!(errs[1], SafetyError::LiveMedia { .. }));
    }

    #[test]
    fn active_swap_rejected() {
        let mut f = facts();
        f.swaps = vec![p("/dev/sdb1"), p("/dev/sda2")];
        assert_eq!(check(&f), [SafetyError::SwapActive(p("/dev/sdb1"))]);
    }

    #[test]
    fn parses_mounts_with_escapes() {
        let m = parse_mounts("/dev/sdb1 /mnt/my\\040disk ext4 rw 0 0\nproc /proc proc rw 0 0\n");
        assert_eq!(m[0].target, p("/mnt/my disk"));
        assert_eq!(m[1].source, p("proc"));
    }

    #[test]
    fn out_of_range_octal_escapes_are_left_alone_not_a_panic() {
        assert_eq!(unescape("a\\777b"), "a\\777b");
        assert_eq!(unescape("\\400"), "\\400");
        assert_eq!(
            unescape("\\377"),
            "\u{fffd}",
            "0xff alone is not valid UTF-8"
        );
        assert_eq!(unescape("x\\040y\\134z"), "x y\\z");
    }

    #[test]
    fn parses_swaps() {
        let s = parse_swaps("Filename\tType\tSize\tUsed\tPriority\n/dev/zram0 partition 1 0 100\n");
        assert_eq!(s, [p("/dev/zram0")]);
    }

    #[test]
    fn real_system_rejects_non_block_and_missing() {
        assert_eq!(
            check_disk("/dev/null"),
            [SafetyError::NotBlockDevice(p("/dev/null"))]
        );
        assert_eq!(
            check_disk("/dev/definitely-not-here"),
            [SafetyError::NotFound(p("/dev/definitely-not-here"))]
        );
    }
}
