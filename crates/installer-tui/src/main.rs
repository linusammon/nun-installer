use installer_core::{
    build_plan, check_disk, render, run_plan, validate, Answers, Catalog, CatalogPaths, DryRun,
    Host, Live,
};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: nun-installer [--config <answers.json> [--dry-run | --yes]]

Without --config the interactive installer starts.

  --config <file>  Install headlessly from an answers file
  --dry-run        Print the plan and generated files, change nothing
  --yes            Allow a headless run to wipe the disk
  -h, --help       Show this help
  -V, --version    Show the version
";

#[derive(Debug, Default)]
struct Args {
    config: Option<PathBuf>,
    dry_run: bool,
    yes: bool,
}

enum Parsed {
    Run(Args),
    Exit(ExitCode),
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Parsed {
    let mut args = Args::default();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => match it.next() {
                Some(p) => args.config = Some(p.into()),
                None => return usage_error("--config needs a file"),
            },
            "--dry-run" => args.dry_run = true,
            "--yes" => args.yes = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Parsed::Exit(ExitCode::SUCCESS);
            }
            "-V" | "--version" => {
                println!("nun-installer {}", env!("CARGO_PKG_VERSION"));
                return Parsed::Exit(ExitCode::SUCCESS);
            }
            other => return usage_error(&format!("unknown argument '{other}'")),
        }
    }
    if args.config.is_none() && (args.dry_run || args.yes) {
        return usage_error("--dry-run and --yes need --config");
    }
    if args.dry_run && args.yes {
        return usage_error("--dry-run and --yes are mutually exclusive");
    }
    Parsed::Run(args)
}

fn usage_error(msg: &str) -> Parsed {
    eprintln!("error: {msg}\n\n{USAGE}");
    Parsed::Exit(ExitCode::from(2))
}

fn headless(config: &PathBuf, dry_run: bool, yes: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(config)
        .map_err(|e| format!("cannot read {}: {e}", config.display()))?;
    let answers: Answers = serde_json::from_str(&text)
        .map_err(|e| format!("invalid answers file {}: {e}", config.display()))?;

    let catalog = Catalog::load(&CatalogPaths::from_env()).map_err(|e| e.to_string())?;
    let errors = validate(&answers, &catalog);
    if !errors.is_empty() {
        let list: Vec<_> = errors
            .iter()
            .map(|e| format!("  {:?}: {}", e.field, e.message))
            .collect();
        return Err(format!("invalid answers:\n{}", list.join("\n")));
    }

    if !dry_run && !yes {
        return Err(format!(
            "refusing to wipe {} without --yes (use --dry-run to preview)",
            answers.disk.device
        ));
    }

    let problems = check_disk(&answers.disk.device);
    if dry_run {
        for p in &problems {
            eprintln!("warning: {p}");
        }
    } else if !problems.is_empty() {
        let list: Vec<_> = problems.iter().map(|p| format!("  {p}")).collect();
        return Err(format!("unsafe target disk:\n{}", list.join("\n")));
    }

    let host = Host::detect().map_err(|e| e.to_string())?;
    let files = render(&answers, &host).map_err(|e| e.to_string())?;
    let plan = build_plan(&answers, &files, &host);

    if dry_run {
        let mut executor = DryRun(std::io::stdout());
        let total = plan.actions.len();
        run_plan(&plan, &mut executor, |i, _| {
            let _ = writeln!(std::io::stdout(), "[{}/{total}]", i + 1);
        })
        .map_err(|e| e.to_string())
    } else {
        let total = plan.actions.len();
        run_plan(&plan, &mut Live::stderr(), |i, a| {
            eprintln!("[{}/{total}] {}", i + 1, a.description);
        })
        .map_err(|e| e.to_string())
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Parsed::Run(a) => a,
        Parsed::Exit(code) => return code,
    };
    let Some(config) = &args.config else {
        eprintln!("error: the interactive installer is not implemented yet, use --config");
        return ExitCode::from(2);
    };
    match headless(config, args.dry_run, args.yes) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
