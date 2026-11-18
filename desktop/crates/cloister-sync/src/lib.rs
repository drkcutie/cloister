//! Cloister sync: how a Device talks to the backend.
//!
//! - [`Mirror`]: the local SQLite copy of the account's Blocks. It is the enforcement source,
//!   online or offline, and it only ever tightens (a Block never shortens or disappears).
//! - [`Backend`]: sign-in (email magic link, Google) and the raw REST calls.
//! - [`Device`]: a signed-in Device. Sends commands to the backend, keeps the mirror in step
//!   through Realtime, reports its status and lists the account's Devices.

mod backend;
mod device;
mod loopback;
mod mirror;
mod realtime;

pub use backend::{Backend, Pkce, Session};
pub use device::{
    new_block_id, CommandOutcome, Device, DeviceInfo, DeviceStatus, DeviceSummary, Platform,
    SyncEvent, SyncHandle, SyncOptions, SyncStatus, STALE_AFTER,
};
pub use loopback::LoopbackRedirect;
pub use mirror::Mirror;

/// Everything that can go wrong talking to the backend or the mirror.
#[derive(Debug)]
pub enum Error {
    /// The backend couldn't be reached. Enforcement carries on from the mirror.
    Offline(String),
    /// The backend answered with an error status.
    Http { status: u16, body: String },
    /// Sign-in failed or the session is no longer valid.
    Auth(String),
    /// The local mirror couldn't be read or written.
    Storage(String),
    /// An unexpected answer.
    Protocol(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Offline(e) => write!(f, "offline: {e}"),
            Error::Http { status, body } => write!(f, "backend answered {status}: {body}"),
            Error::Auth(e) => write!(f, "sign-in: {e}"),
            Error::Storage(e) => write!(f, "mirror: {e}"),
            Error::Protocol(e) => write!(f, "unexpected answer: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A random version-4 UUID.
pub(crate) fn new_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("system randomness");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}
