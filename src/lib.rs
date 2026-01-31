//! D-Bus Router Library
//!
//! A dual-upstream D-Bus router that can route messages to different buses
//! based on destination matching rules.
//!
//! # Example
//!
//! ```no_run
//! use dbus_router::{Config, Router};
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> dbus_router::Result<()> {
//!     let config = Config::default();
//!     let router = Router::new(
//!         PathBuf::from("/tmp/proxy.sock"),
//!         "unix:path=/run/user/1000/bus".to_string(),
//!         "unix:path=/tmp/sandbox.sock".to_string(),
//!         config,
//!     );
//!     router.run().await
//! }
//! ```

mod auth;
mod bus;
mod conn;
mod dbus;
mod error;
mod routing;

pub mod config;
pub mod fake_name;
pub mod router;
pub mod session;

// Re-export main types
pub use bus::Bus;
pub use config::{Config, RouteRule};
pub use error::{Error, Result};
pub use router::Router;
