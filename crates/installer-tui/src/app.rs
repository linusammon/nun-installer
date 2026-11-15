//! The installer's state machine. It knows nothing about the terminal: keys go
//! in, state changes and [`Effect`]s come out, which keeps it testable.

use crate::widgets::{FilterList, Item, TextInput};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use installer_core::{
    build_plan, hash_password, human_size, render, validate, Answers, Catalog, Disk, DiskInfo,
    Field, Host, Plan, Reachability, Secret, System, User, SCHEMA_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Timezone,
    Locale,
    Keyboard,
    Disk,
    DiskOptions,
    User,
    Hostname,
    Summary,
    Installing,
    Done,
}

/// The step bar at the top: a label and the screens that belong to it.
pub const BAR: &[(&str, &[Screen])] = &[
    ("Welcome", &[Screen::Welcome]),
    ("Timezone", &[Screen::Timezone]),
    ("Language", &[Screen::Locale]),
    ("Keyboard", &[Screen::Keyboard]),
    ("Disk", &[Screen::Disk, Screen::DiskOptions]),
    ("User", &[Screen::User]),
    ("Hostname", &[Screen::Hostname]),
    ("Review", &[Screen::Summary]),
    ("Install", &[Screen::Installing]),
    ("Done", &[Screen::Done]),
];

/// Screens in the order they are walked through.
const FLOW: &[Screen] = &[
    Screen::Welcome,
    Screen::Timezone,
    Screen::Locale,
    Screen::Keyboard,
    Screen::Disk,
    Screen::DiskOptions,
    Screen::User,
    Screen::Hostname,
    Screen::Summary,
    Screen::Installing,
    Screen::Done,
];

impl Screen {
    pub fn bar_index(self) -> usize {
        BAR.iter()
            .position(|(_, screens)| screens.contains(&self))
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

#[derive(Debug, Clone)]
pub struct ChecksResult {
    pub internet: Reachability,
    pub clock_note: Option<String>,
    /// `None`: no battery. `Some(false)`: running on battery.
    pub on_ac: Option<bool>,
}

#[derive(Debug, Default)]
pub struct Checks {
    pub running: bool,
    pub result: Option<ChecksResult>,
}

/// Which widget an error belongs to, so it can be drawn in that widget's border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The list or screen as a whole.
    Screen,
    Disk,
    Swap,
    Passphrase,
    PassphraseConfirm,
    Username,
    Password,
    PasswordConfirm,
    Hostname,
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub target: Target,
    pub message: String,
}

fn field_target(field: Field) -> Target {
    match field {
        Field::DiskDevice => Target::Disk,
        Field::SwapSize => Target::Swap,
        Field::EncryptionPassword => Target::Passphrase,
        Field::UserName => Target::Username,
        Field::UserPasswordHash => Target::Password,
        Field::Hostname => Target::Hostname,
        Field::SchemaVersion | Field::Locale | Field::Timezone => Target::Screen,
        Field::KeyboardLayout | Field::KeyboardVariant => Target::Screen,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Layout,
    Variant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsField {
    Swap,
    Encrypt,
    Passphrase,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserField {
    Name,
    Password,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Default)]
pub struct InstallState {
    pub steps: Vec<(String, StepStatus)>,
    pub log: Vec<String>,
    pub result: Option<Result<(), String>>,
}

#[derive(Debug)]
pub enum InstallEvent {
    Step(usize),
    Output(String),
    Finished(Result<(), String>),
}

const LOG_LINES: usize = 1000;

pub struct App {
    pub screen: Screen,
    pub catalog: Catalog,
    pub host: Host,
    /// Offline install: everything needed is on the install medium, so the
    /// internet check is skipped.
    pub offline: bool,
    rescan: Box<dyn Fn() -> Vec<DiskInfo>>,
    pub disks: Vec<DiskInfo>,
    pub checks: Checks,

    pub timezone: FilterList,
    pub locale: FilterList,
    pub layout: FilterList,
    pub variant: FilterList,
    pub pane: Pane,
    /// Layout the variant list was built for.
    variant_layout: String,

    pub disk_selected: usize,
    pub swap: TextInput,
    pub encrypt: bool,
    pub passphrase: TextInput,
    pub passphrase_confirm: TextInput,
    pub options_focus: OptionsField,

    pub username: TextInput,
    pub password: TextInput,
    pub password_confirm: TextInput,
    pub user_focus: UserField,
    password_hash: Option<Secret>,

    pub hostname: TextInput,
    pub confirm: TextInput,

    pub errors: Vec<FieldError>,
    /// A non-error status line, for example the keymap that was applied.
    pub notice: Option<String>,
    pub install: InstallState,
}

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

impl App {
    pub fn new(
        catalog: Catalog,
        host: Host,
        offline: bool,
        rescan: impl Fn() -> Vec<DiskInfo> + 'static,
    ) -> Self {
        let disks = rescan();
        let items = |v: &[String]| v.iter().map(|s| Item::new(s, s)).collect::<Vec<_>>();
        let mut timezone = FilterList::new(items(&catalog.timezones));
        timezone.select_value("UTC");
        let mut locale = FilterList::new(items(&catalog.locales));
        locale.select_value("en_US.UTF-8");
        let mut layout = FilterList::new(
            catalog
                .layouts
                .iter()
                .map(|l| Item::with_detail(&l.name, &l.description, &l.name))
                .collect(),
        );
        layout.select_value("us");
        let disk_selected = disks.iter().position(DiskInfo::selectable).unwrap_or(0);

        let mut app = Self {
            screen: Screen::Welcome,
            catalog,
            host,
            offline,
            rescan: Box::new(rescan),
            disks,
            checks: Checks::default(),
            timezone,
            locale,
            layout,
            variant: FilterList::default(),
            pane: Pane::Layout,
            variant_layout: String::new(),
            disk_selected,
            swap: TextInput::new("4096"),
            encrypt: false,
            passphrase: TextInput::masked(),
            passphrase_confirm: TextInput::masked(),
            options_focus: OptionsField::Swap,
            username: TextInput::default(),
            password: TextInput::masked(),
            password_confirm: TextInput::masked(),
            user_focus: UserField::Name,
            password_hash: None,
            hostname: TextInput::new("nun"),
            confirm: TextInput::default(),
            errors: Vec::new(),
            notice: None,
            install: InstallState::default(),
        };
        app.sync_variants();
        app
    }

    /// Effects to run when the installer starts.
    pub fn start(&mut self) -> Vec<Effect> {
        self.checks.running = true;
        vec![Effect::RunChecks {
            internet: !self.offline,
        }]
    }

    pub fn set_default_timezone(&mut self, tz: &str) {
        if self.catalog.has_timezone(tz) {
            self.timezone.select_value(tz);
        }
    }

    // ---- welcome checks ----

    pub fn has_usable_disk(&self) -> bool {
        self.disks.iter().any(DiskInfo::selectable)
    }

    pub fn internet_ok(&self) -> bool {
        self.offline
            || self
                .checks
                .result
                .as_ref()
                .is_some_and(|r| r.internet.online())
    }

    /// Whether the checks that must pass have passed.
    pub fn can_continue(&self) -> bool {
        !self.checks.running
            && self.checks.result.is_some()
            && self.internet_ok()
            && self.has_usable_disk()
    }

    pub fn on_checks(&mut self, result: ChecksResult) {
        self.checks = Checks {
            running: false,
            result: Some(result),
        };
    }

    pub fn on_keymap_result(&mut self, result: Result<String, String>) {
        self.notice = Some(match result {
            Ok(applied) => format!("Console keyboard set to {applied}"),
            Err(e) => format!("Console keyboard not changed: {e}"),
        });
    }

    // ---- derived values ----

    fn selected_value(list: &FilterList) -> String {
        list.selected().map(|i| i.value.clone()).unwrap_or_default()
    }

    pub fn layout_value(&self) -> String {
        Self::selected_value(&self.layout)
    }

    fn variant_value(&self) -> Option<String> {
        Some(Self::selected_value(&self.variant)).filter(|v| !v.is_empty())
    }

    pub fn has_variants(&self) -> bool {
        self.catalog
            .layout(&self.layout_value())
            .is_some_and(|l| !l.variants.is_empty())
    }

    pub fn selected_disk(&self) -> Option<&DiskInfo> {
        self.disks.get(self.disk_selected)
    }

    pub fn options_fields(&self) -> Vec<OptionsField> {
        let mut f = vec![OptionsField::Swap, OptionsField::Encrypt];
        if self.encrypt {
            f.extend([OptionsField::Passphrase, OptionsField::Confirm]);
        }
        f
    }

    /// The answers as entered so far. Fields not yet filled in are empty and
    /// only fail validation once their screen is reached.
    pub fn answers(&self) -> Answers {
        Answers {
            schema_version: SCHEMA_VERSION,
            disk: Disk {
                device: self
                    .selected_disk()
                    .map(|d| d.path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                swap_size_mib: self.swap.value().parse().unwrap_or(0),
                encrypt: self.encrypt,
                encryption_password: self.encrypt.then(|| Secret::new(self.passphrase.value())),
            },
            user: User {
                name: self.username.value().to_owned(),
                password_hash: self
                    .password_hash
                    .clone()
                    .unwrap_or_else(|| Secret::new("")),
            },
            system: System {
                hostname: self.hostname.value().to_owned(),
                locale: Self::selected_value(&self.locale),
                timezone: Self::selected_value(&self.timezone),
                keyboard_layout: self.layout_value(),
                keyboard_variant: self.variant_value(),
            },
        }
    }

    // ---- errors ----

    pub fn error_for(&self, target: Target) -> Option<&str> {
        self.errors
            .iter()
            .find(|e| e.target == target)
            .map(|e| e.message.as_str())
    }

    fn fail(&mut self, target: Target, message: impl Into<String>) {
        self.errors = vec![FieldError {
            target,
            message: message.into(),
        }];
    }

    /// Errors from validating everything, limited to the fields of one screen.
    fn errors_for(&self, fields: &[Field]) -> Vec<FieldError> {
        validate(&self.answers(), &self.catalog)
            .into_iter()
            .filter(|e| fields.contains(&e.field))
            .map(|e| FieldError {
                target: field_target(e.field),
                message: e.message,
            })
            .collect()
    }

    // ---- navigation ----

    fn index(&self) -> usize {
        FLOW.iter().position(|s| *s == self.screen).unwrap_or(0)
    }

    fn back(&mut self) {
        self.screen = FLOW[self.index().saturating_sub(1)];
    }

    /// Move on if this screen's answers are valid, otherwise show why not.
    fn advance(&mut self, fields: &[Field]) {
        let errors = self.errors_for(fields);
        if !errors.is_empty() {
            self.errors = errors;
            return;
        }
        self.screen = FLOW[(self.index() + 1).min(FLOW.len() - 1)];
    }

    /// Keep the variant pane in step with the highlighted layout.
    fn sync_variants(&mut self) {
        let layout = self.layout_value();
        if layout == self.variant_layout {
            return;
        }
        let mut items = vec![Item::with_detail("", "(default)", "")];
        if let Some(l) = self.catalog.layout(&layout) {
            items.extend(
                l.variants
                    .iter()
                    .map(|v| Item::with_detail(&v.name, &v.description, &v.name)),
            );
        }
        self.variant = FilterList::new(items);
        self.variant_layout = layout;
    }

    /// Description of the highlighted layout, for the variant pane's title.
    pub fn layout_description(&self) -> String {
        self.layout
            .selected()
            .map(|i| i.label.clone())
            .unwrap_or_default()
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
        self.errors.clear();
        match self.screen {
            Screen::Welcome => self.welcome_key(key),
            Screen::Timezone | Screen::Locale => {
                self.list_key(key);
                Vec::new()
            }
            Screen::Keyboard => self.keyboard_key(key),
            Screen::Disk => {
                self.disk_key(key);
                Vec::new()
            }
            Screen::DiskOptions => {
                self.options_key(key);
                Vec::new()
            }
            Screen::User => {
                self.user_key(key);
                Vec::new()
            }
            Screen::Hostname => {
                self.hostname_key(key);
                Vec::new()
            }
            Screen::Summary => self.summary_key(key),
            Screen::Installing => Vec::new(),
            Screen::Done => match key.code {
                KeyCode::Enter if self.install.result == Some(Ok(())) => vec![Effect::Reboot],
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => vec![Effect::Quit],
                _ => Vec::new(),
            },
        }
    }

    fn welcome_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter if self.can_continue() => self.screen = Screen::Timezone,
            KeyCode::Char('r') => {
                self.disks = (self.rescan)();
                self.disk_selected = self
                    .disks
                    .iter()
                    .position(DiskInfo::selectable)
                    .unwrap_or(0);
                self.checks.running = true;
                return vec![Effect::RunChecks {
                    internet: !self.offline,
                }];
            }
            KeyCode::Char('q') | KeyCode::Esc => return vec![Effect::Quit],
            _ => {}
        }
        Vec::new()
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

    fn list_key(&mut self, key: KeyEvent) {
        let (field, list) = match self.screen {
            Screen::Timezone => (Field::Timezone, &mut self.timezone),
            _ => (Field::Locale, &mut self.locale),
        };
        if Self::list_edit(list, &key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                if list.filter().is_empty() {
                    self.back();
                } else {
                    list.clear_filter();
                }
            }
            KeyCode::Enter => {
                if list.selected().is_none() {
                    self.fail(Target::Screen, "nothing matches, edit the filter");
                } else {
                    self.advance(&[field]);
                }
            }
            _ => {}
        }
    }

    fn keyboard_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Tab => {
                self.pane = match self.pane {
                    Pane::Layout => Pane::Variant,
                    Pane::Variant => Pane::Layout,
                };
            }
            KeyCode::Left => self.pane = Pane::Layout,
            KeyCode::Right => self.pane = Pane::Variant,
            KeyCode::Esc => {
                let list = match self.pane {
                    Pane::Layout => &mut self.layout,
                    Pane::Variant => &mut self.variant,
                };
                if !list.filter().is_empty() {
                    list.clear_filter();
                    if self.pane == Pane::Layout {
                        self.sync_variants();
                    }
                } else if self.pane == Pane::Variant {
                    self.pane = Pane::Layout;
                } else {
                    self.back();
                }
            }
            KeyCode::Enter => {
                if self.layout.selected().is_none() {
                    self.fail(Target::Screen, "nothing matches, edit the filter");
                } else if self.pane == Pane::Layout && self.has_variants() {
                    // Let the user pick a variant; Enter there confirms.
                    self.pane = Pane::Variant;
                } else {
                    return self.confirm_keyboard();
                }
            }
            _ => {
                let list = match self.pane {
                    Pane::Layout => &mut self.layout,
                    Pane::Variant => &mut self.variant,
                };
                if Self::list_edit(list, &key) && self.pane == Pane::Layout {
                    self.sync_variants();
                }
            }
        }
        Vec::new()
    }

    fn confirm_keyboard(&mut self) -> Vec<Effect> {
        let errors = self.errors_for(&[Field::KeyboardLayout, Field::KeyboardVariant]);
        if !errors.is_empty() {
            self.errors = errors;
            return Vec::new();
        }
        self.screen = Screen::Disk;
        vec![Effect::ApplyKeymap {
            layout: self.layout_value(),
            variant: self.variant_value(),
        }]
    }

    fn disk_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.disk_selected = self.disk_selected.saturating_sub(1),
            KeyCode::Down => {
                self.disk_selected =
                    (self.disk_selected + 1).min(self.disks.len().saturating_sub(1));
            }
            KeyCode::Char('r') => {
                self.disks = (self.rescan)();
                self.disk_selected = self
                    .disks
                    .iter()
                    .position(DiskInfo::selectable)
                    .unwrap_or(0);
            }
            KeyCode::Esc => self.back(),
            KeyCode::Enter => match self.selected_disk() {
                None => self.fail(Target::Disk, "no disk found, press r to scan again"),
                Some(d) if !d.selectable() => {
                    let msg = d
                        .rejected
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ");
                    self.fail(Target::Disk, msg);
                }
                Some(_) => self.advance(&[Field::DiskDevice]),
            },
            _ => {}
        }
    }

    fn move_options_focus(&mut self, delta: isize) {
        let fields = self.options_fields();
        let i = fields
            .iter()
            .position(|f| *f == self.options_focus)
            .unwrap_or(0) as isize;
        let n = fields.len() as isize;
        self.options_focus = fields[(i + delta).rem_euclid(n) as usize];
    }

    fn options_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_options_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_options_focus(-1),
            KeyCode::Esc => self.back(),
            KeyCode::Enter => self.submit_options(),
            KeyCode::Backspace => match self.options_focus {
                OptionsField::Swap => self.swap.pop(),
                OptionsField::Passphrase => self.passphrase.pop(),
                OptionsField::Confirm => self.passphrase_confirm.pop(),
                OptionsField::Encrypt => {}
            },
            _ => match (self.options_focus, text(&key)) {
                (OptionsField::Encrypt, Some(' ')) => {
                    self.encrypt = !self.encrypt;
                    if !self.encrypt {
                        self.passphrase.clear();
                        self.passphrase_confirm.clear();
                    }
                }
                (OptionsField::Swap, Some(c)) if c.is_ascii_digit() => self.swap.push(c),
                (OptionsField::Passphrase, Some(c)) => self.passphrase.push(c),
                (OptionsField::Confirm, Some(c)) => self.passphrase_confirm.push(c),
                _ => {}
            },
        }
    }

    fn submit_options(&mut self) {
        let mut errors = Vec::new();
        if self.swap.value().parse::<u32>().is_err() {
            errors.push(FieldError {
                target: Target::Swap,
                message: "enter a number of MiB (0 for none)".into(),
            });
        }
        errors.extend(self.errors_for(&[Field::SwapSize, Field::EncryptionPassword]));
        if self.encrypt && self.passphrase.value() != self.passphrase_confirm.value() {
            errors.push(FieldError {
                target: Target::PassphraseConfirm,
                message: "passphrases do not match".into(),
            });
        }
        if errors.is_empty() {
            self.advance(&[]);
        } else {
            self.errors = errors;
        }
    }

    fn move_user_focus(&mut self, delta: isize) {
        const ORDER: [UserField; 3] = [UserField::Name, UserField::Password, UserField::Confirm];
        let i = ORDER
            .iter()
            .position(|f| *f == self.user_focus)
            .unwrap_or(0) as isize;
        self.user_focus = ORDER[(i + delta).rem_euclid(3) as usize];
    }

    fn user_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_user_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_user_focus(-1),
            KeyCode::Esc => self.back(),
            KeyCode::Enter => self.submit_user(),
            KeyCode::Backspace => match self.user_focus {
                UserField::Name => self.username.pop(),
                UserField::Password => self.password.pop(),
                UserField::Confirm => self.password_confirm.pop(),
            },
            _ => {
                if let Some(c) = text(&key) {
                    match self.user_focus {
                        UserField::Name => self.username.push(c),
                        UserField::Password => self.password.push(c),
                        UserField::Confirm => self.password_confirm.push(c),
                    }
                }
            }
        }
    }

    fn submit_user(&mut self) {
        let mut errors = self.errors_for(&[Field::UserName]);
        let keep_existing = self.password.is_empty()
            && self.password_confirm.is_empty()
            && self.password_hash.is_some();
        if !keep_existing {
            if self.password.is_empty() {
                errors.push(FieldError {
                    target: Target::Password,
                    message: "password must not be empty".into(),
                });
            } else if self.password.value() != self.password_confirm.value() {
                errors.push(FieldError {
                    target: Target::PasswordConfirm,
                    message: "password confirmation doesn't match".into(),
                });
            }
        }
        if !errors.is_empty() {
            self.errors = errors;
            return;
        }
        if !keep_existing {
            match hash_password(self.password.value()) {
                Ok(h) => self.password_hash = Some(h),
                Err(e) => {
                    self.fail(Target::Password, e.to_string());
                    return;
                }
            }
            // Only the hash is kept from here on.
            self.password.clear();
            self.password_confirm.clear();
        }
        self.advance(&[]);
    }

    fn hostname_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.back(),
            KeyCode::Enter => self.advance(&[Field::Hostname]),
            KeyCode::Backspace => self.hostname.pop(),
            _ => {
                if let Some(c) = text(&key) {
                    self.hostname.push(c);
                }
            }
        }
    }

    pub fn confirm_word(&self) -> String {
        self.selected_disk()
            .map(|d| d.name.clone())
            .unwrap_or_default()
    }

    fn summary_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Esc => {
                self.confirm.clear();
                self.back();
            }
            KeyCode::Backspace => self.confirm.pop(),
            KeyCode::Enter => return self.start_install().into_iter().collect(),
            _ => {
                if let Some(c) = text(&key) {
                    self.confirm.push(c);
                }
            }
        }
        Vec::new()
    }

    fn start_install(&mut self) -> Option<Effect> {
        if self.confirm.value() != self.confirm_word() {
            let word = self.confirm_word();
            self.fail(
                Target::Confirm,
                format!("type {word} to confirm that it will be erased"),
            );
            return None;
        }
        // The disk may have been mounted since it was picked.
        self.disks = (self.rescan)();
        let path = self.answers().disk.device;
        let still_ok = self
            .disks
            .iter()
            .find(|d| d.path.to_string_lossy() == path)
            .is_some_and(DiskInfo::selectable);
        if !still_ok {
            self.fail(
                Target::Confirm,
                format!("{path} is no longer available, go back and choose again"),
            );
            return None;
        }

        let answers = self.answers();
        let errors = validate(&answers, &self.catalog);
        if let Some(e) = errors.first() {
            self.fail(Target::Confirm, format!("{:?}: {}", e.field, e.message));
            return None;
        }
        let plan = match render(&answers, &self.host) {
            Ok(files) => build_plan(&answers, &files, &self.host),
            Err(e) => {
                self.fail(Target::Confirm, e.to_string());
                return None;
            }
        };
        self.install = InstallState {
            steps: plan
                .actions
                .iter()
                .map(|a| (a.description.clone(), StepStatus::Pending))
                .collect(),
            ..InstallState::default()
        };
        self.screen = Screen::Installing;
        Some(Effect::Install(plan))
    }

    pub fn on_install_event(&mut self, ev: InstallEvent) {
        match ev {
            InstallEvent::Step(i) => {
                for (n, (_, status)) in self.install.steps.iter_mut().enumerate() {
                    if n < i && *status != StepStatus::Failed {
                        *status = StepStatus::Done;
                    } else if n == i {
                        *status = StepStatus::Running;
                    }
                }
            }
            InstallEvent::Output(line) => {
                self.install.log.push(line);
                let extra = self.install.log.len().saturating_sub(LOG_LINES);
                self.install.log.drain(..extra);
            }
            InstallEvent::Finished(result) => {
                for (_, status) in &mut self.install.steps {
                    match (&result, *status) {
                        (Ok(()), _) => *status = StepStatus::Done,
                        (Err(_), StepStatus::Running) => *status = StepStatus::Failed,
                        _ => {}
                    }
                }
                self.install.result = Some(result);
                self.screen = Screen::Done;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use installer_core::{Boot, SafetyError};

    fn catalog() -> Catalog {
        Catalog::from_sources(
            "DE\t+5230+01322\tEurope/Berlin\nCH\t+4723+00832\tEurope/Zurich\n",
            "SUPPORTED-LOCALES=\\\nde_CH.UTF-8/UTF-8 \\\nen_US.UTF-8/UTF-8 \\\n",
            "! layout\n  us  English (US)\n  ch  German (Switzerland)\n\n! variant\n  fr  ch: French (Switzerland)\n",
        )
    }

    fn host() -> Host {
        Host {
            boot: Boot::Uefi,
            system: "x86_64-linux".into(),
            state_version: "26.05".into(),
            nixpkgs_url: "github:NixOS/nixpkgs/nixos-unstable".into(),
            disko_url: "github:nix-community/disko".into(),
            nixpkgs_path: None,
        }
    }

    fn disk(name: &str, rejected: Vec<SafetyError>) -> DiskInfo {
        DiskInfo {
            path: format!("/dev/{name}").into(),
            name: name.into(),
            size_bytes: 20 * 1024 * 1024 * 1024,
            model: "Test Disk".into(),
            removable: false,
            rejected,
        }
    }

    fn two_disks() -> Vec<DiskInfo> {
        vec![
            disk(
                "sda",
                vec![SafetyError::Mounted {
                    device: "/dev/sda1".into(),
                    target: "/boot".into(),
                }],
            ),
            disk("vda", vec![]),
        ]
    }

    fn online() -> ChecksResult {
        ChecksResult {
            internet: Reachability {
                github: true,
                cache: true,
            },
            clock_note: None,
            on_ac: None,
        }
    }

    fn app() -> App {
        App::new(catalog(), host(), false, two_disks)
    }

    /// An app whose Welcome checks have passed.
    fn ready() -> App {
        let mut a = app();
        a.start();
        a.on_checks(online());
        a
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<Effect> {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn message(app: &App) -> String {
        app.errors
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Walk to the Disk screen, keeping defaults (us layout, no variants).
    fn to_disk(app: &mut App) {
        press(app, KeyCode::Enter); // welcome
        press(app, KeyCode::Enter); // timezone
        press(app, KeyCode::Enter); // locale
        press(app, KeyCode::Enter); // keyboard (us, no variants)
    }

    fn to_summary(app: &mut App) {
        to_disk(app);
        assert_eq!(app.screen, Screen::Disk);
        press(app, KeyCode::Enter);
        press(app, KeyCode::Enter); // options as is
        type_str(app, "alice");
        press(app, KeyCode::Tab);
        type_str(app, "hunter2");
        press(app, KeyCode::Tab);
        type_str(app, "hunter2");
        press(app, KeyCode::Enter);
        press(app, KeyCode::Enter); // hostname "nun"
        assert_eq!(app.screen, Screen::Summary);
    }

    #[test]
    fn start_requests_the_checks() {
        let mut a = app();
        let fx = a.start();
        assert!(matches!(fx[..], [Effect::RunChecks { internet: true }]));
        assert!(a.checks.running);
    }

    #[test]
    fn welcome_blocks_until_checks_pass() {
        let mut a = app();
        a.start();
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Welcome, "still running");

        a.on_checks(ChecksResult {
            internet: Reachability {
                github: true,
                cache: false,
            },
            ..online()
        });
        assert!(!a.can_continue());
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Welcome, "no internet");

        a.on_checks(online());
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Timezone);
    }

    #[test]
    fn welcome_needs_a_usable_disk() {
        let mut a = App::new(catalog(), host(), false, || vec![two_disks().remove(0)]);
        a.start();
        a.on_checks(online());
        assert!(!a.has_usable_disk());
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Welcome);
    }

    #[test]
    fn offline_mode_skips_the_internet_requirement() {
        let mut a = App::new(catalog(), host(), true, two_disks);
        let fx = a.start();
        assert!(matches!(fx[..], [Effect::RunChecks { internet: false }]));
        a.on_checks(ChecksResult {
            internet: Reachability {
                github: false,
                cache: false,
            },
            ..online()
        });
        assert!(a.can_continue());
    }

    #[test]
    fn r_rechecks_and_q_quits() {
        let mut a = ready();
        let fx = press(&mut a, KeyCode::Char('r'));
        assert!(matches!(fx[..], [Effect::RunChecks { internet: true }]));
        assert!(a.checks.running);
        assert!(!a.can_continue());
        let fx = press(&mut a, KeyCode::Char('q'));
        assert!(matches!(fx[..], [Effect::Quit]));
    }

    #[test]
    fn typing_filters_and_choice_reaches_answers() {
        let mut a = ready();
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "zur");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Locale);
        assert_eq!(a.answers().system.timezone, "Europe/Zurich");
    }

    #[test]
    fn esc_clears_filter_before_going_back() {
        let mut a = ready();
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "zur");
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.screen, Screen::Timezone);
        assert_eq!(a.timezone.filter(), "");
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.screen, Screen::Welcome);
    }

    #[test]
    fn empty_filter_result_cannot_be_submitted() {
        let mut a = ready();
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "zzz");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Timezone);
        assert!(!a.errors.is_empty());
    }

    fn to_keyboard(a: &mut App) {
        for _ in 0..3 {
            press(a, KeyCode::Enter);
        }
        assert_eq!(a.screen, Screen::Keyboard);
    }

    #[test]
    fn layout_without_variants_confirms_straight_away_and_applies_the_keymap() {
        let mut a = ready();
        to_keyboard(&mut a);
        let fx = press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Disk);
        let [Effect::ApplyKeymap { layout, variant }] = &fx[..] else {
            panic!("expected keymap effect, got {fx:?}");
        };
        assert_eq!((layout.as_str(), variant.as_deref()), ("us", None));
    }

    #[test]
    fn variant_list_follows_the_highlighted_layout() {
        let mut a = ready();
        to_keyboard(&mut a);
        assert_eq!(a.variant.visible().len(), 1, "us has only (default)");
        type_str(&mut a, "switz");
        assert_eq!(a.layout_value(), "ch");
        let names: Vec<_> = a
            .variant
            .visible()
            .iter()
            .map(|i| i.value.clone())
            .collect();
        assert_eq!(names, ["", "fr"]);
        assert_eq!(a.layout_description(), "German (Switzerland)");
    }

    #[test]
    fn enter_on_a_layout_with_variants_moves_to_the_variant_pane_then_confirms() {
        let mut a = ready();
        to_keyboard(&mut a);
        type_str(&mut a, "switz");
        assert!(press(&mut a, KeyCode::Enter).is_empty());
        assert_eq!(a.screen, Screen::Keyboard);
        assert_eq!(a.pane, Pane::Variant);

        type_str(&mut a, "french");
        let fx = press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Disk);
        let [Effect::ApplyKeymap { layout, variant }] = &fx[..] else {
            panic!("expected keymap effect, got {fx:?}");
        };
        assert_eq!((layout.as_str(), variant.as_deref()), ("ch", Some("fr")));
        assert_eq!(a.answers().system.keyboard_variant.as_deref(), Some("fr"));
    }

    #[test]
    fn default_variant_means_none() {
        let mut a = ready();
        to_keyboard(&mut a);
        type_str(&mut a, "switz");
        press(&mut a, KeyCode::Enter);
        let fx = press(&mut a, KeyCode::Enter);
        assert!(matches!(
            &fx[..],
            [Effect::ApplyKeymap { variant: None, .. }]
        ));
        assert_eq!(a.answers().system.keyboard_variant, None);
    }

    #[test]
    fn tab_switches_panes_and_esc_steps_back_through_them() {
        let mut a = ready();
        to_keyboard(&mut a);
        type_str(&mut a, "switz");
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.pane, Pane::Variant);
        type_str(&mut a, "fr");
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.variant.filter(), "", "esc clears the filter first");
        assert_eq!(a.pane, Pane::Variant);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.pane, Pane::Layout);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.layout.filter(), "");
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.screen, Screen::Locale);
    }

    #[test]
    fn typing_goes_to_the_focused_pane_only() {
        let mut a = ready();
        to_keyboard(&mut a);
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "x");
        assert_eq!(a.variant.filter(), "x");
        assert_eq!(a.layout.filter(), "");
    }

    #[test]
    fn disk_picker_starts_on_first_usable_disk_and_refuses_rejected_ones() {
        let mut a = ready();
        to_disk(&mut a);
        assert_eq!(a.selected_disk().unwrap().name, "vda");
        press(&mut a, KeyCode::Up);
        assert_eq!(a.selected_disk().unwrap().name, "sda");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Disk);
        assert!(message(&a).contains("mounted"));
        assert_eq!(a.errors[0].target, Target::Disk);
    }

    #[test]
    fn encryption_fields_appear_only_when_enabled() {
        let mut a = ready();
        to_disk(&mut a);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::DiskOptions);
        assert_eq!(a.options_fields().len(), 2);
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.options_focus, OptionsField::Encrypt);
        press(&mut a, KeyCode::Char(' '));
        assert!(a.encrypt);
        assert_eq!(a.options_fields().len(), 4);
        press(&mut a, KeyCode::Char(' '));
        assert!(!a.encrypt);
        assert_eq!(a.options_fields().len(), 2);
    }

    #[test]
    fn passphrase_rules_are_enforced_on_the_right_field() {
        let mut a = ready();
        to_disk(&mut a);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Tab);
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::DiskOptions, "empty passphrase");
        assert!(a.error_for(Target::Passphrase).is_some());
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "longenough1");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "different11");
        press(&mut a, KeyCode::Enter);
        assert!(a
            .error_for(Target::PassphraseConfirm)
            .is_some_and(|m| m.contains("do not match")));
        for _ in 0..11 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "longenough1");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::User);
        assert_eq!(
            a.answers().disk.encryption_password.unwrap().expose(),
            "longenough1"
        );
    }

    #[test]
    fn swap_field_takes_digits_only() {
        let mut a = ready();
        to_disk(&mut a);
        press(&mut a, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "2a0b48");
        assert_eq!(a.swap.value(), "2048");
        assert_eq!(a.answers().disk.swap_size_mib, 2048);
    }

    #[test]
    fn user_screen_checks_name_and_matching_passwords_then_keeps_only_the_hash() {
        let mut a = ready();
        to_disk(&mut a);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::User);

        type_str(&mut a, "Alice");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::User);
        assert!(a.error_for(Target::Username).is_some());

        for _ in 0..5 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "alice");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "one");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "two");
        press(&mut a, KeyCode::Enter);
        assert!(a
            .error_for(Target::PasswordConfirm)
            .is_some_and(|m| m.contains("doesn't match")));

        for _ in 0..3 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "one");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Hostname);
        assert!(a.password.is_empty() && a.password_confirm.is_empty());
        assert!(a.answers().user.password_hash.expose().starts_with("$y$"));
    }

    #[test]
    fn invalid_hostname_is_rejected() {
        let mut a = ready();
        to_summary(&mut a);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.screen, Screen::Hostname);
        for _ in 0..3 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "-bad");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Hostname);
        assert!(a.error_for(Target::Hostname).is_some());
    }

    #[test]
    fn install_needs_the_disk_name_typed_and_yields_a_plan() {
        let mut a = ready();
        to_summary(&mut a);
        assert_eq!(a.confirm_word(), "vda");
        assert!(press(&mut a, KeyCode::Enter).is_empty());
        assert_eq!(a.screen, Screen::Summary);
        assert!(a.error_for(Target::Confirm).is_some());

        type_str(&mut a, "vda");
        let fx = press(&mut a, KeyCode::Enter);
        let [Effect::Install(plan)] = &fx[..] else {
            panic!("expected install, got {fx:?}");
        };
        assert_eq!(a.screen, Screen::Installing);
        assert!(plan
            .actions
            .iter()
            .any(|x| x.description.contains("WIPES THE DISK")));
        assert_eq!(a.install.steps.len(), plan.actions.len());
    }

    #[test]
    fn install_refuses_if_disk_became_unusable() {
        use std::cell::Cell;
        use std::rc::Rc;
        let ok = Rc::new(Cell::new(true));
        let flag = ok.clone();
        let mut a = App::new(catalog(), host(), false, move || {
            let rejected = if flag.get() {
                vec![]
            } else {
                vec![SafetyError::SwapActive("/dev/vda".into())]
            };
            vec![disk("vda", rejected)]
        });
        a.start();
        a.on_checks(online());
        to_summary(&mut a);
        type_str(&mut a, "vda");
        ok.set(false);
        assert!(press(&mut a, KeyCode::Enter).is_empty());
        assert_eq!(a.screen, Screen::Summary);
        assert!(message(&a).contains("no longer available"));
    }

    #[test]
    fn ctrl_c_quits_except_while_installing() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut a = app();
        assert!(matches!(a.handle_key(ctrl_c)[..], [Effect::Quit]));
        a.screen = Screen::Installing;
        assert!(a.handle_key(ctrl_c).is_empty());
    }

    #[test]
    fn install_events_drive_progress_and_finish() {
        let mut a = ready();
        to_summary(&mut a);
        type_str(&mut a, "vda");
        press(&mut a, KeyCode::Enter);

        a.on_install_event(InstallEvent::Step(2));
        assert_eq!(a.install.steps[0].1, StepStatus::Done);
        assert_eq!(a.install.steps[2].1, StepStatus::Running);
        assert_eq!(a.install.steps[3].1, StepStatus::Pending);
        a.on_install_event(InstallEvent::Output("hello".into()));
        assert_eq!(a.install.log, ["hello"]);
        a.on_install_event(InstallEvent::Finished(Err("boom".into())));
        assert_eq!(a.screen, Screen::Done);
        assert_eq!(a.install.steps[2].1, StepStatus::Failed);
        assert!(matches!(press(&mut a, KeyCode::Enter)[..], [Effect::Quit]));
    }

    #[test]
    fn success_offers_reboot() {
        let mut a = app();
        a.screen = Screen::Installing;
        a.install.steps = vec![("x".into(), StepStatus::Running)];
        a.on_install_event(InstallEvent::Finished(Ok(())));
        assert_eq!(a.install.steps[0].1, StepStatus::Done);
        assert!(matches!(
            press(&mut a, KeyCode::Enter)[..],
            [Effect::Reboot]
        ));
    }

    #[test]
    fn keymap_result_becomes_a_notice() {
        let mut a = app();
        a.on_keymap_result(Ok("ch (fr)".into()));
        assert_eq!(a.notice.as_deref(), Some("Console keyboard set to ch (fr)"));
        a.on_keymap_result(Err("not a console".into()));
        assert!(a.notice.as_deref().unwrap().contains("not a console"));
    }

    #[test]
    fn step_bar_groups_disk_screens() {
        assert_eq!(Screen::Disk.bar_index(), Screen::DiskOptions.bar_index());
        assert_eq!(BAR[Screen::Welcome.bar_index()].0, "Welcome");
        assert_eq!(BAR[Screen::Summary.bar_index()].0, "Review");
        assert_eq!(BAR[Screen::Done.bar_index()].0, "Done");
        let all: Vec<_> = BAR.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        assert_eq!(
            all, FLOW,
            "every screen is in the bar exactly once, in order"
        );
    }
}
