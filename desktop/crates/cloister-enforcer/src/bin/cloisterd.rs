//! `cloisterd`: the Cloister enforcement service (root under launchd on macOS and systemd on
//! Linux; a Windows Service running as LocalSystem on Windows).
//!
//! ```text
//! cloisterd run [--state PATH] [--socket PATH] [--dns ADDR] [--upstream ADDR] [--tick SECS]
//!               [--ritual-flag PATH] [--app PATH] [--backend URL --anon-key KEY] [--dry-run]
//! cloisterd service [run options]    # Windows: the same, under the Service Control Manager
//! cloisterd install --app PATH       # as root/Administrator: copy itself into place, register
//!                                    # the service and the ritual window, start it
//! cloisterd plist --binary PATH      # print the launchd daemon definition
//! cloisterd ritual-plist --app PATH [--flag PATH]   # print the ritual window's launch agent
//! ```
//!
//! While the Shutdown Ritual is due the service keeps a flag file (default
//! `/var/run/cloister-ritual-due` on macOS, `/run/cloister-ritual-due` on Linux; in a dry run
//! only with `--ritual-flag`), which keeps the ritual window's launch agent (macOS) or user unit
//! (Linux) alive. On Windows the service starts the window (`--app`) in the user's session.
//!
//! The service keeps its trusted clock (`clock.json`) and the account it is pinned to
//! (`account.json`) next to the state file. It fetches the account from the backend itself
//! (whatever lifts enforcement, and server time, only come that way): `--backend`/`--anon-key`,
//! else `CLOISTER_API_URL`/`CLOISTER_ANON_KEY`, else `backend.json` (`{"url": .., "anon_key": ..}`)
//! next to the state file, else the values built in (the same variables at build time).
//!
//! `--dry-run` logs each Enforcement instead of touching the system, so the service, its
//! DNS filter (on a high `--dns` port) and IPC can be run without root. Without `--dry-run`
//! it refuses to start unless it is root: a half-applied enforcement is worse than none.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cloister_core::Enforcement;
use cloister_enforcer::account::{AccountLink, Backend};
use cloister_enforcer::clock::TrustedClock;
use cloister_enforcer::dns::DnsFilter;
use cloister_enforcer::service::shared_domains;
use cloister_enforcer::store::StateStore;
use cloister_enforcer::{ipc, Platform, Service};

#[cfg(target_os = "macos")]
const DEFAULT_STATE: &str = "/Library/Application Support/Cloister/state.json";
#[cfg(target_os = "linux")]
const DEFAULT_STATE: &str = "/var/lib/cloister/state.json";
#[cfg(windows)]
const DEFAULT_STATE: &str = cloister_enforcer::windows::STATE_PATH;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
const DEFAULT_STATE: &str = "/var/lib/cloister/state.json";

struct Options {
    state: PathBuf,
    socket: PathBuf,
    dns: SocketAddr,
    upstreams: Vec<SocketAddr>,
    tick: Duration,
    ritual_flag: Option<PathBuf>,
    /// The desktop app, which shows the Shutdown Ritual (`--ritual`). Used on Windows.
    app: Option<String>,
    backend: Option<(String, String)>,
    dry_run: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => parse_run(&args[1..]).and_then(|o| run(o, &AtomicBool::new(false))),
        #[cfg(windows)]
        Some("service") => windows_service_main::start(),
        Some("install") => install(&args[1..]),
        Some("plist") => plist(&args[1..]),
        Some("ritual-plist") => ritual_plist(&args[1..]),
        _ => Err("usage: cloisterd run [options] | cloisterd install --app PATH | cloisterd plist --binary PATH | cloisterd ritual-plist --app PATH".into()),
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
        socket: cloister_enforcer::ipc::DEFAULT_SOCKET.into(),
        dns: "127.0.0.1:53".parse().unwrap(),
        upstreams: vec![],
        tick: Duration::from_secs(5),
        ritual_flag: None,
        app: None,
        backend: None,
        dry_run: false,
    };
    let mut url = None;
    let mut key = None;
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
            "--ritual-flag" => o.ritual_flag = Some(value()?.into()),
            "--app" => o.app = Some(value()?),
            "--backend" => url = Some(value()?),
            "--anon-key" => key = Some(value()?),
            "--dry-run" => o.dry_run = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    o.backend = match (url, key) {
        (Some(u), Some(k)) => Some((u, k)),
        (None, None) => backend_config(&o.state),
        _ => return Err("--backend and --anon-key go together".into()),
    };
    if o.upstreams.is_empty() {
        o.upstreams = vec!["1.1.1.1:53".parse().unwrap(), "9.9.9.9:53".parse().unwrap()];
    }
    Ok(o)
}

/// Run the service until `stop` is set (never, outside a Windows Service).
fn run(o: Options, stop: &AtomicBool) -> Result<(), String> {
    // Before anything touches the system: real enforcement only as root.
    cloister_enforcer::privileges::may_enforce(
        o.dry_run,
        cloister_enforcer::privileges::is_privileged(),
    )?;
    if let Some(dir) = o.state.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let platform: Box<dyn Platform> = if o.dry_run {
        Box::new(LogPlatform::default())
    } else {
        system_platform(&o)?
    };
    let domains = shared_domains();
    let mut service = Service::open(
        StateStore::new(&o.state),
        platform,
        TrustedClock::open(o.state.with_file_name("clock.json")),
        domains.clone(),
    )
    .map_err(|e| format!("loading {}: {e}", o.state.display()))?;
    match &o.backend {
        Some((url, key)) => service.set_account(AccountLink::new(
            Box::new(Backend::new(url, key)?),
            Some(o.state.with_file_name("account.json")),
        )),
        None => eprintln!(
            "cloisterd: no backend configured: rituals, Shutdown Delays and loosening changes won't reach this Device"
        ),
    }
    service.set_ritual_flag(
        o.ritual_flag
            .clone()
            .or_else(|| default_ritual_flag(o.dry_run)),
    );
    service.tick();
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
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(o.tick);
        match service.lock() {
            Ok(mut s) => s.tick(),
            Err(poisoned) => poisoned.into_inner().tick(),
        };
    }
    Ok(())
}

/// The backend from the environment, `backend.json` next to the state, or the build.
fn backend_config(state: &std::path::Path) -> Option<(String, String)> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let (Some(u), Some(k)) = (env("CLOISTER_API_URL"), env("CLOISTER_ANON_KEY")) {
        return Some((u, k));
    }
    #[derive(serde::Deserialize)]
    struct File {
        url: String,
        anon_key: String,
    }
    if let Some(f) = std::fs::read(state.with_file_name("backend.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<File>(&b).ok())
    {
        return Some((f.url, f.anon_key));
    }
    match (
        option_env!("CLOISTER_API_URL"),
        option_env!("CLOISTER_ANON_KEY"),
    ) {
        (Some(u), Some(k)) if !u.is_empty() && !k.is_empty() => Some((u.into(), k.into())),
        _ => None,
    }
}

/// `cloisterd install --app PATH`: as root / Administrator, install this binary as the service.
fn install(args: &[String]) -> Result<(), String> {
    use cloister_enforcer::install::{execute, plan, Os};
    let app = match args {
        [flag, app] if flag == "--app" => app.clone(),
        _ => return Err("usage: cloisterd install --app PATH".into()),
    };
    if !cloister_enforcer::privileges::is_privileged() {
        return Err("installing needs root (Administrator on Windows)".into());
    }
    let me = std::env::current_exe().map_err(|e| format!("finding cloisterd: {e}"))?;
    execute(&plan(Os::current(), &me.to_string_lossy(), &app))?;
    eprintln!("cloisterd: installed and started");
    Ok(())
}

#[cfg(target_os = "macos")]
fn system_platform(o: &Options) -> Result<Box<dyn Platform>, String> {
    Ok(Box::new(cloister_enforcer::macos::MacPlatform::new(
        o.upstreams[0],
    )))
}

#[cfg(target_os = "linux")]
fn system_platform(o: &Options) -> Result<Box<dyn Platform>, String> {
    Ok(Box::new(cloister_enforcer::linux::LinuxPlatform::new(
        o.upstreams[0],
    )))
}

#[cfg(windows)]
fn system_platform(o: &Options) -> Result<Box<dyn Platform>, String> {
    Ok(Box::new(cloister_enforcer::windows::WindowsPlatform::new(
        o.upstreams.clone(),
        o.app.clone(),
    )))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn system_platform(_: &Options) -> Result<Box<dyn Platform>, String> {
    Err("no enforcement for this OS; use --dry-run".into())
}

fn default_ritual_flag(dry_run: bool) -> Option<PathBuf> {
    if dry_run {
        return None;
    }
    if cfg!(target_os = "macos") {
        Some(cloister_enforcer::macos::RITUAL_FLAG_PATH.into())
    } else if cfg!(target_os = "linux") {
        Some(cloister_enforcer::linux::RITUAL_FLAG_PATH.into())
    } else {
        None // Windows: the service starts the window itself.
    }
}

/// Running under the Windows Service Control Manager: `cloisterd service [run options]`.
#[cfg(windows)]
mod windows_service_main {
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    use cloister_enforcer::windows::SERVICE_NAME;

    define_windows_service!(ffi_service_main, service_main);

    pub fn start() -> Result<(), String> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
            .map_err(|e| format!("not started by the Service Control Manager: {e}"))
    }

    fn service_main(_: Vec<OsString>) {
        // The SCM passes the binPath arguments to the process: `service --app PATH ...`.
        let args: Vec<String> = std::env::args().skip(2).collect();
        let stop = Arc::new(AtomicBool::new(false));
        let on_stop = stop.clone();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                on_stop.store(true, Ordering::Relaxed);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else {
            return;
        };
        let report = |state, code| {
            let _ = status.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: if state == ServiceState::Running {
                    ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
                } else {
                    ServiceControlAccept::empty()
                },
                exit_code: ServiceExitCode::Win32(code),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            });
        };
        report(ServiceState::Running, 0);
        let result = super::parse_run(&args).and_then(|o| super::run(o, &stop));
        if let Err(e) = &result {
            eprintln!("cloisterd: {e}");
        }
        // A non-zero exit makes the SCM's recovery actions restart the service.
        report(ServiceState::Stopped, if result.is_ok() { 0 } else { 1 });
    }
}

fn ritual_plist(args: &[String]) -> Result<(), String> {
    match args {
        #[cfg(target_os = "macos")]
        [flag, app, rest @ ..] if flag == "--app" => {
            let path = match rest {
                [] => cloister_enforcer::macos::RITUAL_FLAG_PATH.to_string(),
                [f, path] if f == "--flag" => path.clone(),
                _ => return Err("usage: cloisterd ritual-plist --app PATH [--flag PATH]".into()),
            };
            print!(
                "{}",
                cloister_enforcer::macos::ritual_agent_plist(app, &path)
            );
            Ok(())
        }
        _ => Err("usage: cloisterd ritual-plist --app PATH [--flag PATH]".into()),
    }
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
