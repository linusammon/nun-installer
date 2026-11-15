//! Errors shown next to the widget they belong to.

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
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub target: Target,
    pub message: String,
}

/// The errors of one screen.
#[derive(Debug, Default)]
pub struct Errors(Vec<FieldError>);

impl Errors {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Show this one error and no other.
    pub fn set(&mut self, target: Target, message: impl Into<String>) {
        self.0 = vec![FieldError {
            target,
            message: message.into(),
        }];
    }

    pub fn push(&mut self, target: Target, message: impl Into<String>) {
        self.0.push(FieldError {
            target,
            message: message.into(),
        });
    }

    /// Where the first problem is.
    pub fn first_target(&self) -> Option<Target> {
        self.0.first().map(|e| e.target)
    }

    pub fn get(&self, target: Target) -> Option<&str> {
        self.0
            .iter()
            .find(|e| e.target == target)
            .map(|e| e.message.as_str())
    }

    #[cfg(test)]
    pub fn all(&self) -> &[FieldError] {
        &self.0
    }
}
