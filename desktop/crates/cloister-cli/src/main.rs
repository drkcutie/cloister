//! `cloister`: developer CLI that talks to the enforcement service over its IPC socket.
//!
//! ```text
//! cloister block start --minutes N [--plain] [--confirm-long] [--blocklist ID]...
//! cloister block extend --id ID --until 2026-01-05T11:00:00Z [--confirm-long]
//! cloister status
//! ```
//! Every command takes `--socket PATH` (default `$CLOISTER_SOCKET` or `/var/run/cloister.sock`).
//! There is no command to stop a Block.

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{DateTime, Local, Utc};
use cloister_core::Command;
use cloister_enforcer::ipc::{self, Request, Response};

const USAGE: &str = "usage:
  cloister block start --minutes N [--plain] [--confirm-long] [--blocklist ID]...
  cloister block extend --id ID --until RFC3339 [--confirm-long]
  cloister status
options: --socket PATH";

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

#[derive(Default)]
struct Flags {
    socket: Option<PathBuf>,
    minutes: Option<i64>,
    plain: bool,
    confirm_long: bool,
    blocklists: Vec<String>,
    id: Option<String>,
    until: Option<DateTime<Utc>>,
}

fn parse(args: &[String]) -> Result<Flags, String> {
    let mut f = Flags::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--socket" => f.socket = Some(value()?.into()),
            "--minutes" => {
                f.minutes = Some(
                    value()?
                        .parse()
                        .map_err(|_| "--minutes takes a whole number")?,
                )
            }
            "--plain" => f.plain = true,
            "--confirm-long" => f.confirm_long = true,
            "--blocklist" => f.blocklists.push(value()?),
            "--id" => f.id = Some(value()?),
            "--until" => f.until = Some(value()?.parse().map_err(|e| format!("--until: {e}"))?),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    Ok(f)
}

fn run(args: &[String]) -> Result<(), String> {
    let (verb, rest): (Vec<&str>, &[String]) = match args {
        [a, b, rest @ ..] if a == "block" => (vec![a.as_str(), b.as_str()], rest),
        [a, rest @ ..] => (vec![a.as_str()], rest),
        [] => return Err(USAGE.into()),
    };
    let f = parse(rest)?;
    let socket = f
        .socket
        .clone()
        .or_else(|| std::env::var_os("CLOISTER_SOCKET").map(PathBuf::from))
        .unwrap_or_else(|| "/var/run/cloister.sock".into());
    let request = match verb.as_slice() {
        ["block", "start"] => Request::Command {
            command: Command::StartBlock {
                block_id: new_block_id(),
                minutes: f.minutes.ok_or("--minutes is required")?,
                deep: !f.plain,
                blocklists: f.blocklists,
                confirm_long: f.confirm_long,
                client_time: Some(Utc::now()),
            },
        },
        ["block", "extend"] => Request::Command {
            command: Command::ExtendBlock {
                block_id: f.id.ok_or("--id is required")?,
                ends_at: f.until.ok_or("--until is required")?,
                confirm_long: f.confirm_long,
            },
        },
        ["status"] => Request::Status,
        _ => return Err(USAGE.into()),
    };
    let response = ipc::send(&socket, &request).map_err(|e| {
        format!(
            "The enforcement service isn't reachable at {}: {e}",
            socket.display()
        )
    })?;
    match response {
        Response::Accepted { enforcement } => {
            println!(
                "Accepted. Blocking {} domains.",
                enforcement.blocked_domains.len()
            );
            Ok(())
        }
        Response::Rejected { reason } => Err(explain(&reason)),
        Response::Status {
            server_time,
            blocks,
            enforcement,
        } => {
            let active: Vec<_> = blocks.iter().filter(|b| b.is_active(server_time)).collect();
            if active.is_empty() {
                println!("No Block in force.");
            }
            for b in active {
                let kind = if b.deep { "Deep Session" } else { "Block" };
                println!(
                    "{kind} {} ends at {}. Can't be cancelled.",
                    b.id,
                    local(b.ends_at)
                );
            }
            println!("Blocking {} domains.", enforcement.blocked_domains.len());
            Ok(())
        }
    }
}

fn explain(code: &str) -> String {
    let text = match code {
        "invalid_length" => "A Block needs a length of at least one minute.",
        "block_too_long" => "Blocks are at most 24 hours.",
        "confirmation_required" => {
            "Blocks over 4 hours can't be undone. Add --confirm-long to commit."
        }
        "block_cannot_be_shortened" => "A Block can be extended, not shortened.",
        "block_cannot_be_cancelled" | "loosening_not_accepted" => "A Block can't be cancelled.",
        "block_has_ended" => "That Block has already ended.",
        "unknown_block" => "No Block has that id.",
        "unknown_blocklist" => "No Blocklist has that id.",
        "duplicate_block" => "A Block with that id already exists.",
        _ => "",
    };
    if text.is_empty() {
        format!("Not accepted ({code}).")
    } else {
        format!("{text} ({code})")
    }
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M").to_string()
}

fn new_block_id() -> String {
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("blk-{nanos:x}-{:x}", std::process::id())
}
