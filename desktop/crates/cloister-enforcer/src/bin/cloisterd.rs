//! `cloisterd`: the Cloister enforcement service (runs as root under launchd).
//!
//! ```text
//! cloisterd run [--state PATH] [--socket PATH] [--dns ADDR] [--upstream ADDR] [--tick SECS] [--dry-run]
//! cloisterd plist --binary PATH      # print the launchd daemon definition
//! ```
//!
//! `--dry-run` logs each Enforcement instead of touching the system, so the service, its
//! DNS filter (on a high `--dns` port) and IPC can be run without root.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cloister_core::Enforcement;
use cloister_enforcer::clock::TrustedClock;
use cloister_enforcer::dns::DnsFilter;
use cloister_enforcer::service::shared_domains;
use cloister_enforcer::store::StateStore;
use cloister_enforcer::{ipc, Platform, Service};

const DEFAULT_STATE: &str = "/Library/Application Support/Cloister/state.json";
const DEFAULT_SOCKET: &str = "/var/run/cloister.sock";

struct Options {
    state: PathBuf,
    socket: PathBuf,
    dns: SocketAddr,
    upstreams: Vec<SocketAddr>,
    tick: Duration,
    dry_run: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => parse_run(&args[1..]).and_then(run),
        Some("plist") => plist(&args[1..]),
        _ => Err("usage: cloisterd run [options] | cloisterd plist --binary PATH".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cloisterd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_run(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        state: DEFAULT_STATE.into(),
        socket: DEFAULT_SOCKET.into(),
        dns: "127.0.0.1:53".parse().unwrap(),
        upstreams: vec![],
        tick: Duration::from_secs(5),
        dry_run: false,
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--state" => o.state = value()?.into(),
            "--socket" => o.socket = value()?.into(),
            "--dns" => o.dns = value()?.parse().map_err(|e| format!("--dns: {e}"))?,
            "--upstream" => o
                .upstreams
                .push(value()?.parse().map_err(|e| format!("--upstream: {e}"))?),
            "--tick" => {
                o.tick = Duration::from_secs(value()?.parse().map_err(|e| format!("--tick: {e}"))?)
            }
            "--dry-run" => o.dry_run = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    if o.upstreams.is_empty() {
        o.upstreams = vec!["1.1.1.1:53".parse().unwrap(), "9.9.9.9:53".parse().unwrap()];
    }
    Ok(o)
}

fn run(o: Options) -> Result<(), String> {
    let platform: Box<dyn Platform> = if o.dry_run {
        Box::new(LogPlatform::default())
    } else {
        system_platform(o.upstreams[0])?
    };
    let domains = shared_domains();
    let service = Service::open(
        StateStore::new(&o.state),
        platform,
        TrustedClock::start(),
        domains.clone(),
    )
    .map_err(|e| format!("loading {}: {e}", o.state.display()))?;
    let filter = DnsFilter::spawn(o.dns, o.upstreams.clone(), domains)
        .map_err(|e| format!("DNS filter on {}: {e}", o.dns))?;
    let service = Arc::new(Mutex::new(service));
    ipc::serve(&o.socket, service.clone())
        .map_err(|e| format!("socket {}: {e}", o.socket.display()))?;
    eprintln!(
        "cloisterd: DNS filter on {}, IPC on {}, ticking every {:?}",
        filter.local_addr(),
        o.socket.display(),
        o.tick
    );
    loop {
        std::thread::sleep(o.tick);
        match service.lock() {
            Ok(mut s) => s.tick(),
            Err(poisoned) => poisoned.into_inner().tick(),
        };
    }
}

#[cfg(target_os = "macos")]
fn system_platform(upstream: SocketAddr) -> Result<Box<dyn Platform>, String> {
    Ok(Box::new(cloister_enforcer::macos::MacPlatform::new(
        upstream,
    )))
}

#[cfg(not(target_os = "macos"))]
fn system_platform(_: SocketAddr) -> Result<Box<dyn Platform>, String> {
    Err("no enforcement for this OS yet; use --dry-run".into())
}

fn plist(args: &[String]) -> Result<(), String> {
    match args {
        #[cfg(target_os = "macos")]
        [flag, binary] if flag == "--binary" => {
            print!("{}", cloister_enforcer::macos::launchd_plist(binary));
            Ok(())
        }
        _ => Err("usage: cloisterd plist --binary PATH".into()),
    }
}

#[derive(Default)]
struct LogPlatform {
    last: Option<Enforcement>,
}

impl Platform for LogPlatform {
    fn apply(&mut self, e: &Enforcement) -> io::Result<()> {
        if self.last.as_ref() != Some(e) {
            eprintln!(
                "cloisterd (dry run): blocking {} domain(s): {:?}",
                e.blocked_domains.len(),
                e.blocked_domains
            );
            self.last = Some(e.clone());
        }
        Ok(())
    }
}
