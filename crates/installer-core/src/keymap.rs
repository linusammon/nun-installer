//! Applying the chosen keyboard layout to the live console, so passwords are
//! typed with the layout the user picked. This is the same pipeline NixOS's
//! `console.useXkbConfig` uses: `ckbcomp` converts an XKB layout to a console
//! keymap and `loadkeys` loads it.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    #[error("cannot run {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("{program} failed: {message}")]
    Failed { program: String, message: String },
}

/// Where the conversion tools live. Defaults are found through `PATH`.
#[derive(Debug, Clone)]
pub struct KeymapTools {
    pub ckbcomp: PathBuf,
    pub loadkeys: PathBuf,
    /// XKB data directory (the one containing `rules/`), if not the system one.
    pub xkb_dir: Option<PathBuf>,
}

/// `base.lst` sits in `<xkb dir>/rules/`.
fn xkb_dir_from_base_lst(base_lst: &std::path::Path) -> Option<PathBuf> {
    Some(base_lst.parent()?.parent()?.to_path_buf())
}

impl KeymapTools {
    pub fn from_env() -> Self {
        let var = |name: &str, default: &str| {
            std::env::var_os(name).map_or_else(|| PathBuf::from(default), PathBuf::from)
        };
        let xkb_dir = std::env::var_os("NUN_XKB_BASE_LST")
            .and_then(|p| xkb_dir_from_base_lst(std::path::Path::new(&p)));
        Self {
            ckbcomp: var("NUN_CKBCOMP", "ckbcomp"),
            loadkeys: var("NUN_LOADKEYS", "loadkeys"),
            xkb_dir,
        }
    }
}

fn run_error(program: &std::path::Path, out: &std::process::Output) -> KeymapError {
    let message = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    KeymapError::Failed {
        program: program.display().to_string(),
        message: if message.is_empty() {
            out.status.to_string()
        } else {
            message
        },
    }
}

/// Convert `layout`/`variant` to a console keymap and load it. Fails when not
/// running on a Linux virtual console (for example over SSH); callers should
/// treat that as non-fatal.
pub fn apply_console_keymap(
    tools: &KeymapTools,
    layout: &str,
    variant: Option<&str>,
) -> Result<(), KeymapError> {
    let mut ckbcomp = Command::new(&tools.ckbcomp);
    if let Some(dir) = &tools.xkb_dir {
        ckbcomp.arg(format!("-I{}", dir.display()));
    }
    ckbcomp.args(["-layout", layout]);
    if let Some(v) = variant {
        ckbcomp.args(["-variant", v]);
    }
    let keymap = ckbcomp.output().map_err(|source| KeymapError::Spawn {
        program: tools.ckbcomp.display().to_string(),
        source,
    })?;
    if !keymap.status.success() {
        return Err(run_error(&tools.ckbcomp, &keymap));
    }

    let mut loadkeys = Command::new(&tools.loadkeys)
        .args(["-u", "-q", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| KeymapError::Spawn {
            program: tools.loadkeys.display().to_string(),
            source,
        })?;
    if let Some(mut stdin) = loadkeys.stdin.take() {
        // loadkeys may exit early on a bad console; its status tells us why.
        let _ = stdin.write_all(&keymap.stdout);
    }
    let out = loadkeys
        .wait_with_output()
        .map_err(|source| KeymapError::Spawn {
            program: tools.loadkeys.display().to_string(),
            source,
        })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(run_error(&tools.loadkeys, &out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// Another test forking while a script is still open for writing makes the
    /// first exec fail with ETXTBSY; it clears within moments, so retry.
    fn apply(tools: &KeymapTools, layout: &str, variant: Option<&str>) -> Result<(), KeymapError> {
        for _ in 0..50 {
            match apply_console_keymap(tools, layout, variant) {
                Err(KeymapError::Spawn { source, .. }) if source.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => return other,
            }
        }
        apply_console_keymap(tools, layout, variant)
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nun-keymap-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn pipes_ckbcomp_output_into_loadkeys() {
        let d = tmp("ok");
        let ckbcomp = script(&d, "ckbcomp", "echo \"ARGS: $*\"");
        let loadkeys = script(
            &d,
            "loadkeys",
            &format!("echo \"LK: $*\" > {d}/lk\ncat >> {d}/lk", d = d.display()),
        );
        let tools = KeymapTools {
            ckbcomp,
            loadkeys,
            xkb_dir: Some("/xkb".into()),
        };
        apply(&tools, "ch", Some("fr")).unwrap();
        let got = std::fs::read_to_string(d.join("lk")).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(got.contains("LK: -u -q -"));
        assert!(got.contains("ARGS: -I/xkb -layout ch -variant fr"));
    }

    #[test]
    fn omits_variant_and_xkb_dir_when_unset() {
        let d = tmp("plain");
        let ckbcomp = script(&d, "ckbcomp", "echo \"ARGS: $*\"");
        let loadkeys = script(&d, "loadkeys", &format!("cat > {}/lk", d.display()));
        let tools = KeymapTools {
            ckbcomp,
            loadkeys,
            xkb_dir: None,
        };
        apply(&tools, "us", None).unwrap();
        let got = std::fs::read_to_string(d.join("lk")).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(got.trim(), "ARGS: -layout us");
    }

    #[test]
    fn loadkeys_failure_carries_its_message() {
        let d = tmp("fail");
        let ckbcomp = script(&d, "ckbcomp", "echo map");
        let loadkeys = script(
            &d,
            "loadkeys",
            "cat >/dev/null; echo 'not a console' >&2; exit 1",
        );
        let tools = KeymapTools {
            ckbcomp,
            loadkeys,
            xkb_dir: None,
        };
        let err = apply(&tools, "us", None).unwrap_err();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains("not a console"), "{err}");
    }

    #[test]
    fn ckbcomp_failure_stops_before_loadkeys() {
        let d = tmp("ckb");
        let ckbcomp = script(&d, "ckbcomp", "echo 'unknown layout' >&2; exit 2");
        let loadkeys = script(&d, "loadkeys", &format!("touch {}/ran", d.display()));
        let tools = KeymapTools {
            ckbcomp,
            loadkeys,
            xkb_dir: None,
        };
        let err = apply(&tools, "xx", None).unwrap_err();
        let ran = d.join("ran").exists();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains("unknown layout"));
        assert!(!ran);
    }

    #[test]
    fn missing_tool_is_a_spawn_error() {
        let tools = KeymapTools {
            ckbcomp: "/definitely/not/here".into(),
            loadkeys: "loadkeys".into(),
            xkb_dir: None,
        };
        assert!(matches!(
            apply_console_keymap(&tools, "us", None),
            Err(KeymapError::Spawn { .. })
        ));
    }

    #[test]
    fn derives_xkb_dir_from_base_lst() {
        let dir = xkb_dir_from_base_lst(std::path::Path::new(
            "/nix/store/x/share/X11/xkb/rules/base.lst",
        ));
        assert_eq!(
            dir.as_deref(),
            Some(std::path::Path::new("/nix/store/x/share/X11/xkb"))
        );
        assert_eq!(
            xkb_dir_from_base_lst(std::path::Path::new("base.lst")),
            None
        );
    }
}
