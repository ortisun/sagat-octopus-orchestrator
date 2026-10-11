//! The development identity provider (gap G10 of feature 001, closed by feature 012).
//!
//! The platform verifies bearer tokens from an OIDC provider and the repository had none, so a
//! developer without one could not use the local stack. This signs a token for any subject of any
//! tenant it is asked for, **which is exactly why it only runs when `SAGAT_ENV=dev`**, listens on
//! loopback unless told that only the machine's own containers can reach it, and answers only to
//! the host names it is configured with. It is a separate binary and image target, in no production
//! image.

pub mod config;
pub mod key;
pub mod server;

pub use config::{Config, ConfigError};
pub use key::{DevKey, KeyError};
pub use server::router;
