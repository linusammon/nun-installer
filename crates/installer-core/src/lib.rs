//! Core logic for the nun installer: answers -> generated config + install plan.

pub mod answers;
pub mod generate;
pub mod validate;

pub use answers::{Answers, Disk, Filesystem, System, User, SCHEMA_VERSION};
pub use generate::{render, Boot, GeneratedFiles, Host, RenderError, NIXPKGS_URL};
pub use validate::{validate, Field, ValidationError};
