use crate::answers::{Answers, SCHEMA_VERSION};
use crate::catalog::Catalog;
use crate::secret::Secret;

/// Which answer a validation error belongs to, so the TUI can show it per field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    SchemaVersion,
    DiskDevice,
    SwapSize,
    EncryptionPassword,
    UserName,
    UserPasswordHash,
    Hostname,
    Locale,
    Timezone,
    KeyboardLayout,
    KeyboardVariant,
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Field::SchemaVersion => "schema version",
            Field::DiskDevice => "disk",
            Field::SwapSize => "swap size",
            Field::EncryptionPassword => "encryption passphrase",
            Field::UserName => "username",
            Field::UserPasswordHash => "password",
            Field::Hostname => "hostname",
            Field::Locale => "language",
            Field::Timezone => "timezone",
            Field::KeyboardLayout => "keyboard layout",
            Field::KeyboardVariant => "keyboard variant",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ValidationError {
    pub field: Field,
    pub message: String,
}

fn err(field: Field, message: impl Into<String>) -> ValidationError {
    ValidationError {
        field,
        message: message.into(),
    }
}

/// Pure validation of answers against the catalog. Checks that need the live
/// system (disk exists, is not mounted, ...) belong to the safety checks, not here.
pub fn validate(a: &Answers, catalog: &Catalog) -> Vec<ValidationError> {
    let mut errs = Vec::new();

    if a.schema_version != SCHEMA_VERSION {
        errs.push(err(
            Field::SchemaVersion,
            format!(
                "unsupported schema version {} (expected {SCHEMA_VERSION})",
                a.schema_version
            ),
        ));
    }
    if let Err(m) = check_device(&a.disk.device) {
        errs.push(err(Field::DiskDevice, m));
    }
    if let Err(m) = check_swap_size(a.disk.swap_size_mib) {
        errs.push(err(Field::SwapSize, m));
    }
    if let Err(m) = check_encryption(
        a.disk.encrypt,
        a.disk.encryption_password.as_ref().map(Secret::expose),
    ) {
        errs.push(err(Field::EncryptionPassword, m));
    }
    if let Err(m) = check_username(&a.user.name) {
        errs.push(err(Field::UserName, m));
    }
    if let Err(m) = check_password_hash(a.user.password_hash.expose()) {
        errs.push(err(Field::UserPasswordHash, m));
    }
    if let Err(m) = check_hostname(&a.system.hostname) {
        errs.push(err(Field::Hostname, m));
    }
    if let Err(m) = check_locale(catalog, &a.system.locale) {
        errs.push(err(Field::Locale, m));
    }
    if let Err(m) = check_timezone(catalog, &a.system.timezone) {
        errs.push(err(Field::Timezone, m));
    }
    match check_keyboard_layout(catalog, &a.system.keyboard_layout) {
        Err(m) => errs.push(err(Field::KeyboardLayout, m)),
        Ok(()) => {
            if let Some(v) = &a.system.keyboard_variant {
                if let Err(m) = check_keyboard_variant(catalog, &a.system.keyboard_layout, v) {
                    errs.push(err(Field::KeyboardVariant, m));
                }
            }
        }
    }
    errs
}

pub const MAX_SWAP_MIB: u32 = 1024 * 1024;

// The per-field checks below are public so that a form can check its own field
// without building a whole `Answers`; [`validate`] is these, applied to every field.
pub fn check_swap_size(mib: u32) -> Result<(), String> {
    if mib > MAX_SWAP_MIB {
        Err(format!("must be at most {MAX_SWAP_MIB} MiB"))
    } else {
        Ok(())
    }
}

pub fn check_timezone(catalog: &Catalog, tz: &str) -> Result<(), String> {
    if catalog.has_timezone(tz) {
        Ok(())
    } else {
        Err("unknown timezone".into())
    }
}

pub fn check_locale(catalog: &Catalog, locale: &str) -> Result<(), String> {
    if catalog.has_locale(locale) {
        Ok(())
    } else {
        Err("not a supported locale".into())
    }
}

pub fn check_keyboard_layout(catalog: &Catalog, layout: &str) -> Result<(), String> {
    if catalog.layout(layout).is_some() {
        Ok(())
    } else {
        Err("unknown keyboard layout".into())
    }
}

pub fn check_keyboard_variant(
    catalog: &Catalog,
    layout: &str,
    variant: &str,
) -> Result<(), String> {
    match catalog.layout(layout) {
        Some(l) if l.variants.iter().any(|v| v.name == variant) => Ok(()),
        _ => Err(format!("unknown variant for layout '{layout}'")),
    }
}
const MIN_ENCRYPTION_PASSWORD_LEN: usize = 8;

pub fn check_encryption(encrypt: bool, password: Option<&str>) -> Result<(), String> {
    match (encrypt, password) {
        (true, None) => Err("required when encryption is enabled".into()),
        (true, Some(p)) if p.chars().count() < MIN_ENCRYPTION_PASSWORD_LEN => Err(format!(
            "must be at least {MIN_ENCRYPTION_PASSWORD_LEN} characters"
        )),
        (false, Some(_)) => Err("must not be set when encryption is disabled".into()),
        _ => Ok(()),
    }
}

pub fn check_device(d: &str) -> Result<(), String> {
    let name = d
        .strip_prefix("/dev/")
        .ok_or("must be a path under /dev/")?;
    if name.is_empty()
        || name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_'))
    {
        return Err("invalid device path".into());
    }
    Ok(())
}

pub fn check_username(n: &str) -> Result<(), String> {
    const RESERVED: &[&str] = &["root", "nobody", "nixbld"];
    if n.is_empty() || n.len() > 32 {
        return Err("must be 1-32 characters".into());
    }
    let mut chars = n.chars();
    let first = chars.next().unwrap_or_default();
    if !(first.is_ascii_lowercase() || first == '_') {
        return Err("must start with a lowercase letter or underscore".into());
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-')) {
        return Err("may only contain a-z, 0-9, '_' and '-'".into());
    }
    if RESERVED.contains(&n) {
        return Err(format!("'{n}' is reserved"));
    }
    Ok(())
}

pub fn check_password_hash(h: &str) -> Result<(), String> {
    const PREFIXES: &[&str] = &["$y$", "$6$", "$5$", "$2b$"];
    if !PREFIXES.iter().any(|p| h.starts_with(p)) {
        return Err("must be a crypt hash (yescrypt, sha512, sha256 or bcrypt)".into());
    }
    if h.chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        return Err("contains invalid characters".into());
    }
    Ok(())
}

/// The same rule NixOS applies to `networking.hostName`: a single DNS label (no
/// domain part), 1-63 characters, alphanumeric at both ends, with `-` and `_`
/// allowed in between. Anything else passes here and then fails when the
/// configuration is evaluated, after the disk has been wiped.
pub fn check_hostname(h: &str) -> Result<(), String> {
    if h.is_empty() || h.len() > 63 {
        return Err("must be 1-63 characters".into());
    }
    if h.contains('.') {
        return Err("must be a single name without dots".into());
    }
    let edge_ok = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    if !edge_ok(h.chars().next()) || !edge_ok(h.chars().next_back()) {
        return Err("must start and end with a letter or digit".into());
    }
    if !h
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("may only contain letters, digits, '-' and '_'".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::answers::*;

    fn good() -> Answers {
        Answers::example()
    }

    fn fields(a: &Answers) -> Vec<Field> {
        validate(a, &Catalog::example())
            .into_iter()
            .map(|e| e.field)
            .collect()
    }

    #[test]
    fn fields_have_readable_names() {
        assert_eq!(Field::UserPasswordHash.to_string(), "password");
        assert_eq!(
            Field::EncryptionPassword.to_string(),
            "encryption passphrase"
        );
        assert!(!Field::KeyboardVariant
            .to_string()
            .contains(char::is_uppercase));
    }

    #[test]
    fn good_answers_pass() {
        assert!(validate(&good(), &Catalog::example()).is_empty());
    }

    #[test]
    fn bad_schema_version() {
        let mut a = good();
        a.schema_version = 99;
        assert_eq!(fields(&a), [Field::SchemaVersion]);
    }

    #[test]
    fn bad_device() {
        for d in ["sda", "/dev/", "/dev/../etc/passwd", "/dev/sd a"] {
            let mut a = good();
            a.disk.device = d.into();
            assert_eq!(fields(&a), [Field::DiskDevice], "{d}");
        }
    }

    #[test]
    fn bad_usernames() {
        for n in ["", "Alice", "1bob", "root", "a b", &"a".repeat(33)] {
            let mut a = good();
            a.user.name = n.into();
            assert_eq!(fields(&a), [Field::UserName], "{n}");
        }
    }

    #[test]
    fn bad_password_hash() {
        for h in ["hunter2", "$y$ a", ""] {
            let mut a = good();
            a.user.password_hash = h.into();
            assert_eq!(fields(&a), [Field::UserPasswordHash], "{h}");
        }
    }

    #[test]
    fn encryption_password_rules() {
        let mut a = good();
        a.disk.encrypt = true;
        assert_eq!(fields(&a), [Field::EncryptionPassword]);
        a.disk.encryption_password = Some("short".into());
        assert_eq!(fields(&a), [Field::EncryptionPassword]);
        a.disk.encryption_password = Some("long-enough-pass".into());
        assert!(validate(&a, &Catalog::example()).is_empty());
        a.disk.encrypt = false;
        assert_eq!(fields(&a), [Field::EncryptionPassword]);
    }

    #[test]
    fn swap_size_limit() {
        let mut a = good();
        a.disk.swap_size_mib = 0;
        assert!(validate(&a, &Catalog::example()).is_empty());
        a.disk.swap_size_mib = u32::MAX;
        assert_eq!(fields(&a), [Field::SwapSize]);
    }

    #[test]
    fn keyboard_variant() {
        let mut a = good();
        a.system.keyboard_variant = None;
        assert!(validate(&a, &Catalog::example()).is_empty());
        for v in ["", "de", "intl"] {
            a.system.keyboard_variant = Some(v.into());
            assert_eq!(fields(&a), [Field::KeyboardVariant], "{v}");
        }
    }

    #[test]
    fn hostnames_must_be_a_single_nixos_acceptable_label() {
        for h in [
            "",
            "-a",
            "a-",
            "_a",
            "a_",
            "a.b",
            "box.example.org",
            "a b",
            "a\u{e9}b",
            &"a".repeat(64),
        ] {
            let mut a = good();
            a.system.hostname = h.into();
            assert_eq!(fields(&a), [Field::Hostname], "{h:?} should be rejected");
        }
        for h in [
            "a",
            "nun",
            "nun-box",
            "Nun-Box1",
            "a_b",
            "a-_-b",
            &"a".repeat(63),
        ] {
            let mut a = good();
            a.system.hostname = h.into();
            assert!(
                validate(&a, &Catalog::example()).is_empty(),
                "{h:?} should be accepted"
            );
        }
    }

    #[test]
    fn locales() {
        for l in ["", "en_US", "en_US.UTF-8/UTF-8", "xx_XX.UTF-8"] {
            let mut a = good();
            a.system.locale = l.into();
            assert_eq!(fields(&a), [Field::Locale], "{l}");
        }
    }

    #[test]
    fn timezones() {
        for t in ["", "Berlin", "Europe/Atlantis", "../etc"] {
            let mut a = good();
            a.system.timezone = t.into();
            assert_eq!(fields(&a), [Field::Timezone], "{t}");
        }
        let mut a = good();
        a.system.timezone = "UTC".into();
        assert!(validate(&a, &Catalog::example()).is_empty());
    }

    #[test]
    fn keyboard_layout_must_exist() {
        let mut a = good();
        a.system.keyboard_layout = "xx".into();
        a.system.keyboard_variant = None;
        assert_eq!(fields(&a), [Field::KeyboardLayout]);
    }

    #[test]
    fn collects_multiple_errors() {
        let mut a = good();
        a.system.hostname = "".into();
        a.user.name = "".into();
        assert_eq!(fields(&a), [Field::UserName, Field::Hostname]);
    }
}
