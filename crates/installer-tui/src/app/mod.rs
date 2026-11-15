//! The installer's state machine. It knows nothing about the terminal: keys go
//! in, state changes and [`Effect`]s come out, which keeps it testable.
//!
//! Each screen's state lives in its own module and is changed only through
//! that module's `key`; [`App`] decides what happens between screens.

mod checks;
mod disk;
mod errors;
mod install;
mod picker;
mod summary;
mod user;

#[cfg(test)]
mod tests;

pub use checks::ChecksResult;
pub use disk::DiskField;
pub use errors::{Errors, Target};
pub use install::{InstallEvent, InstallState, StepStatus};
pub use picker::Pane;
pub use user::UserField;

use crate::widgets::FilterList;
use checks::Checks;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use disk::DiskForm;
#[cfg(test)]
use errors::FieldError;
use installer_core::{
    build_plan, human_size, render, validate, Answers, Catalog, DiskInfo, Host, Plan, System,
    SCHEMA_VERSION,
};
use picker::{KeyboardPicker, Picker};
use summary::Summary;
use user::UserForm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Timezone,
    Locale,
    Keyboard,
    Disk,
    User,
    Summary,
    Installing,
    Done,
}

/// Every screen in the order it is walked through, with its label in the step
/// bar. The one place the flow is defined. The keyboard comes first so that
/// searching the timezone and language lists already uses the right layout.
pub const STEPS: &[(Screen, &str)] = &[
    (Screen::Welcome, "Welcome"),
    (Screen::Keyboard, "Keyboard"),
    (Screen::Timezone, "Timezone"),
    (Screen::Locale, "Language"),
    (Screen::Disk, "Disk"),
    (Screen::User, "User"),
    (Screen::Summary, "Review"),
    (Screen::Installing, "Install"),
    (Screen::Done, "Done"),
];

impl Screen {
    /// Position in [`STEPS`].
    pub fn step(self) -> usize {
        STEPS
            .iter()
            .position(|(screen, _)| *screen == self)
            .unwrap_or(0)
    }
}

/// Work the event loop has to do on the app's behalf.
#[derive(Debug)]
pub enum Effect {
    Quit,
    Install(Plan),
    Reboot,
    /// Load this layout into the live console.
    ApplyKeymap {
        layout: String,
        variant: Option<String>,
    },
    /// Re-run the Welcome checks. `internet` is false in offline mode.
    RunChecks {
        internet: bool,
    },
}

/// Results coming back from background threads.
#[derive(Debug)]
pub enum AppEvent {
    Install(InstallEvent),
    /// `Ok` carries a short description of what was applied.
    Keymap(Result<String, String>),
    Checks(ChecksResult),
}

/// What a screen wants after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Back,
    Next,
}

/// Every installed system gets this name; it is not asked for.
const DEFAULT_HOSTNAME: &str = "nun";

fn text(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c)
        }
        _ => None,
    }
}

/// Apply a navigation or editing key to a filterable list. Returns whether
/// it was consumed. Enter and Esc are left to the caller.
fn list_edit(list: &mut FilterList, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Up => list.up(),
        KeyCode::Down => list.down(),
        KeyCode::PageUp => list.page(-10),
        KeyCode::PageDown => list.page(10),
        KeyCode::Backspace => list.pop_filter(),
        _ => match text(key) {
            Some(c) => list.push_filter(c),
            None => return false,
        },
    }
    true
}

pub struct App {
    screen: Screen,
    catalog: Catalog,
    host: Host,
    /// Offline install: everything needed is on the install medium, so the
    /// internet check is skipped.
    offline: bool,
    checks: Checks,
    timezone: Picker,
    locale: Picker,
    keyboard: KeyboardPicker,
    disk: DiskForm,
    user: UserForm,
    summary: Summary,
    /// A non-error status line, for example the keymap that was applied.
    notice: Option<String>,
    install: InstallState,
}

impl App {
    pub fn new(
        catalog: Catalog,
        host: Host,
        offline: bool,
        rescan: impl Fn() -> Vec<DiskInfo> + 'static,
    ) -> Self {
        Self {
            screen: Screen::Welcome,
            timezone: Picker::new(&catalog.timezones, "UTC"),
            locale: Picker::new(&catalog.locales, "en_US.UTF-8"),
            keyboard: KeyboardPicker::new(&catalog),
            disk: DiskForm::new(rescan),
            user: UserForm::default(),
            summary: Summary::default(),
            catalog,
            host,
            offline,
            checks: Checks::default(),
            notice: None,
            install: InstallState::default(),
        }
    }

    /// Effects to run when the installer starts.
    pub fn start(&mut self) -> Vec<Effect> {
        self.checks.start();
        vec![Effect::RunChecks {
            internet: !self.offline,
        }]
    }

    /// Show a screen without walking there. Tests only.
    #[cfg(test)]
    pub fn jump_to(&mut self, screen: Screen) {
        self.screen = screen;
    }

    #[cfg(test)]
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    #[cfg(test)]
    pub fn set_install_steps(&mut self, steps: Vec<(String, StepStatus)>) {
        self.install = InstallState::with_steps(steps);
    }

    pub fn set_default_timezone(&mut self, tz: &str) {
        if self.catalog.has_timezone(tz) {
            self.timezone.select(tz);
        }
    }

    // ---- what the screens draw ----

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn offline(&self) -> bool {
        self.offline
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn timezone(&self) -> &FilterList {
        self.timezone.list()
    }

    pub fn locale(&self) -> &FilterList {
        self.locale.list()
    }

    pub fn keyboard(&self) -> &KeyboardPicker {
        &self.keyboard
    }

    pub fn disk(&self) -> &DiskForm {
        &self.disk
    }

    pub fn user(&self) -> &UserForm {
        &self.user
    }

    pub fn summary(&self) -> &Summary {
        &self.summary
    }

    pub fn install(&self) -> &InstallState {
        &self.install
    }

    pub fn checks_running(&self) -> bool {
        self.checks.running()
    }

    pub fn checks_result(&self) -> Option<&ChecksResult> {
        self.checks.result()
    }

    // ---- welcome checks ----

    pub fn has_usable_disk(&self) -> bool {
        self.disk.has_usable()
    }

    pub fn missing_tools(&self) -> &[String] {
        self.checks.missing_tools()
    }

    /// Whether the checks that must pass have passed.
    pub fn can_continue(&self) -> bool {
        self.checks.passed(self.offline) && self.has_usable_disk()
    }

    pub fn on_checks(&mut self, result: ChecksResult) {
        self.checks.finish(result);
    }

    pub fn on_keymap_result(&mut self, result: Result<String, String>) {
        self.notice = Some(match result {
            Ok(applied) => format!("Console keyboard set to {applied}"),
            Err(e) => format!("Console keyboard not changed: {e}"),
        });
    }

    // ---- derived values ----

    /// The answers as entered so far. Fields not yet filled in are empty and
    /// only fail validation once their screen is reached.
    pub fn answers(&self) -> Answers {
        Answers {
            schema_version: SCHEMA_VERSION,
            disk: self.disk.answers(),
            user: self.user.answers(),
            system: System {
                hostname: DEFAULT_HOSTNAME.to_owned(),
                locale: self.locale.value(),
                timezone: self.timezone.value(),
                keyboard_layout: self.keyboard.layout_value(),
                keyboard_variant: self.keyboard.variant_value(),
            },
        }
    }

    pub fn confirm_word(&self) -> String {
        self.disk
            .selected()
            .map(|d| d.name.clone())
            .unwrap_or_default()
    }

    // ---- errors ----

    /// The errors of the screen being shown.
    #[cfg(test)]
    pub fn errors(&self) -> &[FieldError] {
        self.screen_errors().map_or(&[], Errors::all)
    }

    pub fn error_for(&self, target: Target) -> Option<&str> {
        self.screen_errors().and_then(|e| e.get(target))
    }

    fn screen_errors(&self) -> Option<&Errors> {
        match self.screen {
            Screen::Timezone => Some(self.timezone.errors()),
            Screen::Locale => Some(self.locale.errors()),
            Screen::Keyboard => Some(self.keyboard.errors()),
            Screen::Disk => Some(self.disk.errors()),
            Screen::User => Some(self.user.errors()),
            Screen::Summary => Some(self.summary.errors()),
            Screen::Welcome | Screen::Installing | Screen::Done => None,
        }
    }

    // ---- navigation ----

    fn back(&mut self) {
        self.screen = STEPS[self.screen.step().saturating_sub(1)].0;
    }

    fn go_next(&mut self) {
        self.screen = STEPS[(self.screen.step() + 1).min(STEPS.len() - 1)].0;
    }

    fn follow(&mut self, flow: Flow) {
        match flow {
            Flow::Stay => {}
            Flow::Back => self.back(),
            Flow::Next => self.go_next(),
        }
    }

    // ---- input ----

    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return if self.screen == Screen::Installing {
                Vec::new()
            } else {
                vec![Effect::Quit]
            };
        }
        match self.screen {
            Screen::Welcome => return self.welcome_key(key),
            Screen::Timezone => {
                let flow = self.timezone.key(&key);
                self.follow(flow);
            }
            Screen::Locale => {
                let flow = self.locale.key(&key);
                self.follow(flow);
            }
            Screen::Keyboard => {
                let flow = self.keyboard.key(&key, &self.catalog);
                self.follow(flow);
                if flow == Flow::Next {
                    return vec![Effect::ApplyKeymap {
                        layout: self.keyboard.layout_value(),
                        variant: self.keyboard.variant_value(),
                    }];
                }
            }
            Screen::Disk => {
                let flow = self.disk.key(&key);
                self.follow(flow);
            }
            Screen::User => {
                let flow = self.user.key(&key);
                self.follow(flow);
            }
            Screen::Summary => match self.summary.key(&key) {
                Flow::Next => return self.start_install().into_iter().collect(),
                flow => self.follow(flow),
            },
            Screen::Installing => {}
            Screen::Done => {
                return match key.code {
                    KeyCode::Enter if self.install.succeeded() => vec![Effect::Reboot],
                    KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => vec![Effect::Quit],
                    _ => Vec::new(),
                };
            }
        }
        Vec::new()
    }

    fn welcome_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter if self.can_continue() => self.go_next(),
            KeyCode::Char('r') => {
                self.disk.rescan();
                self.checks.start();
                return vec![Effect::RunChecks {
                    internet: !self.offline,
                }];
            }
            KeyCode::Char('q') | KeyCode::Esc => return vec![Effect::Quit],
            _ => {}
        }
        Vec::new()
    }

    fn start_install(&mut self) -> Option<Effect> {
        let word = self.confirm_word();
        if self.summary.confirm().value() != word {
            self.summary
                .fail(format!("type {word} to confirm that it will be erased"));
            return None;
        }
        // The disk may have been mounted, or another one plugged in, since it was
        // picked. What gets wiped is the disk that was just confirmed, found again
        // by path, never whatever now sits at the old list position.
        let confirmed = self.disk.selected().map(|d| d.path.clone());
        self.disk.rescan();
        let still_ok = confirmed.is_some()
            && self.disk.selected().map(|d| &d.path) == confirmed.as_ref()
            && self.disk.selected().is_some_and(DiskInfo::selectable);
        if !still_ok {
            let shown = confirmed.map_or_else(|| "the disk".into(), |p| p.display().to_string());
            self.summary.fail(format!(
                "{shown} is no longer available, go back and choose again"
            ));
            return None;
        }

        let plan = match self.plan() {
            Ok(plan) => plan,
            Err(message) => {
                self.summary.fail(message);
                return None;
            }
        };
        self.install = InstallState::for_plan(&plan);
        self.screen = Screen::Installing;
        Some(Effect::Install(plan))
    }

    fn plan(&self) -> Result<Plan, String> {
        let answers = self.answers();
        if let Some(e) = validate(&answers, &self.catalog).first() {
            return Err(format!("{}: {}", e.field, e.message));
        }
        let files = render(&answers, &self.host).map_err(|e| e.to_string())?;
        build_plan(&answers, &files, &self.host).map_err(|e| e.to_string())
    }

    pub fn on_install_event(&mut self, ev: InstallEvent) {
        if self.install.on_event(ev) {
            self.screen = Screen::Done;
        }
    }

    /// One-line description of a disk, for the summary.
    pub fn disk_label(d: &DiskInfo) -> String {
        let model = if d.model.is_empty() {
            String::new()
        } else {
            format!("  {}", d.model)
        };
        format!("{}  {}{model}", d.path.display(), human_size(d.size_bytes))
    }
}
