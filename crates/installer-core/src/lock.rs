//! Pinning the installed system's inputs to the ones this installer was built
//! with, so an install is reproducible and can work without network access.

use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("the lock file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the installer's lock file has no `{0}` input")]
    Missing(&'static str),
    #[error("the installer's `disko` input does not follow `nixpkgs`, unlike the generated flake")]
    DiskoDoesNotFollowNixpkgs,
}

/// Reduce the installer's own `flake.lock` to the two inputs the generated flake
/// has. The generated flake declares the same URLs, and `disko` following
/// `nixpkgs`, so these nodes can be copied as they are.
pub fn target_lock(installer_lock: &str) -> Result<String, LockError> {
    let lock: Value = serde_json::from_str(installer_lock)?;
    let node = |name: &'static str| {
        lock.get("nodes")
            .and_then(|n| n.get(name))
            .cloned()
            .ok_or(LockError::Missing(name))
    };
    let nixpkgs = node("nixpkgs")?;
    let disko = node("disko")?;
    if disko.pointer("/inputs/nixpkgs") != Some(&json!(["nixpkgs"])) {
        return Err(LockError::DiskoDoesNotFollowNixpkgs);
    }
    let reduced = json!({
        "nodes": {
            "disko": disko,
            "nixpkgs": nixpkgs,
            "root": { "inputs": { "disko": "disko", "nixpkgs": "nixpkgs" } },
        },
        "root": "root",
        "version": lock.get("version").cloned().unwrap_or(json!(7)),
    });
    Ok(serde_json::to_string_pretty(&reduced)? + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"{
      "nodes": {
        "crane": { "locked": { "rev": "c" }, "original": { "repo": "crane" } },
        "disko": {
          "inputs": { "nixpkgs": ["nixpkgs"] },
          "locked": { "rev": "d", "narHash": "sha256-d" },
          "original": { "owner": "nix-community", "repo": "disko", "type": "github" }
        },
        "nixpkgs": {
          "locked": { "rev": "n", "narHash": "sha256-n" },
          "original": { "owner": "NixOS", "ref": "nixos-unstable", "repo": "nixpkgs", "type": "github" }
        },
        "root": { "inputs": { "crane": "crane", "disko": "disko", "nixpkgs": "nixpkgs" } }
      },
      "root": "root",
      "version": 7
    }"#;

    #[test]
    fn keeps_only_the_inputs_the_generated_flake_has() {
        let out: Value = serde_json::from_str(&target_lock(LOCK).unwrap()).unwrap();
        let nodes = out["nodes"].as_object().unwrap();
        let mut names: Vec<_> = nodes.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["disko", "nixpkgs", "root"]);
        assert_eq!(out["root"], "root");
        assert_eq!(out["version"], 7);
        assert_eq!(
            out["nodes"]["root"]["inputs"],
            json!({ "disko": "disko", "nixpkgs": "nixpkgs" })
        );
    }

    #[test]
    fn copies_the_pins_unchanged() {
        let out: Value = serde_json::from_str(&target_lock(LOCK).unwrap()).unwrap();
        assert_eq!(out["nodes"]["nixpkgs"]["locked"]["rev"], "n");
        assert_eq!(out["nodes"]["nixpkgs"]["original"]["ref"], "nixos-unstable");
        assert_eq!(out["nodes"]["disko"]["locked"]["narHash"], "sha256-d");
        assert_eq!(
            out["nodes"]["disko"]["inputs"]["nixpkgs"],
            json!(["nixpkgs"])
        );
    }

    #[test]
    fn ends_with_a_newline_like_nix_writes_it() {
        assert!(target_lock(LOCK).unwrap().ends_with("}\n"));
    }

    #[test]
    fn reports_a_missing_input() {
        let no_disko = LOCK.replace("\"disko\": {", "\"disko2\": {");
        assert!(matches!(
            target_lock(&no_disko),
            Err(LockError::Missing("disko"))
        ));
        assert!(matches!(
            target_lock("{}"),
            Err(LockError::Missing("nixpkgs"))
        ));
    }

    #[test]
    fn refuses_a_disko_that_does_not_follow_nixpkgs() {
        let own = LOCK.replace("\"inputs\": { \"nixpkgs\": [\"nixpkgs\"] },", "");
        assert!(matches!(
            target_lock(&own),
            Err(LockError::DiskoDoesNotFollowNixpkgs)
        ));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(target_lock("not json"), Err(LockError::Json(_))));
    }

    #[test]
    fn works_on_this_repos_real_lock_file() {
        let real = include_str!("../../../flake.lock");
        let out = target_lock(real).expect("the repo's own lock must reduce cleanly");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["nodes"]["nixpkgs"]["locked"]["narHash"].is_string());
        assert!(v["nodes"]["disko"]["locked"]["rev"].is_string());
    }
}
