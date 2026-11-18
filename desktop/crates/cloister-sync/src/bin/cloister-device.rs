//! `cloister-device`: a Device without the GUI. Signs in, keeps the local mirror in step with
//! the backend and hands what it knows to the enforcement service. Also the two-Device demo
//! (`scripts/demo-two-devices.sh`).
//!
//! ```text
//! cloister-device sign-in --email ADDRESS [--code 123456]   # magic link (or the emailed code)
//! cloister-device sign-in --google                          # needs Google set up (backend/README.md)
//! cloister-device run [--enforcer SOCKET]                   # sync until stopped
//! cloister-device start-block --minutes N [--plain] [--confirm-long] [--blocklist ID]...
//! cloister-device status
//! cloister-device devices
//! ```
//! Every command takes `--mirror PATH` (default `$CLOISTER_MIRROR`, else the user data dir)
//! and `--name NAME` (default the host name). The backend comes from `CLOISTER_API_URL` and
//! `CLOISTER_ANON_KEY`. There is no command to stop a Block.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use cloister_core::Command;
use cloister_enforcer::ipc::{self, Request, Response};
use cloister_sync::{
    Backend, CommandOutcome, Device, DeviceInfo, LoopbackRedirect, Pkce, Platform, SyncEvent,
    SyncOptions,
};

const USAGE: &str = "usage:
  cloister-device sign-in (--email ADDRESS [--code CODE] | --google)
  cloister-device run [--enforcer SOCKET]
  cloister-device start-block --minutes N [--plain] [--confirm-long] [--blocklist ID]...
  cloister-device status
  cloister-device devices
options: --mirror PATH  --name NAME
env: CLOISTER_API_URL, CLOISTER_ANON_KEY";

#[derive(Default)]
struct Flags {
    mirror: Option<PathBuf>,
    name: Option<String>,
    email: Option<String>,
    code: Option<String>,
    google: bool,
    enforcer: Option<PathBuf>,
    minutes: Option<i64>,
    plain: bool,
    confirm_long: bool,
    blocklists: Vec<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: &[String]) -> Result<Flags, String> {
    let mut f = Flags::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--mirror" => f.mirror = Some(value()?.into()),
            "--name" => f.name = Some(value()?),
            "--email" => f.email = Some(value()?),
            "--code" => f.code = Some(value()?),
            "--google" => f.google = true,
            "--enforcer" => f.enforcer = Some(value()?.into()),
            "--minutes" => {
                f.minutes = Some(value()?.parse().map_err(|_| "--minutes takes a whole number")?)
            }
            "--plain" => f.plain = true,
            "--confirm-long" => f.confirm_long = true,
            "--blocklist" => f.blocklists.push(value()?),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    Ok(f)
}

fn run(args: &[String]) -> Result<(), String> {
    let (cmd, rest) = args.split_first().ok_or(USAGE)?;
    let f = parse(rest)?;
    let backend = Backend::from_env().ok_or("set CLOISTER_API_URL and CLOISTER_ANON_KEY")?;
    let device = open(&backend, &f)?;
    match cmd.as_str() {
        "sign-in" => sign_in(&backend, &device, &f),
        "run" => sync(&device, f.enforcer.as_deref()),
        "start-block" => start_block(&device, &f),
        "status" => status(&device),
        "devices" => devices(&device),
        _ => Err(USAGE.into()),
    }
}

fn open(backend: &Backend, f: &Flags) -> Result<Device, String> {
    let mirror = f
        .mirror
        .clone()
        .or_else(|| std::env::var_os("CLOISTER_MIRROR").map(PathBuf::from))
        .unwrap_or_else(default_mirror);
    let name = f.name.clone().unwrap_or_else(host_name);
    Device::open(
        backend.clone(),
        &mirror,
        DeviceInfo {
            name,
            platform: Platform::current(),
        },
    )
    .map_err(|e| e.to_string())
}

fn default_mirror() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Cloister/mirror.sqlite")
    } else {
        home.join(".local/share/cloister/mirror.sqlite")
    }
}

fn host_name() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "This Device".into())
}

fn sign_in(backend: &Backend, device: &Device, f: &Flags) -> Result<(), String> {
    let session = if let (Some(email), Some(code)) = (&f.email, &f.code) {
        backend.verify_email_code(email, code).map_err(|e| e.to_string())?
    } else {
        let pkce = Pkce::new();
        let redirect = LoopbackRedirect::bind().map_err(|e| e.to_string())?;
        if f.google {
            let url = backend.google_sign_in_url(&redirect.url(), &pkce);
            println!("Opening the browser for Google sign-in:\n{url}");
            let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" })
                .arg(&url)
                .status();
        } else {
            let email = f.email.as_deref().ok_or("sign-in needs --email or --google")?;
            backend
                .send_magic_link(email, &redirect.url(), &pkce)
                .map_err(|e| e.to_string())?;
            println!("Sign-in link sent to {email}. Open it on this computer.");
        }
        let code = redirect
            .wait_for_code(Duration::from_secs(600))
            .map_err(|e| e.to_string())?;
        backend.exchange_code(&code, &pkce).map_err(|e| e.to_string())?
    };
    let email = session.email.clone().unwrap_or_default();
    device.sign_in(session).map_err(|e| e.to_string())?;
    println!("Signed in as {email}. This Device is registered.");
    Ok(())
}

/// Sync until killed. Every change (and every few seconds) the mirror is handed to the
/// enforcement service, which only ever tightens.
fn sync(device: &Device, enforcer: Option<&std::path::Path>) -> Result<(), String> {
    if !device.is_signed_in() {
        return Err("not signed in: run cloister-device sign-in first".into());
    }
    let events = device.events();
    let _sync = device.start_sync(SyncOptions::default());
    let mut last_domains = None;
    loop {
        match events.recv_timeout(Duration::from_secs(5)) {
            Ok(SyncEvent::SyncStatusChanged(s)) => println!("{}", s.label()),
            Ok(SyncEvent::StateChanged) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let state = device.state().map_err(|e| e.to_string())?;
        let mut enforcing = true;
        if let Some(socket) = enforcer {
            match ipc::send(socket, &Request::Sync { blocks: state.blocks.clone() }) {
                Ok(Response::Accepted { .. }) => {}
                Ok(other) => eprintln!("enforcement service: {other:?}"),
                Err(e) => {
                    eprintln!("enforcement service unreachable: {e}");
                    enforcing = false;
                }
            }
        }
        let _ = device.report_enforcing(enforcing);
        let domains = device.enforcement().map_err(|e| e.to_string())?.blocked_domains;
        if last_domains.as_ref() != Some(&domains) {
            if domains.is_empty() {
                println!("Nothing blocked.");
            } else {
                println!("Blocking {} domains: {}", domains.len(), domains.join(" "));
            }
            last_domains = Some(domains);
        }
    }
}

fn start_block(device: &Device, f: &Flags) -> Result<(), String> {
    let command = Command::StartBlock {
        block_id: cloister_sync::new_block_id(),
        minutes: f.minutes.ok_or("start-block needs --minutes")?,
        deep: !f.plain,
        blocklists: f.blocklists.clone(),
        confirm_long: f.confirm_long,
        client_time: Some(chrono::Utc::now()),
    };
    match device.send(&command).map_err(|e| e.to_string())? {
        CommandOutcome::Accepted => {
            println!("Block started. It reaches your other Devices now.");
            Ok(())
        }
        CommandOutcome::Rejected { reason } => Err(format!("Not started ({reason}).")),
    }
}

fn status(device: &Device) -> Result<(), String> {
    let status = match device.pull() {
        Ok(_) => "Online",
        Err(_) => cloister_sync::SyncStatus::Offline.label(),
    };
    println!("{status}");
    let now = device.server_time();
    for b in device.state().map_err(|e| e.to_string())?.blocks.iter().filter(|b| b.is_active(now)) {
        println!(
            "{} until {}. Can't be cancelled.",
            if b.deep { "Deep Session" } else { "Block" },
            b.ends_at.with_timezone(&chrono::Local).format("%H:%M")
        );
    }
    Ok(())
}

fn devices(device: &Device) -> Result<(), String> {
    for d in device.devices().map_err(|e| e.to_string())? {
        let marker = if d.this_device { " (this Device)" } else { "" };
        println!("■ {:<10} {}{marker}", d.status.label(), d.name);
    }
    Ok(())
}
