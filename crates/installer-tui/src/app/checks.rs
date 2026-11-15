//! The Welcome screen's checks and their results.

use installer_core::Reachability;

#[derive(Debug, Clone)]
pub struct ChecksResult {
    pub internet: Reachability,
    pub clock_note: Option<String>,
    /// `None`: no battery. `Some(false)`: running on battery.
    pub on_ac: Option<bool>,
    /// Programs the install needs that are not on PATH.
    pub missing_tools: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Checks {
    running: bool,
    result: Option<ChecksResult>,
}

impl Checks {
    pub fn running(&self) -> bool {
        self.running
    }

    pub fn result(&self) -> Option<&ChecksResult> {
        self.result.as_ref()
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn finish(&mut self, result: ChecksResult) {
        self.running = false;
        self.result = Some(result);
    }

    pub fn missing_tools(&self) -> &[String] {
        self.result
            .as_ref()
            .map_or(&[], |r| r.missing_tools.as_slice())
    }

    /// Whether the internet check passed. Offline installs need none.
    pub fn internet_ok(&self, offline: bool) -> bool {
        offline || self.result.as_ref().is_some_and(|r| r.internet.online())
    }

    /// Whether the checks have run and found nothing missing. The disk is
    /// checked elsewhere, as it is the disk form's business.
    pub fn passed(&self, offline: bool) -> bool {
        !self.running
            && self.result.is_some()
            && self.internet_ok(offline)
            && self.missing_tools().is_empty()
    }
}
