//! Lists of valid timezones, locales and keyboard layouts, used both to fill
//! the TUI pickers and to validate answers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub name: String,
    pub description: String,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    /// Sorted, e.g. `Europe/Berlin`.
    pub timezones: Vec<String>,
    /// Sorted UTF-8 locales, e.g. `en_US.UTF-8`.
    pub locales: Vec<String>,
    pub layouts: Vec<Layout>,
}

#[derive(Debug, thiserror::Error)]
#[error("failed to read {}: {source}", path.display())]
pub struct CatalogError {
    path: PathBuf,
    source: std::io::Error,
}

/// Where to read the raw lists from. The Nix package points these at store
/// paths via environment variables so they match the installed release.
#[derive(Debug, Clone)]
pub struct CatalogPaths {
    /// tzdata `zone.tab`.
    pub zone_tab: PathBuf,
    /// glibc `i18n/SUPPORTED`.
    pub locales_supported: PathBuf,
    /// xkeyboard-config `rules/base.lst`.
    pub xkb_base_lst: PathBuf,
}

impl CatalogPaths {
    pub fn from_env() -> Self {
        let get = crate::proc::env_path;
        Self {
            zone_tab: get("NUN_ZONE_TAB", "/usr/share/zoneinfo/zone.tab"),
            locales_supported: get("NUN_LOCALE_SUPPORTED", "/usr/share/i18n/SUPPORTED"),
            xkb_base_lst: get("NUN_XKB_BASE_LST", "/usr/share/X11/xkb/rules/base.lst"),
        }
    }
}

impl Catalog {
    pub fn load(paths: &CatalogPaths) -> Result<Self, CatalogError> {
        let read = |p: &Path| {
            std::fs::read_to_string(p).map_err(|source| CatalogError {
                path: p.to_path_buf(),
                source,
            })
        };
        Ok(Self::from_sources(
            &read(&paths.zone_tab)?,
            &read(&paths.locales_supported)?,
            &read(&paths.xkb_base_lst)?,
        ))
    }

    pub fn from_sources(zone_tab: &str, locales_supported: &str, xkb_base_lst: &str) -> Self {
        Self {
            timezones: parse_zone_tab(zone_tab),
            locales: parse_locales(locales_supported),
            layouts: parse_xkb(xkb_base_lst),
        }
    }

    pub fn has_timezone(&self, tz: &str) -> bool {
        self.timezones
            .binary_search_by(|t| t.as_str().cmp(tz))
            .is_ok()
    }

    pub fn has_locale(&self, locale: &str) -> bool {
        self.locales
            .binary_search_by(|l| l.as_str().cmp(locale))
            .is_ok()
    }

    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts.iter().find(|l| l.name == name)
    }
}

fn parse_zone_tab(src: &str) -> Vec<String> {
    let mut zones: Vec<String> = src
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::to_owned)
        .collect();
    zones.push("UTC".into());
    zones.sort();
    zones.dedup();
    zones
}

/// Keeps `xx_YY.UTF-8/UTF-8` entries and returns the locale name.
fn parse_locales(src: &str) -> Vec<String> {
    let mut locales: Vec<String> = src
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("SUPPORTED-LOCALES"))
        .filter_map(|l| l.trim().trim_end_matches('\\').trim().split_once('/'))
        .filter(|(name, charset)| name.ends_with(".UTF-8") && *charset == "UTF-8")
        .map(|(name, _)| name.to_owned())
        .collect();
    locales.sort();
    locales.dedup();
    locales
}

fn parse_xkb(src: &str) -> Vec<Layout> {
    let mut section = "";
    let mut layouts: Vec<Layout> = Vec::new();
    let mut variants: BTreeMap<String, Vec<Variant>> = BTreeMap::new();

    for line in src.lines() {
        if let Some(name) = line.strip_prefix("! ") {
            section = name.trim();
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim();
        match section {
            "layout" => layouts.push(Layout {
                name: name.to_owned(),
                description: rest.to_owned(),
                variants: Vec::new(),
            }),
            "variant" => {
                if let Some((layout, description)) = rest.split_once(": ") {
                    variants
                        .entry(layout.to_owned())
                        .or_default()
                        .push(Variant {
                            name: name.to_owned(),
                            description: description.to_owned(),
                        });
                }
            }
            _ => {}
        }
    }
    for layout in &mut layouts {
        layout.variants = variants.remove(&layout.name).unwrap_or_default();
    }
    layouts
}

#[cfg(test)]
pub(crate) fn test_catalog() -> Catalog {
    Catalog::from_sources(
        "# c\nDE\t+5230+01322\tEurope/Berlin\nCH\t+4723+00832\tEurope/Zurich\n",
        "# c\nSUPPORTED-LOCALES=\\\nde_CH.UTF-8/UTF-8 \\\nde_CH/ISO-8859-1 \\\nen_US.UTF-8/UTF-8 \\\n",
        "! model\n  pc105  Generic 105-key PC\n\n! layout\n  us  English (US)\n  ch  German (Switzerland)\n\n! variant\n  intl  us: English (US, intl., with dead keys)\n  de_nodeadkeys  ch: German (Switzerland, no dead keys)\n  fr  ch: French (Switzerland)\n\n! option\n  grp  Switching\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timezones() {
        let c = test_catalog();
        assert_eq!(c.timezones, ["Europe/Berlin", "Europe/Zurich", "UTC"]);
        assert!(c.has_timezone("Europe/Berlin"));
        assert!(!c.has_timezone("Europe/Atlantis"));
    }

    #[test]
    fn parses_only_utf8_locales() {
        let c = test_catalog();
        assert_eq!(c.locales, ["de_CH.UTF-8", "en_US.UTF-8"]);
        assert!(c.has_locale("en_US.UTF-8"));
        assert!(!c.has_locale("de_CH"));
    }

    #[test]
    fn parses_layouts_and_variants() {
        let c = test_catalog();
        let names: Vec<_> = c.layouts.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["us", "ch"]);
        let ch = c.layout("ch").unwrap();
        assert_eq!(ch.description, "German (Switzerland)");
        let v: Vec<_> = ch.variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(v, ["de_nodeadkeys", "fr"]);
        assert_eq!(ch.variants[1].description, "French (Switzerland)");
        assert!(c.layout("xx").is_none());
    }
}
