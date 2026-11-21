//! Desktop host support shared with headless acceptance entry points.
pub mod cli;
#[cfg(feature = "core")]
pub mod desktop;
pub mod icons;
pub mod instance;
pub mod native;
pub mod self_update;
