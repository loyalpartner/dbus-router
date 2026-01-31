//! Error types for dbus-router.

use thiserror::Error;

/// Result alias for dbus-router operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Error type for dbus-router.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Glob pattern error: {0}")]
    Glob(#[from] glob::PatternError),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("zvariant error: {0}")]
    Zvariant(#[from] zvariant::Error),

    #[error("D-Bus protocol error: {0}")]
    Protocol(String),

    #[error("Authentication error: {0}")]
    Auth(String),
}
