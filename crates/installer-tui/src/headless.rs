//! Installing from an answers file without the interactive UI.

use installer_core::{
    build_plan, check_disk, missing_tools, render, run_plan, validate, Answers, Catalog,
    CatalogPaths, DryRun, Host, Live,
};
use std::io::Write;
use std::path::Path;

/// Read and validate an answers file.
fn load(config: &Path) -> Result<Answers, String> {
    let text = std::fs::read_to_string(config)
        .map_err(|e| format!("cannot read {}: {e}", config.display()))?;
    let answers: Answers = serde_json::from_str(&text)
        .map_err(|e| format!("invalid answers file {}: {e}", config.display()))?;

    let catalog = Catalog::load(&CatalogPaths::from_env()).map_err(|e| e.to_string())?;
    let errors = validate(&answers, &catalog);
    if !errors.is_empty() {
        let list: Vec<_> = errors
            .iter()
            .map(|e| format!("  {}: {}", e.field, e.message))
            .collect();
        return Err(format!("invalid answers:\n{}", list.join("\n")));
    }
    Ok(answers)
}

/// Write the generated files into `dir` and change nothing else.
pub fn render_to(config: &Path, dir: &Path) -> Result<(), String> {
    let answers = load(config)?;
    let host = Host::detect().map_err(|e| e.to_string())?;
    let files = render(&answers, &host).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (name, content) in [
        ("flake.nix", &files.flake_nix),
        ("configuration.nix", &files.configuration_nix),
        ("disko.nix", &files.disko_nix),
    ] {
        std::fs::write(dir.join(name), content).map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    Ok(())
}

pub fn run(config: &Path, dry_run: bool, yes: bool) -> Result<(), String> {
    let answers = load(config)?;

    if !dry_run && !yes {
        return Err(format!(
            "refusing to wipe {} without --yes (use --dry-run to preview)",
            answers.disk.device
        ));
    }
    if !dry_run {
        // After the cheap checks above, so mistakes fail before a sudo prompt.
        crate::privilege::require_root();
    }

    let problems = check_disk(Path::new(&answers.disk.device));
    if dry_run {
        for p in &problems {
            eprintln!("warning: {p}");
        }
    } else if !problems.is_empty() {
        let list: Vec<_> = problems.iter().map(|p| format!("  {p}")).collect();
        return Err(format!("unsafe target disk:\n{}", list.join("\n")));
    }

    let host = Host::detect().map_err(|e| e.to_string())?;
    if !dry_run {
        let missing = missing_tools();
        if !missing.is_empty() {
            return Err(format!(
                "required programs not found: {} (run this from a NixOS installer environment)",
                missing.join(", ")
            ));
        }
    }
    let files = render(&answers, &host).map_err(|e| e.to_string())?;
    let plan = build_plan(&answers, &files, &host).map_err(|e| e.to_string())?;
    let total = plan.actions.len();

    if dry_run {
        run_plan(&plan, &mut DryRun(std::io::stdout()), |i, _| {
            let _ = writeln!(std::io::stdout(), "[{}/{total}]", i + 1);
        })
    } else {
        run_plan(&plan, &mut Live::stderr(), |i, a| {
            eprintln!("[{}/{total}] {}", i + 1, a.description);
        })
    }
    .map_err(|e| e.to_string())
}
