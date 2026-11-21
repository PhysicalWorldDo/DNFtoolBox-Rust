//! Portable toolbox application services. All managed paths are relative to an explicit workspace.
mod catalog;
mod model;
mod network;
mod paths;
mod service;
mod settings;
mod transaction;

pub use model::{CoreError, CoreResult, EventEnvelope, NativeAction, Request, Response};
pub use paths::validate_entry;
pub use service::Service;

pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    match (semver::Version::parse(left), semver::Version::parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}
