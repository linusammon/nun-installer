mod app;
mod headless;
mod privilege;
mod tui;
mod ui;
mod widgets;
mod workers;

use installer_core::{Catalog, CatalogPaths, Host};
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

fn interactive() -> Result<(), String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(
            "the interactive installer needs a terminal (use --config for headless)".into(),
        );
    }
    let catalog = Catalog::load(&CatalogPaths::from_env()).map_err(|e| e.to_string())?;
    let host = Host::detect().map_err(|e| e.to_string())?;
    // After the terminal and data checks, so mistakes fail before a sudo prompt.
    privilege::require_root();
    tui::run(catalog, host).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let Some(raw) = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        eprintln!("error: arguments must be valid UTF-8");
        return ExitCode::from(2);
    };
    let args = match parse_args(raw.into_iter()) {
        Parsed::Run(a) => a,
        Parsed::Exit(code) => return code,
    };
    let result = match &args.config {
        Some(config) => headless::run(config, args.dry_run, args.yes),
        None => interactive(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
