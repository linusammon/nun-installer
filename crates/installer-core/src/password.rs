//! Hashing the user's password in memory, so only the hash ever leaves the TUI.

use crate::secret::Secret;
use yescrypt::{PasswordHasher, Yescrypt};

#[derive(Debug, thiserror::Error)]
#[error("failed to hash password: {0}")]
pub struct HashError(String);

/// Hash with yescrypt (the NixOS default) and a fresh random salt.
pub fn hash_password(plain: &str) -> Result<Secret, HashError> {
    let hash = Yescrypt::default()
        .hash_password(plain.as_bytes())
        .map_err(|e| HashError(e.to_string()))?;
    Ok(Secret::new(hash.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yescrypt::PasswordVerifier;

    #[test]
    fn produces_a_yescrypt_hash_that_verifies() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.expose().starts_with("$y$"));
        Yescrypt::default()
            .verify_password(b"correct horse battery", h.expose())
            .unwrap();
        assert!(Yescrypt::default()
            .verify_password(b"wrong", h.expose())
            .is_err());
    }

    /// Produced by libxcrypt: `crypt("hunter2 with spaces", "$y$j9T$li9efvvSEOSTc5NAJFhm5.")`.
    #[test]
    fn verifies_hash_made_by_libxcrypt() {
        let known = "$y$j9T$li9efvvSEOSTc5NAJFhm5.$lVFs7sJE/ajGFKTNEAcXwqD5qW0yHKc3HY1LN.W.BMB";
        Yescrypt::default()
            .verify_password(b"hunter2 with spaces", known)
            .unwrap();
    }

    #[test]
    fn salts_differ() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn passes_validation() {
        let h = hash_password("x").unwrap();
        assert!(h.expose().starts_with("$y$"));
        assert!(!h.expose().contains(char::is_whitespace));
    }
}
