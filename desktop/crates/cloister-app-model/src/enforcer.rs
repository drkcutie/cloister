use std::io;
use std::path::{Path, PathBuf};

use cloister_enforcer::ipc::{self, Request, Response};

/// The local enforcement service, as the app sees it: one request, one response.
pub trait Enforcer {
    fn send(&self, request: &Request) -> io::Result<Response>;
}

/// The enforcement service on its Unix socket.
#[derive(Debug, Clone)]
pub struct SocketEnforcer {
    path: PathBuf,
}

impl SocketEnforcer {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SocketEnforcer { path: path.into() }
    }

    /// `$CLOISTER_SOCKET`, or the installed service's socket ([`cloister_enforcer::ipc::DEFAULT_SOCKET`]).
    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("CLOISTER_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| cloister_enforcer::ipc::DEFAULT_SOCKET.into()),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Enforcer for SocketEnforcer {
    fn send(&self, request: &Request) -> io::Result<Response> {
        ipc::send(&self.path, request)
    }
}
