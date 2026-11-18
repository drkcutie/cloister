//! Cloister desktop enforcement service.
//!
//! Everything here except [`macos`] runs unprivileged and is tested that way. The privileged
//! layer is [`Platform`]: one call that applies an Enforcement to the operating system.

pub mod apps;
pub mod clock;
pub mod dns;
pub mod domains;
pub mod hosts;
pub mod ipc;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod pf;
pub mod privileges;
pub mod service;
pub mod store;

pub use service::{Clock, Platform, Service};
