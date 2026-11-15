//! Core logic for the nun installer: answers -> generated config + install plan.

pub mod answers;
pub mod validate;

pub use answers::{Answers, Disk, Filesystem, System, User, SCHEMA_VERSION};
pub use validate::{validate, Field, ValidationError};
