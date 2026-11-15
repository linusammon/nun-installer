use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroize;

/// A string that never shows up in `Debug` or `Display` output, so it cannot
/// leak through logging or a dry-run. Serializes as a plain string.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_printed() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "<redacted>");
        assert_eq!(format!("{s}"), "<redacted>");
        assert_eq!(s.expose(), "hunter2");
    }

    #[test]
    fn serde_is_transparent() {
        let s: Secret = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(s.expose(), "abc");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"abc\"");
    }
}
