//! The installer's state machine. It knows nothing about the terminal: keys go
//! in, state changes and an [`Effect`] come out, which keeps it testable.

use crate::widgets::{FilterList, Item, TextInput};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use installer_core::{
    build_plan, hash_password, human_size, render, validate, Answers, Catalog, Disk, DiskInfo,
    Field, Host, Plan, Secret, System, User, SCHEMA_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Timezone,
    Locale,
    KeyboardLayout,
    KeyboardVariant,
    Disk,
    DiskOptions,
    User,
    Hostname,
    Summary,
    Installing,
    Done,
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Screen::Welcome => "Welcome",
            Screen::Timezone => "Timezone",
            Screen::Locale => "Language",
            Screen::KeyboardLayout => "Keyboard layout",
            Screen::KeyboardVariant => "Keyboard variant",
            Screen::Disk => "Disk",
            Screen::DiskOptions => "Disk options",
            Screen::User => "User account",
            Screen::Hostname => "Hostname",
            Screen::Summary => "Review and install",
            Screen::Installing => "Installing",
            Screen::Done => "Finished",
        }
    }
}

/// What the event loop has to do after a key press.
#[derive(Debug)]
pub enum Effect {
    None,
    Quit,
    Install(Plan),
    Reboot,
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
    rescan: Box<dyn Fn() -> Vec<DiskInfo>>,
    pub disks: Vec<DiskInfo>,

    pub timezone: FilterList,
    pub locale: FilterList,
    pub layout: FilterList,
    pub variant: FilterList,

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

    pub errors: Vec<String>,
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
    pub fn new(catalog: Catalog, host: Host, rescan: impl Fn() -> Vec<DiskInfo> + 'static) -> Self {
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
                .map(|l| Item::new(&l.name, &format!("{} - {}", l.name, l.description)))
                .collect(),
        );
        layout.select_value("us");
        let disk_selected = disks.iter().position(DiskInfo::selectable).unwrap_or(0);

        Self {
            screen: Screen::Welcome,
            catalog,
            host,
            rescan: Box::new(rescan),
            disks,
            timezone,
            locale,
            layout,
            variant: FilterList::default(),
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
            install: InstallState::default(),
        }
    }

    pub fn set_default_timezone(&mut self, tz: &str) {
        if self.catalog.has_timezone(tz) {
            self.timezone.select_value(tz);
        }
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

    // ---- navigation ----

    /// Screens the user walks through, for the "step n of m" header.
    pub fn flow(&self) -> Vec<Screen> {
        let mut flow = vec![Screen::Timezone, Screen::Locale, Screen::KeyboardLayout];
        if self.has_variants() {
            flow.push(Screen::KeyboardVariant);
        }
        flow.extend([
            Screen::Disk,
            Screen::DiskOptions,
            Screen::User,
            Screen::Hostname,
            Screen::Summary,
        ]);
        flow
    }

    pub fn progress(&self) -> Option<(usize, usize)> {
        let flow = self.flow();
        let i = flow.iter().position(|s| *s == self.screen)?;
        Some((i + 1, flow.len()))
    }

    fn next_screen(&self) -> Screen {
        let flow = self.flow();
        match flow.iter().position(|s| *s == self.screen) {
            Some(i) if i + 1 < flow.len() => flow[i + 1],
            Some(_) => Screen::Installing,
            None => Screen::Timezone,
        }
    }

    fn prev_screen(&self) -> Screen {
        let flow = self.flow();
        match flow.iter().position(|s| *s == self.screen) {
            Some(i) if i > 0 => flow[i - 1],
            _ => Screen::Welcome,
        }
    }

    fn back(&mut self) {
        self.screen = self.prev_screen();
    }

    /// Errors from validating everything, limited to the fields of one screen.
    fn errors_for(&self, fields: &[Field]) -> Vec<String> {
        validate(&self.answers(), &self.catalog)
            .into_iter()
            .filter(|e| fields.contains(&e.field))
            .map(|e| e.message)
            .collect()
    }

    /// Move on if this screen's answers are valid, otherwise show why not.
    fn advance(&mut self, fields: &[Field]) {
        let errors = self.errors_for(fields);
        if !errors.is_empty() {
            self.errors = errors;
            return;
        }
        if self.screen == Screen::KeyboardLayout {
            self.rebuild_variants();
        }
        self.screen = self.next_screen();
    }

    fn rebuild_variants(&mut self) {
        let mut items = vec![Item::new("", "(default)")];
        if let Some(l) = self.catalog.layout(&self.layout_value()) {
            items.extend(
                l.variants
                    .iter()
                    .map(|v| Item::new(&v.name, &format!("{} - {}", v.name, v.description))),
            );
        }
        self.variant = FilterList::new(items);
    }

    // ---- input ----

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return if self.screen == Screen::Installing {
                Effect::None
            } else {
                Effect::Quit
            };
        }
        self.errors.clear();
        match self.screen {
            Screen::Welcome => match key.code {
                KeyCode::Enter => {
                    self.screen = Screen::Timezone;
                    Effect::None
                }
                KeyCode::Char('q') | KeyCode::Esc => Effect::Quit,
                _ => Effect::None,
            },
            Screen::Timezone
            | Screen::Locale
            | Screen::KeyboardLayout
            | Screen::KeyboardVariant => {
                self.list_key(key);
                Effect::None
            }
            Screen::Disk => {
                self.disk_key(key);
                Effect::None
            }
            Screen::DiskOptions => {
                self.options_key(key);
                Effect::None
            }
            Screen::User => {
                self.user_key(key);
                Effect::None
            }
            Screen::Hostname => {
                self.hostname_key(key);
                Effect::None
            }
            Screen::Summary => self.summary_key(key),
            Screen::Installing => Effect::None,
            Screen::Done => match key.code {
                KeyCode::Enter if self.install.result == Some(Ok(())) => Effect::Reboot,
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => Effect::Quit,
                _ => Effect::None,
            },
        }
    }

    pub fn list_mut(&mut self) -> Option<&mut FilterList> {
        match self.screen {
            Screen::Timezone => Some(&mut self.timezone),
            Screen::Locale => Some(&mut self.locale),
            Screen::KeyboardLayout => Some(&mut self.layout),
            Screen::KeyboardVariant => Some(&mut self.variant),
            _ => None,
        }
    }

    fn list_key(&mut self, key: KeyEvent) {
        let screen = self.screen;
        let Some(list) = self.list_mut() else { return };
        match key.code {
            KeyCode::Up => list.up(),
            KeyCode::Down => list.down(),
            KeyCode::PageUp => list.page(-10),
            KeyCode::PageDown => list.page(10),
            KeyCode::Backspace => list.pop_filter(),
            KeyCode::Esc => {
                if list.filter().is_empty() {
                    self.back();
                } else {
                    list.clear_filter();
                }
            }
            KeyCode::Enter => {
                let field = match screen {
                    Screen::Timezone => Field::Timezone,
                    Screen::Locale => Field::Locale,
                    Screen::KeyboardLayout => Field::KeyboardLayout,
                    _ => Field::KeyboardVariant,
                };
                if list.selected().is_none() {
                    self.errors = vec!["nothing matches, edit the filter".into()];
                } else {
                    self.advance(&[field]);
                }
            }
            _ => {
                if let Some(c) = text(&key) {
                    list.push_filter(c);
                }
            }
        }
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
                None => self.errors = vec!["no disk found, press r to scan again".into()],
                Some(d) if !d.selectable() => {
                    self.errors = d.rejected.iter().map(ToString::to_string).collect();
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
            errors.push("swap size must be a number of MiB (0 for none)".to_owned());
        }
        errors.extend(self.errors_for(&[Field::SwapSize, Field::EncryptionPassword]));
        if self.encrypt && self.passphrase.value() != self.passphrase_confirm.value() {
            errors.push("passphrases do not match".into());
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
                errors.push("password must not be empty".into());
            } else if self.password.value() != self.password_confirm.value() {
                errors.push("passwords do not match".into());
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
                    self.errors = vec![e.to_string()];
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

    fn summary_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Esc => {
                self.confirm.clear();
                self.back();
            }
            KeyCode::Backspace => self.confirm.pop(),
            KeyCode::Enter => return self.start_install(),
            _ => {
                if let Some(c) = text(&key) {
                    self.confirm.push(c);
                }
            }
        }
        Effect::None
    }

    fn start_install(&mut self) -> Effect {
        if self.confirm.value() != self.confirm_word() {
            self.errors = vec![format!(
                "type {} to confirm that it will be erased",
                self.confirm_word()
            )];
            return Effect::None;
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
            self.errors = vec![format!(
                "{path} is no longer available, go back and choose again"
            )];
            return Effect::None;
        }

        let answers = self.answers();
        let errors = validate(&answers, &self.catalog);
        if let Some(e) = errors.first() {
            self.errors = vec![format!("{:?}: {}", e.field, e.message)];
            return Effect::None;
        }
        let plan = match render(&answers, &self.host) {
            Ok(files) => build_plan(&answers, &files, &self.host),
            Err(e) => {
                self.errors = vec![e.to_string()];
                return Effect::None;
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
        Effect::Install(plan)
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

    /// One-line description of a disk for lists and the summary.
    pub fn disk_label(d: &DiskInfo) -> String {
        let model = if d.model.is_empty() {
            String::new()
        } else {
            format!("  {}", d.model)
        };
        let removable = if d.removable { "  [removable]" } else { "" };
        format!(
            "{}  {}{model}{removable}",
            d.path.display(),
            human_size(d.size_bytes)
        )
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

    fn app() -> App {
        App::new(catalog(), host(), || {
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
        })
    }

    fn press(app: &mut App, code: KeyCode) -> Effect {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// Walk to the screen after the keyboard pages, keeping defaults (no variant).
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
    fn welcome_starts_or_quits() {
        let mut a = app();
        assert!(matches!(press(&mut a, KeyCode::Char('q')), Effect::Quit));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Timezone);
    }

    #[test]
    fn typing_filters_and_choice_reaches_answers() {
        let mut a = app();
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "zur");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Locale);
        assert_eq!(a.answers().system.timezone, "Europe/Zurich");
    }

    #[test]
    fn esc_clears_filter_before_going_back() {
        let mut a = app();
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
        let mut a = app();
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "zzz");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Timezone);
        assert!(!a.errors.is_empty());
    }

    #[test]
    fn variant_screen_only_for_layouts_that_have_variants() {
        let mut a = app();
        to_disk(&mut a);
        assert_eq!(a.screen, Screen::Disk);

        let mut a = app();
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        // now on keyboard layout
        a.layout.clear_filter();
        type_str(&mut a, "switz");
        assert_eq!(a.layout_value(), "ch");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::KeyboardVariant);
        press(&mut a, KeyCode::Down); // (default) -> fr
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Disk);
        assert_eq!(a.answers().system.keyboard_variant.as_deref(), Some("fr"));
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.screen, Screen::KeyboardVariant);
    }

    #[test]
    fn default_variant_means_none() {
        let mut a = app();
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "switz");
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.answers().system.keyboard_variant, None);
    }

    #[test]
    fn disk_picker_starts_on_first_usable_disk_and_refuses_rejected_ones() {
        let mut a = app();
        to_disk(&mut a);
        assert_eq!(a.selected_disk().unwrap().name, "vda");
        press(&mut a, KeyCode::Up);
        assert_eq!(a.selected_disk().unwrap().name, "sda");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Disk);
        assert!(a.errors[0].contains("mounted"));
    }

    #[test]
    fn encryption_fields_appear_only_when_enabled() {
        let mut a = app();
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
    fn passphrase_rules_are_enforced() {
        let mut a = app();
        to_disk(&mut a);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Tab);
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::DiskOptions, "empty passphrase");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "longenough1");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "different11");
        press(&mut a, KeyCode::Enter);
        assert!(a.errors.iter().any(|e| e.contains("do not match")));
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
        let mut a = app();
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
        let mut a = app();
        to_disk(&mut a);
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::User);

        type_str(&mut a, "Alice");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::User);
        assert!(!a.errors.is_empty());

        for _ in 0..5 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "alice");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "one");
        press(&mut a, KeyCode::Tab);
        type_str(&mut a, "two");
        press(&mut a, KeyCode::Enter);
        assert!(a.errors.iter().any(|e| e.contains("do not match")));

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
        let mut a = app();
        to_summary(&mut a);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.screen, Screen::Hostname);
        for _ in 0..3 {
            press(&mut a, KeyCode::Backspace);
        }
        type_str(&mut a, "-bad");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.screen, Screen::Hostname);
        assert!(!a.errors.is_empty());
    }

    #[test]
    fn install_needs_the_disk_name_typed_and_yields_a_plan() {
        let mut a = app();
        to_summary(&mut a);
        assert_eq!(a.confirm_word(), "vda");
        let wrong = press(&mut a, KeyCode::Enter);
        assert!(matches!(wrong, Effect::None));
        assert_eq!(a.screen, Screen::Summary);

        type_str(&mut a, "vda");
        let Effect::Install(plan) = press(&mut a, KeyCode::Enter) else {
            panic!("expected install");
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
        let mut a = App::new(catalog(), host(), move || {
            let rejected = if flag.get() {
                vec![]
            } else {
                vec![SafetyError::SwapActive("/dev/vda".into())]
            };
            vec![disk("vda", rejected)]
        });
        to_summary(&mut a);
        type_str(&mut a, "vda");
        ok.set(false);
        assert!(matches!(press(&mut a, KeyCode::Enter), Effect::None));
        assert_eq!(a.screen, Screen::Summary);
        assert!(a.errors[0].contains("no longer available"));
    }

    #[test]
    fn ctrl_c_quits_except_while_installing() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut a = app();
        assert!(matches!(a.handle_key(ctrl_c), Effect::Quit));
        a.screen = Screen::Installing;
        assert!(matches!(a.handle_key(ctrl_c), Effect::None));
    }

    #[test]
    fn install_events_drive_progress_and_finish() {
        let mut a = app();
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
        assert!(matches!(press(&mut a, KeyCode::Enter), Effect::Quit));
    }

    #[test]
    fn success_offers_reboot() {
        let mut a = app();
        a.screen = Screen::Installing;
        a.install.steps = vec![("x".into(), StepStatus::Running)];
        a.on_install_event(InstallEvent::Finished(Ok(())));
        assert_eq!(a.install.steps[0].1, StepStatus::Done);
        assert!(matches!(press(&mut a, KeyCode::Enter), Effect::Reboot));
    }

    #[test]
    fn progress_counts_the_variant_step_only_when_present() {
        let mut a = app();
        a.screen = Screen::Disk;
        assert_eq!(a.progress(), Some((4, 8)));
        a.layout.clear_filter();
        for c in "switz".chars() {
            a.layout.push_filter(c);
        }
        assert_eq!(a.progress(), Some((5, 9)));
    }
}
