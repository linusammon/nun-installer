//! Core logic for the nun installer: answers -> generated config + install plan.

pub mod answers;
pub mod catalog;
pub mod exec;
pub mod generate;
pub mod plan;
pub mod safety;
pub mod secret;
pub mod validate;

pub use answers::{Answers, Disk, System, User, SCHEMA_VERSION};
pub use catalog::{Catalog, CatalogError, CatalogPaths, Layout, Variant};
pub use exec::{run_plan, DryRun, ExecError, Executor, Live};
pub use generate::{
    render, Boot, GeneratedFiles, Host, RenderError, LUKS_PASSWORD_PATH, NIXPKGS_URL,
};
pub use plan::{build_plan, Action, ActionKind, Command, Content, Plan};
pub use safety::{check_disk, SafetyError};
pub use secret::Secret;
pub use validate::{validate, Field, ValidationError};
