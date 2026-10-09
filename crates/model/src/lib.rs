//! The toolbox's own state, as plain serde data: what is installed ([`Inventory`]) and how the
//! user wants updates handled ([`Settings`]). The app stores both as JSON in its data directory;
//! this crate does no I/O, and loading never panics on a corrupt or hostile file.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod inventory;
pub mod settings;
pub mod state;

pub use inventory::{Installation, Inventory};
pub use settings::{AppSettings, Channel, Settings};
pub use state::ToolboxState;
// The types the inventory and settings are made of.
pub use artcraft_toolbox_release::{PackageKind, Version};

/// Largest state file accepted (inventory or settings).
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{what} is not valid: {reason}")]
    Corrupt { what: &'static str, reason: String },
    #[error("{what} is too large ({len} bytes, limit {MAX_FILE_BYTES})")]
    TooLarge { what: &'static str, len: usize },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn from_json<T: serde::de::DeserializeOwned>(what: &'static str, text: &str) -> Result<T> {
    if text.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge { what, len: text.len() });
    }
    serde_json::from_str(text).map_err(|e| Error::Corrupt { what, reason: e.to_string() })
}
