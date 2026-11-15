//! Drawing. Reads the app state and never changes it.

use crate::app::{App, OptionsField, Screen, StepStatus, UserField};
use crate::widgets::{FilterList, TextInput};
use installer_core::{human_size, Boot};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

const ACCENT: Color = Color::Cyan;
const DIM: Style = Style::new().fg(Color::DarkGray);
const DANGER: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);

pub fn render(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    match app.screen {
        Screen::Welcome => welcome(frame, body, app),
        Screen::Timezone => list_page(frame, body, "Choose your timezone", &app.timezone),
        Screen::Locale => list_page(frame, body, "Choose your language", &app.locale),
        Screen::KeyboardLayout => {
            list_page(frame, body, "Choose your keyboard layout", &app.layout)
        }
        Screen::KeyboardVariant => {
            list_page(frame, body, "Choose a keyboard variant", &app.variant)
        }
        Screen::Disk => disk_page(frame, body, app),
        Screen::DiskOptions => options_page(frame, body, app),
        Screen::User => user_page(frame, body, app),
        Screen::Hostname => hostname_page(frame, body, app),
        Screen::Summary => summary_page(frame, body, app),
        Screen::Installing | Screen::Done => install_page(frame, body, app),
    }
    render_footer(frame, footer, app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let step = app
        .progress()
        .map(|(i, n)| format!("  step {i} of {n}"))
        .unwrap_or_default();
    let line = Line::from(vec![
        Span::styled(
            "nun installer",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.screen.title(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Span::styled(step, DIM),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = if app.errors.is_empty() {
        vec![Line::styled(hint(app.screen), DIM)]
    } else {
        app.errors
            .iter()
            .map(|e| Line::styled(format!("! {e}"), Style::new().fg(Color::Red)))
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn hint(screen: Screen) -> &'static str {
    match screen {
        Screen::Welcome => "Enter: begin   q: quit",
        Screen::Timezone | Screen::Locale | Screen::KeyboardLayout | Screen::KeyboardVariant => {
            "type to filter   Up/Down: move   Enter: next   Esc: back"
        }
        Screen::Disk => "Up/Down: move   Enter: next   r: rescan   Esc: back",
        Screen::DiskOptions => "Tab: next field   Space: toggle   Enter: next   Esc: back",
        Screen::User => "Tab: next field   Enter: next   Esc: back",
        Screen::Hostname => "Enter: next   Esc: back",
        Screen::Summary => "type the disk name, then Enter to install   Esc: back",
        Screen::Installing => "installing, please wait",
        Screen::Done => "Enter: continue",
    }
}

fn block(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(title)
}

fn welcome(frame: &mut Frame, area: Rect, app: &App) {
    let boot = match app.host.boot {
        Boot::Uefi => "UEFI",
        Boot::Bios => "BIOS",
    };
    let lines = vec![
        Line::from("This installs NixOS with the nun stack:"),
        Line::from("  noctalia, noctalia-greeter and umbriel."),
        Line::from(""),
        Line::from(format!("Detected: {boot} boot, {}", app.host.system)),
        Line::from(""),
        Line::styled(
            "The disk you choose will be erased completely.",
            Style::new().fg(Color::Yellow),
        ),
        Line::from(""),
        Line::from("Pick your timezone, language and keyboard first, so the"),
        Line::from("rest of the questions use the right layout."),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("Welcome")), area);
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

fn list_page(frame: &mut Frame, area: Rect, prompt: &str, list: &FilterList) {
    let [filter, rest] = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);
    frame.render_widget(
        Paragraph::new(format!("{}_", list.filter()))
            .block(block(&format!("{prompt}, type to filter"))),
        filter,
    );
    let visible = list.visible();
    let items: Vec<ListItem> = visible
        .iter()
        .map(|i| ListItem::new(i.label.as_str()))
        .collect();
    let title = format!("{} of {}", visible.len(), list.total());
    let widget = List::new(items)
        .block(block(&title))
        .highlight_style(Style::new().fg(Color::Black).bg(ACCENT))
        .highlight_symbol("> ");
    let mut state = list_state(list.selected_index(), visible.len(), rest.height);
    frame.render_stateful_widget(widget, rest, &mut state);
}

fn disk_page(frame: &mut Frame, area: Rect, app: &App) {
    if app.disks.is_empty() {
        frame.render_widget(
            Paragraph::new("No disks found. Press r to scan again.").block(block("Disks")),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .disks
        .iter()
        .map(|d| {
            let mut lines = vec![Line::raw(App::disk_label(d))];
            if let Some(reason) = d.rejected.first() {
                lines.push(Line::styled(format!("    unavailable: {reason}"), DIM));
            }
            let item = ListItem::new(lines);
            if d.selectable() {
                item
            } else {
                item.style(DIM)
            }
        })
        .collect();
    let widget = List::new(items)
        .block(block("Choose the disk to erase and install on"))
        .highlight_style(Style::new().fg(Color::Black).bg(ACCENT))
        .highlight_symbol("> ");
    let mut state = list_state(app.disk_selected, app.disks.len(), area.height);
    frame.render_stateful_widget(widget, area, &mut state);
}

fn field(label: &str, input: &TextInput, focused: bool) -> Line<'static> {
    let cursor = if focused { "_" } else { "" };
    let style = if focused {
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    Line::from(vec![
        Span::styled(format!("{label:<22}"), style),
        Span::raw(format!("{}{cursor}", input.display())),
    ])
}

fn toggle(label: &str, on: bool, focused: bool) -> Line<'static> {
    let style = if focused {
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    Line::from(vec![
        Span::styled(format!("{label:<22}"), style),
        Span::raw(if on { "[x] yes" } else { "[ ] no" }),
    ])
}

fn options_page(frame: &mut Frame, area: Rect, app: &App) {
    let focus = app.options_focus;
    let mut lines = vec![
        field("Swap size (MiB)", &app.swap, focus == OptionsField::Swap),
        Line::styled("  0 disables swap", DIM),
        Line::from(""),
        toggle(
            "Encrypt the disk",
            app.encrypt,
            focus == OptionsField::Encrypt,
        ),
    ];
    if app.encrypt {
        lines.push(Line::from(""));
        lines.push(field(
            "Passphrase",
            &app.passphrase,
            focus == OptionsField::Passphrase,
        ));
        lines.push(field(
            "Repeat passphrase",
            &app.passphrase_confirm,
            focus == OptionsField::Confirm,
        ));
        lines.push(Line::styled(
            "  You will type this passphrase every time the computer boots.",
            DIM,
        ));
    }
    frame.render_widget(Paragraph::new(lines).block(block("Disk options")), area);
}

fn user_page(frame: &mut Frame, area: Rect, app: &App) {
    let lines = vec![
        field("Username", &app.username, app.user_focus == UserField::Name),
        field(
            "Password",
            &app.password,
            app.user_focus == UserField::Password,
        ),
        field(
            "Repeat password",
            &app.password_confirm,
            app.user_focus == UserField::Confirm,
        ),
        Line::from(""),
        Line::styled(
            "The account can use sudo. The root account stays locked.",
            DIM,
        ),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("User account")), area);
}

fn hostname_page(frame: &mut Frame, area: Rect, app: &App) {
    let lines = vec![
        field("Hostname", &app.hostname, true),
        Line::from(""),
        Line::styled("The name of this computer on the network.", DIM),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("Hostname")), area);
}

fn summary_page(frame: &mut Frame, area: Rect, app: &App) {
    let a = app.answers();
    let disk = app.selected_disk();
    let row =
        |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<14}"), DIM), Span::raw(v)]);
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
        row("Timezone", a.system.timezone.clone()),
        row("Language", a.system.locale.clone()),
        row("Keyboard", keyboard),
        row("Disk", disk.map(App::disk_label).unwrap_or_default()),
        row(
            "Encryption",
            if a.disk.encrypt {
                "yes".into()
            } else {
                "no".into()
            },
        ),
        row("Swap", swap),
        row("User", a.user.name.clone()),
        row("Hostname", a.system.hostname.clone()),
        Line::from(""),
        Line::styled(
            format!("EVERYTHING ON {} WILL BE ERASED.", a.disk.device),
            DANGER,
        ),
        Line::from(""),
        field(
            &format!("Type \"{}\" to confirm", app.confirm_word()),
            &app.confirm,
            true,
        ),
    ];
    frame.render_widget(Paragraph::new(lines).block(block("Review")), area);
}

fn install_page(frame: &mut Frame, area: Rect, app: &App) {
    let steps_height = (app.install.steps.len() as u16 + 2).min(area.height / 2);
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
                StepStatus::Done => ("[x]", Style::new().fg(Color::Green)),
                StepStatus::Failed => ("[!]", Style::new().fg(Color::Red)),
            };
            ListItem::new(format!("{mark} {name}")).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block("Steps")), steps);

    let rows = log.height.saturating_sub(2) as usize;
    let skip = app.install.log.len().saturating_sub(rows);
    let mut lines: Vec<Line> = app
        .install
        .log
        .iter()
        .skip(skip)
        .map(|l| Line::raw(l.as_str()))
        .collect();
    match &app.install.result {
        Some(Ok(())) => lines.push(Line::styled(
            "Installation finished. Press Enter to reboot.",
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        )),
        Some(Err(e)) => lines.push(Line::styled(format!("Installation failed: {e}"), DANGER)),
        None => {}
    }
    let skip = lines.len().saturating_sub(rows);
    let lines: Vec<Line> = lines.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(lines).block(block("Output")), log);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use installer_core::{Boot, Catalog, DiskInfo, Host};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app() -> App {
        let catalog = Catalog::from_sources(
            "DE\t+5230+01322\tEurope/Berlin\n",
            "SUPPORTED-LOCALES=\\\nen_US.UTF-8/UTF-8 \\\n",
            "! layout\n  us  English (US)\n",
        );
        let host = Host {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: String::new(),
            disko_url: String::new(),
            nixpkgs_path: None,
        };
        App::new(catalog, host, || {
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

    fn draw(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
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

    #[test]
    fn every_screen_renders_without_panicking() {
        let mut a = app();
        for screen in [
            Screen::Welcome,
            Screen::Timezone,
            Screen::Locale,
            Screen::KeyboardLayout,
            Screen::KeyboardVariant,
            Screen::Disk,
            Screen::DiskOptions,
            Screen::User,
            Screen::Hostname,
            Screen::Summary,
            Screen::Installing,
            Screen::Done,
        ] {
            a.screen = screen;
            let out = draw(&a);
            assert!(out.contains(screen.title()), "{screen:?}");
        }
    }

    #[test]
    fn welcome_shows_detected_boot_mode() {
        let out = draw(&app());
        assert!(out.contains("UEFI"));
        assert!(out.contains("erased"));
    }

    #[test]
    fn passwords_are_masked_on_screen() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut a = app();
        a.screen = Screen::User;
        a.user_focus = UserField::Password;
        for c in "s3cret".chars() {
            a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let out = draw(&a);
        assert!(!out.contains("s3cret"));
        assert!(out.contains("••••••"));
    }

    #[test]
    fn summary_names_the_disk_to_erase() {
        let mut a = app();
        a.screen = Screen::Summary;
        let out = draw(&a);
        assert!(out.contains("EVERYTHING ON /dev/vda WILL BE ERASED"));
        assert!(out.contains("Type \"vda\" to confirm"));
    }

    #[test]
    fn errors_replace_the_hint() {
        let mut a = app();
        a.errors = vec!["bad thing".into()];
        assert!(draw(&a).contains("! bad thing"));
    }
}
