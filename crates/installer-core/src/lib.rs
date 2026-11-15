//! Core logic for the nun installer: answers -> generated config + install plan.

pub mod answers;
pub mod catalog;
pub mod checks;
pub mod disks;
pub mod exec;
pub mod generate;
pub mod keymap;
pub mod lock;
pub mod network;
pub mod password;
pub mod plan;
mod proc;
pub mod safety;
pub mod secret;
pub mod tools;

#[cfg(any(test, feature = "test-util"))]
mod testing;
pub mod validate;

pub use answers::{Answers, Disk, System, User, SCHEMA_VERSION};
pub use catalog::{Catalog, CatalogError, CatalogPaths, Layout, Variant};
pub use checks::{clock_note, on_ac_power, system_clock_note, system_on_ac_power};
pub use disks::{human_size, list_disks, DiskInfo};
pub use exec::{run_plan, DryRun, ExecError, Executor, Live};
pub use generate::{
    render, Boot, GeneratedFiles, Host, HostError, RenderError, DISKO_URL, LUKS_PASSWORD_PATH,
    NIXPKGS_URL,
};
pub use keymap::{apply_console_keymap, KeymapError, KeymapTools};
pub use lock::{target_lock, LockError};
pub use network::{check_online, Reachability};
pub use password::{hash_password, HashError};
pub use plan::{
    build_plan, Action, ActionKind, Command, Content, Plan, PlanError, CONFIG_DIR, MOUNT_ROOT,
    STAGING_DIR,
};
pub use safety::{check_disk, Facts, Mount, SafetyError};
pub use secret::Secret;
pub use tools::{missing_tools, missing_tools_in, REQUIRED_TOOLS};
pub use validate::{
    check_device, check_encryption, check_hostname, check_keyboard_layout, check_keyboard_variant,
    check_locale, check_password_hash, check_swap_size, check_timezone, check_username, validate,
    Field, ValidationError, MAX_SWAP_MIB,
};
