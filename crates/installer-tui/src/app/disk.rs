//! The disk screen: which disk, how much swap, whether to encrypt.

use super::{text, Errors, Flow, Target};
use crate::widgets::TextInput;
use crossterm::event::{KeyCode, KeyEvent};
use installer_core::{check_encryption, check_swap_size, Disk, DiskInfo, Secret};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskField {
    Disk,
    Swap,
    Encrypt,
    Passphrase,
    Confirm,
}

const SWAP_NOT_A_NUMBER: &str = "enter a number of MiB (0 for none)";

pub struct DiskForm {
    rescan: Box<dyn Fn() -> Vec<DiskInfo>>,
    disks: Vec<DiskInfo>,
    selected: usize,
    swap: TextInput,
    encrypt: bool,
    passphrase: TextInput,
    passphrase_confirm: TextInput,
    focus: DiskField,
    errors: Errors,
}

impl DiskForm {
    pub fn new(rescan: impl Fn() -> Vec<DiskInfo> + 'static) -> Self {
        let disks = rescan();
        let selected = disks.iter().position(DiskInfo::selectable).unwrap_or(0);
        Self {
            rescan: Box::new(rescan),
            disks,
            selected,
            swap: TextInput::new("0"),
            encrypt: false,
            passphrase: TextInput::masked(),
            passphrase_confirm: TextInput::masked(),
            focus: DiskField::Disk,
            errors: Errors::default(),
        }
    }

    pub fn disks(&self) -> &[DiskInfo] {
        &self.disks
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&DiskInfo> {
        self.disks.get(self.selected)
    }

    pub fn has_usable(&self) -> bool {
        self.disks.iter().any(DiskInfo::selectable)
    }

    pub fn swap(&self) -> &TextInput {
        &self.swap
    }

    pub fn encrypt(&self) -> bool {
        self.encrypt
    }

    pub fn passphrase(&self) -> &TextInput {
        &self.passphrase
    }

    pub fn passphrase_confirm(&self) -> &TextInput {
        &self.passphrase_confirm
    }

    pub fn focus(&self) -> DiskField {
        self.focus
    }

    pub fn errors(&self) -> &Errors {
        &self.errors
    }

    pub fn fields(&self) -> Vec<DiskField> {
        let mut f = vec![DiskField::Disk, DiskField::Swap, DiskField::Encrypt];
        if self.encrypt {
            f.extend([DiskField::Passphrase, DiskField::Confirm]);
        }
        f
    }

    /// The disk answers as entered so far.
    pub fn answers(&self) -> Disk {
        Disk {
            device: self
                .selected()
                .map(|d| d.path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            swap_size_mib: self.swap.value().parse().unwrap_or(0),
            encrypt: self.encrypt,
            encryption_password: self.encrypt.then(|| Secret::new(self.passphrase.value())),
        }
    }

    /// Look at the disks again. The highlighted disk stays highlighted if it is
    /// still there, whatever its position in the new list; otherwise the first
    /// usable one is picked.
    pub fn rescan(&mut self) {
        let keep = self.selected().map(|d| d.path.clone());
        self.disks = (self.rescan)();
        self.selected = keep
            .and_then(|path| self.disks.iter().position(|d| d.path == path))
            .or_else(|| self.disks.iter().position(DiskInfo::selectable))
            .unwrap_or(0);
    }

    /// Why the highlighted disk cannot be used, if it cannot.
    fn disk_problem(&self) -> Option<String> {
        match self.selected() {
            None => Some("no disk found, press r to scan again".into()),
            Some(d) if !d.selectable() => Some(
                d.rejected
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            Some(_) => None,
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let fields = self.fields();
        let i = fields.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let n = fields.len() as isize;
        self.focus = fields[(i + delta).rem_euclid(n) as usize];
    }

    /// Enter on a field checks it and moves to the next one; only the last
    /// field submits the screen.
    fn enter(&mut self) -> Flow {
        match self.focus {
            DiskField::Disk => match self.disk_problem() {
                Some(problem) => self.errors.set(Target::Disk, problem),
                None => self.focus = DiskField::Swap,
            },
            DiskField::Swap => {
                if self.swap.value().parse::<u32>().is_err() {
                    self.errors.set(Target::Swap, SWAP_NOT_A_NUMBER);
                } else {
                    self.focus = DiskField::Encrypt;
                }
            }
            DiskField::Encrypt if self.encrypt => self.focus = DiskField::Passphrase,
            DiskField::Passphrase => self.focus = DiskField::Confirm,
            DiskField::Encrypt | DiskField::Confirm => return self.submit(),
        }
        Flow::Stay
    }

    /// Next means the whole screen is valid.
    pub fn key(&mut self, key: &KeyEvent) -> Flow {
        self.errors.clear();
        match (self.focus, key.code) {
            (_, KeyCode::Esc) => return Flow::Back,
            (_, KeyCode::Enter) => return self.enter(),
            (_, KeyCode::Tab) => self.move_focus(1),
            (_, KeyCode::BackTab) => self.move_focus(-1),

            // In the list the arrows pick a disk; leaving it is Tab or Enter.
            (DiskField::Disk, KeyCode::Up) => self.selected = self.selected.saturating_sub(1),
            (DiskField::Disk, KeyCode::Down) => {
                self.selected = (self.selected + 1).min(self.disks.len().saturating_sub(1));
            }
            (DiskField::Disk, KeyCode::Char('r')) => self.rescan(),

            // In the fields the arrows move between them.
            (_, KeyCode::Up) => self.move_focus(-1),
            (_, KeyCode::Down) => self.move_focus(1),

            (DiskField::Swap, KeyCode::Backspace) => self.swap.pop(),
            (DiskField::Passphrase, KeyCode::Backspace) => self.passphrase.pop(),
            (DiskField::Confirm, KeyCode::Backspace) => self.passphrase_confirm.pop(),
            (focus, _) => match (focus, text(key)) {
                (DiskField::Encrypt, Some(' ')) => {
                    self.encrypt = !self.encrypt;
                    if !self.encrypt {
                        self.passphrase.clear();
                        self.passphrase_confirm.clear();
                    }
                }
                (DiskField::Swap, Some(c)) if c.is_ascii_digit() => {
                    // The default is "0"; typing a size replaces it.
                    if self.swap.value() == "0" {
                        self.swap.clear();
                    }
                    self.swap.push(c);
                }
                (DiskField::Passphrase, Some(c)) => self.passphrase.push(c),
                (DiskField::Confirm, Some(c)) => self.passphrase_confirm.push(c),
                _ => {}
            },
        }
        Flow::Stay
    }

    /// Check the whole screen and move on, or focus the first problem.
    fn submit(&mut self) -> Flow {
        let mut errors = Errors::default();
        if let Some(problem) = self.disk_problem() {
            errors.push(Target::Disk, problem);
        }
        match self.swap.value().parse::<u32>() {
            Err(_) => errors.push(Target::Swap, SWAP_NOT_A_NUMBER),
            Ok(mib) => {
                if let Err(m) = check_swap_size(mib) {
                    errors.push(Target::Swap, m);
                }
            }
        }
        let passphrase = self.encrypt.then(|| self.passphrase.value());
        if let Err(m) = check_encryption(self.encrypt, passphrase) {
            errors.push(Target::Passphrase, m);
        }
        if self.encrypt && self.passphrase.value() != self.passphrase_confirm.value() {
            errors.push(Target::PassphraseConfirm, "passphrases do not match");
        }
        let Some(first) = errors.first_target() else {
            return Flow::Next;
        };
        self.focus = match first {
            Target::Disk => DiskField::Disk,
            Target::Swap => DiskField::Swap,
            Target::Passphrase => DiskField::Passphrase,
            _ => DiskField::Confirm,
        };
        self.errors = errors;
        Flow::Stay
    }
}
