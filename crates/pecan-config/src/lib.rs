//! Portable, bounded preferences and recent paths; desktop scheduling stays in the app.
mod preferences;
pub mod recent;
pub mod startup;
pub use preferences::{
    PreferencesData, config_directory, log_directory, read_config_with_legacy, recovery_directory,
    save_preferences,
};

/// A configuration serialization or filesystem error with its original cause.
#[derive(Debug, thiserror::Error)]
#[error("configuration save failed: {source:#}")]
pub struct ConfigError {
    #[source]
    source: anyhow::Error,
}
impl ConfigError {
    pub(crate) fn new(source: anyhow::Error) -> Self {
        Self { source }
    }
}
