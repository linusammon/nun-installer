use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nun-installer"))
        .args(args)
        .env("NUN_ZONE_TAB", fixture("zone.tab"))
        .env("NUN_LOCALE_SUPPORTED", fixture("SUPPORTED"))
        .env("NUN_XKB_BASE_LST", fixture("base.lst"))
        .env("NUN_STATE_VERSION", "26.05")
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn dry_run_prints_plan_and_redacts_secrets() {
    let answers = fixture("answers.json");
    let out = run(&["--config", answers.to_str().unwrap(), "--dry-run"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("Partition, format and mount"));
    assert!(stdout.contains("disko --mode destroy,format,mount"));
    assert!(stdout.contains("programs.umbriel.enable = true;"));
    assert!(stdout.contains("<redacted>"));
    assert!(!stdout.contains("correct horse battery"));
    assert!(!stdout.contains("$y$j9T$abc$def"));
}

#[test]
fn live_run_without_yes_is_refused() {
    let answers = fixture("answers.json");
    let out = run(&["--config", answers.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("--yes"));
}

#[test]
fn invalid_answers_are_reported_per_field() {
    let dir = std::env::temp_dir().join(format!("nun-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.json");
    let json = std::fs::read_to_string(fixture("answers.json"))
        .unwrap()
        .replace("Europe/Berlin", "Europe/Atlantis");
    std::fs::write(&bad, json).unwrap();
    let out = run(&["--config", bad.to_str().unwrap(), "--dry-run"]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("timezone"));
}

#[test]
fn usage_errors_exit_2() {
    assert_eq!(run(&["--dry-run"]).status.code(), Some(2));
    assert_eq!(run(&["--bogus"]).status.code(), Some(2));
}

#[test]
fn interactive_mode_needs_a_terminal() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("needs a terminal"));
}
