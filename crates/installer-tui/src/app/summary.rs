//! The review screen: type the disk's name to confirm the wipe.

use super::{text, Errors, Flow, Target};
use crate::widgets::TextInput;
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Default)]
pub struct Summary {
    confirm: TextInput,
    errors: Errors,
}

impl Summary {
    pub fn confirm(&self) -> &TextInput {
        &self.confirm
    }

    pub fn errors(&self) -> &Errors {
        &self.errors
    }

    pub fn fail(&mut self, message: impl Into<String>) {
        self.errors.set(Target::Confirm, message);
    }

    /// Next means the user pressed Enter; whether the word is right is for the
    /// caller to judge.
    pub fn key(&mut self, key: &KeyEvent) -> Flow {
        self.errors.clear();
        match key.code {
            KeyCode::Esc => {
                self.confirm.clear();
                return Flow::Back;
            }
            KeyCode::Backspace => self.confirm.pop(),
            KeyCode::Enter => return Flow::Next,
            _ => {
                if let Some(c) = text(key) {
                    self.confirm.push(c);
                }
            }
        }
        Flow::Stay
    }
}
