//! Drawing. Reads the app state and never changes it.
//!
//! The look follows the old noctalia installer: a step bar on top, key hints
//! at the bottom right, borderless pages, and bordered widgets that turn
//! yellow when focused and show their own error in the bottom border.

use crate::app::{App, DiskField, Pane, Screen, StepStatus, Target, UserField, BAR};
use crate::widgets::{FilterList, TextInput};
use installer_core::{human_size, DiskInfo};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

/// Text, borders and the cursor. Light blue, because plain blue is too dark to
/// read on the Linux console's black background.
const ACCENT: Color = Color::LightBlue;
/// Background of the current step in the step bar, under white text.
const ACCENT_BG: Color = Color::Blue;
const DIM: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const OK: Style = Style::new().fg(Color::Green);
const WARN: Style = Style::new().fg(Color::Yellow);
const FAIL: Style = Style::new().fg(Color::Red);
const DANGER: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);

pub fn render(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    step_bar(frame, middle_row(header), app);
    match app.screen {
        Screen::Welcome => welcome(frame, body, app),
        Screen::Timezone => pick_list(
            frame,
            body,
            "Timezone",
            &app.timezone,
            true,
            app.error_for(Target::Screen),
        ),
        Screen::Locale => pick_list(
            frame,
            body,
            "Language",
            &app.locale,
            true,
            app.error_for(Target::Screen),
        ),
        Screen::Keyboard => keyboard(frame, body, app),
        Screen::Disk => disk_page(frame, body, app),
        Screen::User => user_page(frame, body, app),
        Screen::Summary => summary_page(frame, body, app),
        Screen::Installing | Screen::Done => install_page(frame, body, app),
    }
    footer_bar(frame, middle_row(footer), app);
}

/// The bars are three rows tall with the text on the middle one, which leaves
/// breathing room above and below.
fn middle_row(area: Rect) -> Rect {
    Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    }
}

fn step_bar(frame: &mut Frame, area: Rect, app: &App) {
    let current = app.screen.bar_index();
    let width: usize = BAR.iter().map(|(l, _)| l.chars().count() + 3).sum();
    let line = if width <= area.width as usize {
        let mut spans = Vec::new();
        for (i, (label, _)) in BAR.iter().enumerate() {
            let style = if i == current {
                Style::new().fg(Color::White).bg(ACCENT_BG)
            } else {
                Style::new()
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!(" {label} "), style));
        }
        Line::from(spans)
    } else {
        // Too narrow for every step: show only where we are.
        Line::from(vec![
            Span::styled(
                format!(" {} ", BAR[current].0),
                Style::new().fg(Color::White).bg(ACCENT_BG),
            ),
            Span::styled(format!("  step {} of {}", current + 1, BAR.len()), DIM),
        ])
    };
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.screen {
        Screen::Welcome if app.can_continue() => {
            vec![("Enter", "continue"), ("r", "check again"), ("q", "quit")]
        }
        Screen::Welcome => vec![("r", "check again"), ("q", "quit")],
        Screen::Timezone | Screen::Locale => vec![
            ("Type", "filter"),
            ("Up/Down", "select"),
            ("Esc", "prev"),
            ("Enter", "confirm"),
        ],
        Screen::Keyboard => vec![
            ("Type", "filter"),
            ("Up/Down", "select"),
            ("Tab", "switch list"),
            ("Esc", "prev"),
            ("Enter", "confirm"),
        ],
        Screen::Disk if app.disk_focus == DiskField::Disk => vec![
            ("Up/Down", "select"),
            ("Tab", "next field"),
            ("r", "rescan"),
            ("Esc", "prev"),
            ("Enter", "confirm"),
        ],
        Screen::Disk if app.disk_focus == DiskField::Encrypt => vec![
            ("Space", "toggle"),
            ("Tab", "next field"),
            ("Esc", "prev"),
            ("Enter", "confirm"),
        ],
        Screen::Disk => vec![("Tab", "next field"), ("Esc", "prev"), ("Enter", "confirm")],
        Screen::User => vec![("Tab", "next"), ("Esc", "prev"), ("Enter", "confirm")],
        Screen::Summary => vec![("Esc", "prev"), ("Enter", "install")],
        Screen::Installing => vec![],
        Screen::Done if app.install.result == Some(Ok(())) => vec![("Enter", "reboot")],
        Screen::Done => vec![("Enter", "quit")],
    }
}

fn footer_bar(frame: &mut Frame, area: Rect, app: &App) {
    let hints = hints(app);
    let mut spans = Vec::new();
    let mut width = 0;
    for (key, action) in &hints {
        spans.push(Span::styled(
            format!(" {key}"),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {action} ")));
        width += key.chars().count() + action.chars().count() + 3;
    }
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width as u16)]).areas(area);
    if let Some(notice) = &app.notice {
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {notice}"), DIM)),
            left,
        );
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), right);
}

// ---- building blocks ----

fn field_block(title: &str, focused: bool, error: Option<&str>) -> Block<'static> {
    let border = if error.is_some() {
        FAIL
    } else if focused {
        Style::new().fg(ACCENT)
    } else {
        DIM
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Line::from(format!(" {title} ")));
    if let Some(e) = error {
        block = block.title_bottom(Line::styled(format!(" {e} "), FAIL));
    }
    block
}

/// The text with a block cursor at the end while focused.
fn input_line(input: &TextInput, focused: bool) -> Line<'static> {
    let mut spans = vec![Span::raw(input.display())];
    if focused {
        spans.push(Span::styled(" ", Style::new().bg(ACCENT)));
    }
    Line::from(spans)
}

fn input_field(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    input: &TextInput,
    focused: bool,
    error: Option<&str>,
) {
    frame.render_widget(
        Paragraph::new(input_line(input, focused)).block(field_block(title, focused, error)),
        area,
    );
}

fn checkbox(label: &str, checked: bool, focused: bool) -> Line<'static> {
    let bracket = if checked { "[x]" } else { "[ ]" };
    let (bracket_style, label_style) = if focused {
        (Style::new().fg(ACCENT), BOLD)
    } else {
        (Style::new(), Style::new())
    };
    Line::from(vec![
        Span::styled(bracket, bracket_style),
        Span::raw(" "),
        Span::styled(label.to_owned(), label_style),
    ])
}

/// Keep the selection roughly centred as the list scrolls.
fn list_state(selected: usize, len: usize, height: u16) -> ListState {
    let rows = height.saturating_sub(2) as usize;
    let offset = selected
        .saturating_sub(rows / 2)
        .min(len.saturating_sub(rows));
    ListState::default()
        .with_offset(offset)
        .with_selected(Some(selected))
}

fn pick_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    list: &FilterList,
    focused: bool,
    error: Option<&str>,
) {
    let visible = list.visible();
    let count = if list.filter().is_empty() {
        let n = list.total();
        format!("{n} item{}", if n == 1 { "" } else { "s" })
    } else {
        let n = visible.len();
        format!(
            "{n} match{} for \"{}\"",
            if n == 1 { "" } else { "es" },
            list.filter()
        )
    };
    let items: Vec<ListItem> = visible
        .iter()
        .map(|i| {
            let mut spans = vec![Span::raw(i.label.clone())];
            if !i.detail.is_empty() {
                spans.push(Span::styled(format!("  {}", i.detail), DIM));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let widget = List::new(items)
        .block(field_block(&format!("{title} — {count}"), focused, error))
        .highlight_symbol("> ")
        .highlight_style(BOLD);
    let mut state = list_state(list.selected_index(), visible.len(), area.height);
    frame.render_stateful_widget(widget, area, &mut state);
}

// ---- screens ----

/// `[OK] Label`, one line per check. `kind` is OK, FAIL, WARN or `--`.
fn status(kind: &str, label: &str) -> Line<'static> {
    let style = match kind {
        "OK" => OK,
        "FAIL" => FAIL,
        "WARN" => WARN,
        _ => DIM,
    };
    Line::styled(format!("[{kind}] {label}"), style)
}

/// An explanation under a check, only shown when there is something to say.
fn note(text: &str) -> Line<'static> {
    Line::styled(format!("       {text}"), DIM)
}

fn welcome(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![
        Line::raw("Welcome to the NUN installer."),
        Line::raw(""),
        Line::raw("It will install NixOS with Noctalia, its Greeter as well as Umbriel."),
        Line::raw(""),
    ];

    match (&app.checks.result, app.checks.running) {
        (_, true) | (None, _) => lines.push(Line::raw("Checking prerequisites...")),
        (Some(result), false) => {
            lines.push(status("OK", &app.host.system));

            if app.offline {
                lines.push(status("--", "Internet"));
                lines.push(note(
                    "Offline install: everything needed is on this medium.",
                ));
            } else if result.internet.online() {
                lines.push(status("OK", "Internet"));
            } else {
                lines.push(status("FAIL", "Internet"));
                let mut down = Vec::new();
                if !result.internet.github {
                    down.push("github.com");
                }
                if !result.internet.cache {
                    down.push("cache.nixos.org");
                }
                lines.push(note(&format!("Cannot reach {}.", down.join(" and "))));
                lines.push(note(
                    "Connect with a cable, or run `nmtui` in another terminal, then press r.",
                ));
            }

            if app.has_usable_disk() {
                lines.push(status("OK", "Disk"));
            } else {
                lines.push(status("FAIL", "Disk"));
                lines.push(note("No disk to install onto was found."));
            }

            if !app.missing_tools().is_empty() {
                lines.push(status("FAIL", "Tools"));
                lines.push(note(&format!(
                    "Not found: {}. Run this from a NixOS installer environment.",
                    app.missing_tools().join(", ")
                )));
            }

            if let Some(clock) = &result.clock_note {
                lines.push(status("WARN", "Clock"));
                lines.push(note(clock));
            }
            if result.on_ac == Some(false) {
                lines.push(status("WARN", "Battery"));
                lines.push(note(
                    "Not on AC power. Losing power mid-install can leave the disk broken.",
                ));
            }

            lines.push(Line::raw(""));
            if app.can_continue() {
                lines.push(Line::raw("Press Enter to continue."));
            } else {
                lines.push(Line::styled(
                    "Cannot continue until the above is resolved. Press r to check again.",
                    FAIL,
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn keyboard(frame: &mut Frame, area: Rect, app: &App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    pick_list(
        frame,
        left,
        "Layout",
        &app.layout,
        app.pane == Pane::Layout,
        app.error_for(Target::Screen),
    );
    pick_list(
        frame,
        right,
        &format!("Variant of \"{}\"", app.layout_description()),
        &app.variant,
        app.pane == Pane::Variant,
        None,
    );
}

/// Second line of a disk row. Virtual disks report no model, so name the kind.
fn disk_detail(d: &DiskInfo) -> String {
    if let Some(reason) = d.rejected.first() {
        return format!("unavailable: {reason}");
    }
    let model = if !d.model.is_empty() {
        d.model.clone()
    } else if d.name.starts_with("vd") || d.name.starts_with("xvd") {
        "Virtual disk".to_owned()
    } else {
        "Disk".to_owned()
    };
    let removable = if d.removable { "  [removable]" } else { "" };
    format!("{model}{removable}")
}

fn disk_list(frame: &mut Frame, area: Rect, app: &App, focused: bool) {
    let error = app.error_for(Target::Disk);
    let title = "Disk — choose the disk to erase";
    if app.disks.is_empty() {
        frame.render_widget(
            Paragraph::new("No disks found. Press r to scan again.")
                .block(field_block(title, focused, error)),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .disks
        .iter()
        .map(|d| {
            let title_style = if d.selectable() { BOLD } else { DIM };
            let first = Line::from(vec![
                Span::styled(d.path.display().to_string(), title_style),
                Span::styled(format!("  {}", human_size(d.size_bytes)), title_style),
            ]);
            ListItem::new(vec![first, Line::styled(disk_detail(d), DIM)])
        })
        .collect();
    let widget = List::new(items)
        .block(field_block(title, focused, error))
        .highlight_symbol("> ");
    let mut state = list_state(app.disk_selected, app.disks.len(), area.height);
    frame.render_stateful_widget(widget, area, &mut state);
}

/// Disk, swap and encryption share one screen.
fn disk_page(frame: &mut Frame, area: Rect, app: &App) {
    let focus = app.disk_focus;
    let mut rows = vec![
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(1),
    ];
    if app.encrypt {
        rows.extend([Constraint::Length(3), Constraint::Length(1)]);
    }
    let areas = Layout::vertical(rows).split(area);

    disk_list(frame, areas[0], app, focus == DiskField::Disk);
    input_field(
        frame,
        areas[1],
        "Swap size (MiB, 0 for none)",
        &app.swap,
        focus == DiskField::Swap,
        app.error_for(Target::Swap),
    );
    frame.render_widget(
        Paragraph::new(checkbox(
            "Encrypt system (LUKS)",
            app.encrypt,
            focus == DiskField::Encrypt,
        )),
        areas[2],
    );
    if app.encrypt {
        let [a, b] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(areas[3]);
        input_field(
            frame,
            a,
            "Passphrase",
            &app.passphrase,
            focus == DiskField::Passphrase,
            app.error_for(Target::Passphrase),
        );
        input_field(
            frame,
            b,
            "Repeat passphrase",
            &app.passphrase_confirm,
            focus == DiskField::Confirm,
            app.error_for(Target::PassphraseConfirm),
        );
        frame.render_widget(
            Paragraph::new(Line::styled(
                "You will type this passphrase every time the computer boots.",
                DIM,
            )),
            areas[4],
        );
    }
}

fn user_page(frame: &mut Frame, area: Rect, app: &App) {
    let [name, pw, note, _] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .areas(area);
    input_field(
        frame,
        name,
        "Username",
        &app.username,
        app.user_focus == UserField::Name,
        app.error_for(Target::Username),
    );
    let [a, b] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(pw);
    input_field(
        frame,
        a,
        "Password",
        &app.password,
        app.user_focus == UserField::Password,
        app.error_for(Target::Password),
    );
    input_field(
        frame,
        b,
        "Confirm password",
        &app.password_confirm,
        app.user_focus == UserField::Confirm,
        app.error_for(Target::PasswordConfirm),
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "The account can use sudo. The root account stays locked.",
            DIM,
        )),
        note,
    );
}

fn summary_page(frame: &mut Frame, area: Rect, app: &App) {
    let a = app.answers();
    let row =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<12}"), DIM), Span::raw(v)]);
    let keyboard = match &a.system.keyboard_variant {
        Some(v) => format!("{} ({v})", a.system.keyboard_layout),
        None => a.system.keyboard_layout.clone(),
    };
    let swap = if a.disk.swap_size_mib == 0 {
        "none".to_owned()
    } else {
        human_size(u64::from(a.disk.swap_size_mib) * 1024 * 1024)
    };
    let lines = vec![
        row(
            "Disk",
            app.selected_disk().map(App::disk_label).unwrap_or_default(),
        ),
        row(
            "Encryption",
            if a.disk.encrypt {
                "yes".into()
            } else {
                "no".into()
            },
        ),
        row("Swap", swap),
        row("Hostname", a.system.hostname.clone()),
        row("Timezone", a.system.timezone.clone()),
        row("Language", a.system.locale.clone()),
        row("Keyboard", keyboard),
        row("User", a.user.name.clone()),
        Line::raw(""),
        Line::styled(
            format!("EVERYTHING ON {} WILL BE ERASED.", a.disk.device),
            DANGER,
        ),
        // One empty line between the warning and the confirmation field.
        Line::raw(""),
    ];
    let [text, field, _] = Layout::vertical([
        Constraint::Length(lines.len() as u16),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(lines), text);
    input_field(
        frame,
        field,
        &format!("Type the disk name ({}) to confirm", app.confirm_word()),
        &app.confirm,
        true,
        app.error_for(Target::Confirm),
    );
}

fn install_page(frame: &mut Frame, area: Rect, app: &App) {
    let steps_height = (app.install.steps.len() as u16).min(area.height / 2);
    let [steps, log] =
        Layout::vertical([Constraint::Length(steps_height), Constraint::Min(3)]).areas(area);

    let items: Vec<ListItem> = app
        .install
        .steps
        .iter()
        .map(|(name, status)| {
            let (mark, style) = match status {
                StepStatus::Pending => ("[ ]", DIM),
                StepStatus::Running => ("[>]", Style::new().fg(ACCENT)),
                StepStatus::Done => ("[x]", OK),
                StepStatus::Failed => ("[!]", FAIL),
            };
            ListItem::new(format!("{mark} {name}")).style(style)
        })
        .collect();
    frame.render_widget(List::new(items), steps);

    let rows = log.height.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = app
        .install
        .log
        .iter()
        .map(|l| Line::styled(l.clone(), DIM))
        .collect();
    match &app.install.result {
        Some(Ok(())) => lines.push(Line::styled(
            "Installation complete. Press Enter to reboot.",
            OK.add_modifier(Modifier::BOLD),
        )),
        Some(Err(e)) => lines.push(Line::styled(format!("Installation failed: {e}"), DANGER)),
        None => {}
    }
    let skip = lines.len().saturating_sub(rows);
    let lines: Vec<Line> = lines.into_iter().skip(skip).collect();
    frame.render_widget(
        Paragraph::new(lines).block(field_block("Output", false, None)),
        log,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ChecksResult;
    use installer_core::{Boot, Catalog, DiskInfo, Host, Reachability};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app() -> App {
        let catalog = Catalog::from_sources(
            "DE\t+5230+01322\tEurope/Berlin\n",
            "SUPPORTED-LOCALES=\\\nen_US.UTF-8/UTF-8 \\\n",
            "! layout\n  us  English (US)\n  ch  German (Switzerland)\n\n! variant\n  fr  ch: French (Switzerland)\n",
        );
        let host = Host {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: String::new(),
            disko_url: String::new(),
            nixpkgs_path: None,
            pinned_lock: None,
        };
        App::new(catalog, host, false, || {
            vec![DiskInfo {
                path: "/dev/vda".into(),
                name: "vda".into(),
                size_bytes: 20 << 30,
                model: "Virtual".into(),
                removable: false,
                rejected: vec![],
            }]
        })
    }

    fn checks(online: bool) -> ChecksResult {
        ChecksResult {
            internet: Reachability {
                github: online,
                cache: online,
            },
            clock_note: None,
            on_ac: None,
            missing_tools: vec![],
        }
    }

    fn draw_sized(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw(app: &App) -> String {
        draw_sized(app, 110, 30)
    }

    #[test]
    fn every_screen_renders_without_panicking() {
        let mut a = app();
        for screen in [
            Screen::Welcome,
            Screen::Timezone,
            Screen::Locale,
            Screen::Keyboard,
            Screen::Disk,
            Screen::User,
            Screen::Summary,
            Screen::Installing,
            Screen::Done,
        ] {
            a.screen = screen;
            for (w, h) in [(110, 30), (80, 24), (40, 12)] {
                draw_sized(&a, w, h);
            }
        }
    }

    #[test]
    fn step_bar_highlights_the_current_step_and_lists_all() {
        let mut a = app();
        a.screen = Screen::Summary;
        let out = draw(&a);
        for (label, _) in BAR {
            assert!(out.contains(label), "{label}");
        }
    }

    #[test]
    fn narrow_terminals_get_a_compact_step_bar() {
        let mut a = app();
        a.screen = Screen::Disk;
        let out = draw_sized(&a, 60, 24);
        assert!(out.contains("step 5 of 9"), "{out}");
    }

    #[test]
    fn welcome_shows_check_results_in_the_old_style() {
        let mut a = app();
        a.start();
        assert!(draw(&a).contains("Checking prerequisites"));

        a.on_checks(checks(true));
        let out = draw(&a);
        assert!(out.contains("[OK] x86_64-linux"));
        assert!(out.contains("[OK] Internet"));
        assert!(out.contains("[OK] Disk"));
        assert!(out.contains("Press Enter to continue."));

        a.on_checks(checks(false));
        let out = draw(&a);
        assert!(out.contains("[FAIL] Internet"));
        assert!(out.contains("github.com and cache.nixos.org"));
        assert!(out.contains("Cannot continue"));
    }

    #[test]
    fn disk_swap_and_encryption_share_one_screen() {
        let mut a = app();
        a.screen = Screen::Disk;
        let out = draw(&a);
        assert!(out.contains("Disk — choose the disk to erase"));
        assert!(out.contains("Swap size (MiB, 0 for none)"));
        assert!(out.contains("[ ] Encrypt system (LUKS)"));
        assert!(
            !out.contains("Passphrase"),
            "only shown once encryption is on"
        );

        a.encrypt = true;
        let out = draw(&a);
        assert!(out.contains("[x] Encrypt system (LUKS)"));
        assert!(out.contains("Passphrase"));
        assert!(out.contains("Repeat passphrase"));
    }

    #[test]
    fn disk_rows_describe_what_the_disk_is() {
        let disk = |name: &str, model: &str, removable: bool| DiskInfo {
            path: format!("/dev/{name}").into(),
            name: name.into(),
            size_bytes: 1 << 30,
            model: model.into(),
            removable,
            rejected: vec![],
        };
        assert_eq!(disk_detail(&disk("vdb", "", false)), "Virtual disk");
        assert_eq!(disk_detail(&disk("sda", "", false)), "Disk");
        assert_eq!(
            disk_detail(&disk("sda", "USB Stick", true)),
            "USB Stick  [removable]"
        );
        let mut rejected = disk("sda", "", false);
        rejected.rejected = vec![installer_core::SafetyError::SwapActive("/dev/sda1".into())];
        assert!(disk_detail(&rejected).starts_with("unavailable: "));
    }

    #[test]
    fn welcome_intro_text() {
        let out = draw(&app());
        assert!(out.contains("Welcome to the NUN installer."));
        assert!(
            out.contains("It will install NixOS with Noctalia, its Greeter as well as Umbriel.")
        );
    }

    #[test]
    fn welcome_blocks_and_explains_when_tools_are_missing() {
        let mut a = app();
        a.start();
        a.on_checks(ChecksResult {
            missing_tools: vec!["disko".into(), "nixos-install".into()],
            ..checks(true)
        });
        assert!(!a.can_continue());
        let out = draw(&a);
        assert!(out.contains("[FAIL] Tools"));
        assert!(out.contains("Not found: disko, nixos-install"));
        assert!(out.contains("Cannot continue"));
    }

    #[test]
    fn welcome_stays_quiet_about_tools_when_all_are_there() {
        let mut a = app();
        a.start();
        a.on_checks(checks(true));
        assert!(!draw(&a).contains("Tools"));
    }

    #[test]
    fn welcome_warns_about_clock_and_battery() {
        let mut a = app();
        a.start();
        a.on_checks(ChecksResult {
            clock_note: Some("The system clock looks wrong".into()),
            on_ac: Some(false),
            ..checks(true)
        });
        let out = draw(&a);
        assert!(out.contains("[WARN] Clock"));
        assert!(out.contains("The system clock looks wrong"));
        assert!(out.contains("[WARN] Battery"));
        assert!(out.contains("Not on AC power"));
    }

    #[test]
    fn offline_mode_says_so_instead_of_checking_the_internet() {
        let mut a = app();
        a.offline = true;
        a.start();
        a.on_checks(checks(false));
        let out = draw(&a);
        assert!(out.contains("[--] Internet"));
        assert!(out.contains("Offline install"));
        assert!(!out.contains("[FAIL]"));
    }

    #[test]
    fn keyboard_screen_shows_both_panes_with_the_variant_title() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut a = app();
        a.screen = Screen::Keyboard;
        for c in "switz".chars() {
            a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let out = draw(&a);
        assert!(out.contains("Layout — "));
        assert!(out.contains("Variant of \"German (Switzerland)\""));
        assert!(out.contains("French (Switzerland)"));
    }

    #[test]
    fn passwords_are_masked_with_stars() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut a = app();
        a.screen = Screen::User;
        a.user_focus = UserField::Password;
        for c in "s3cret".chars() {
            a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let out = draw(&a);
        assert!(!out.contains("s3cret"));
        assert!(out.contains("******"));
    }

    #[test]
    fn summary_names_the_disk_to_erase() {
        let mut a = app();
        a.screen = Screen::Summary;
        let out = draw(&a);
        assert!(out.contains("EVERYTHING ON /dev/vda WILL BE ERASED"));
        assert!(out.contains("Type the disk name (vda) to confirm"));
        assert!(
            !out.contains("below to confirm"),
            "the prompt is in the field title"
        );
    }

    #[test]
    fn the_confirm_field_sits_one_blank_line_below_the_warning() {
        let mut a = app();
        a.screen = Screen::Summary;
        let out = draw(&a);
        let lines: Vec<_> = out.lines().collect();
        let warning = lines
            .iter()
            .position(|l| l.contains("WILL BE ERASED"))
            .unwrap();
        assert!(lines[warning + 1].trim().is_empty(), "one empty line");
        assert!(
            lines[warning + 2].contains("Type the disk name (vda) to confirm"),
            "{}",
            lines[warning + 2]
        );
    }

    #[test]
    fn errors_show_in_the_field_border_not_a_footer() {
        use crate::app::FieldError;
        let mut a = app();
        a.screen = Screen::User;
        a.errors = vec![FieldError {
            target: Target::Username,
            message: "bad thing".into(),
        }];
        let out = draw(&a);
        assert!(out.contains(" bad thing "));
        // Rendered on the border row, directly under the text row.
        let lines: Vec<_> = out.lines().collect();
        let i = lines.iter().position(|l| l.contains("bad thing")).unwrap();
        assert!(lines[i].contains('└'), "{}", lines[i]);
    }

    #[test]
    fn hints_sit_on_the_right_and_notice_on_the_left() {
        let mut a = app();
        a.screen = Screen::Summary;
        a.on_keymap_result(Ok("ch".into()));
        let out = draw(&a);
        let line = out.lines().find(|l| l.contains("Esc prev")).unwrap();
        assert!(line.contains("Console keyboard set to ch"));
        assert!(line.find("Console").unwrap() < line.find("Esc").unwrap());
    }
}
