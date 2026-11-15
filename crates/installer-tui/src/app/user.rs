//! The user screen: name and password. Only the password's hash is kept.

use super::{text, Errors, Flow, Target};
use crate::widgets::TextInput;
use crossterm::event::{KeyCode, KeyEvent};
use installer_core::{check_username, hash_password, Secret, User};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserField {
    Name,
    Password,
    Confirm,
}

const EMPTY_PASSWORD: &str = "password must not be empty";

pub struct UserForm {
    username: TextInput,
    password: TextInput,
    password_confirm: TextInput,
    focus: UserField,
    hash: Option<Secret>,
    errors: Errors,
}

impl Default for UserForm {
    fn default() -> Self {
        Self {
            username: TextInput::default(),
            password: TextInput::masked(),
            password_confirm: TextInput::masked(),
            focus: UserField::Name,
            hash: None,
            errors: Errors::default(),
        }
    }
}

impl UserForm {
    pub fn username(&self) -> &TextInput {
        &self.username
    }

    pub fn password(&self) -> &TextInput {
        &self.password
    }

    pub fn password_confirm(&self) -> &TextInput {
        &self.password_confirm
    }

    pub fn focus(&self) -> UserField {
        self.focus
    }

    pub fn errors(&self) -> &Errors {
        &self.errors
    }

    /// The user as entered so far; the hash is empty until the screen is submitted.
    pub fn answers(&self) -> User {
        User {
            name: self.username.value().to_owned(),
            password_hash: self.hash.clone().unwrap_or_else(|| Secret::new("")),
        }
    }

    fn move_focus(&mut self, delta: isize) {
        const ORDER: [UserField; 3] = [UserField::Name, UserField::Password, UserField::Confirm];
        let i = ORDER.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        self.focus = ORDER[(i + delta).rem_euclid(3) as usize];
    }

    /// Next means the whole screen is valid and the password is hashed.
    pub fn key(&mut self, key: &KeyEvent) -> Flow {
        self.errors.clear();
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(-1),
            KeyCode::Esc => return Flow::Back,
            KeyCode::Enter => return self.enter(),
            KeyCode::Backspace => match self.focus {
                UserField::Name => self.username.pop(),
                UserField::Password => self.password.pop(),
                UserField::Confirm => self.password_confirm.pop(),
            },
            _ => {
                if let Some(c) = text(key) {
                    match self.focus {
                        UserField::Name => self.username.push(c),
                        UserField::Password => self.password.push(c),
                        UserField::Confirm => self.password_confirm.push(c),
                    }
                }
            }
        }
        Flow::Stay
    }

    /// Enter checks the field and moves to the next one; only the last field
    /// submits the screen.
    fn enter(&mut self) -> Flow {
        match self.focus {
            UserField::Name => match check_username(self.username.value()) {
                Ok(()) => self.focus = UserField::Password,
                Err(m) => self.errors.set(Target::Username, m),
            },
            UserField::Password => {
                if self.password.is_empty() && self.hash.is_none() {
                    self.errors.set(Target::Password, EMPTY_PASSWORD);
                } else {
                    self.focus = UserField::Confirm;
                }
            }
            UserField::Confirm => return self.submit(),
        }
        Flow::Stay
    }

    fn submit(&mut self) -> Flow {
        let mut errors = Errors::default();
        if let Err(m) = check_username(self.username.value()) {
            errors.push(Target::Username, m);
        }
        let keep_existing =
            self.password.is_empty() && self.password_confirm.is_empty() && self.hash.is_some();
        if !keep_existing {
            if self.password.is_empty() {
                errors.push(Target::Password, EMPTY_PASSWORD);
            } else if self.password.value() != self.password_confirm.value() {
                errors.push(
                    Target::PasswordConfirm,
                    "password confirmation doesn't match",
                );
            }
        }
        if let Some(first) = errors.first_target() {
            // Put the cursor on the first problem.
            self.focus = match first {
                Target::Username => UserField::Name,
                Target::Password => UserField::Password,
                _ => UserField::Confirm,
            };
            self.errors = errors;
            return Flow::Stay;
        }
        if !keep_existing {
            match hash_password(self.password.value()) {
                Ok(h) => self.hash = Some(h),
                Err(e) => {
                    self.errors.set(Target::Password, e.to_string());
                    return Flow::Stay;
                }
            }
            // Only the hash is kept from here on.
            self.password.clear();
            self.password_confirm.clear();
        }
        Flow::Next
    }
}
