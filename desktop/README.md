# Desktop (Rust workspace)

The Cargo workspace root is the repository root (`Cargo.toml`); crates live here.

| Crate | What it is |
| --- | --- |
| `crates/cloister-core` | The **rules engine**: one pure interface, `decide(state, command, server_time)` and `enforcement(state, server_time)`. No I/O, no clock. Runs the shared scenarios in `fixtures/rules`. |
| `crates/cloister-enforcer` | The enforcement service, `cloisterd`: local DNS filter, `pf`, hosts file fallback, IPC socket, state persistence. Only `src/macos.rs` needs root. |
| `crates/cloister-cli` | `cloister`, a developer CLI that talks to the service over IPC. |
| `crates/cloister-app-model` | What the desktop app shows and does, with no UI toolkit: the Begin form, the running Deep Session, the Depth Rating prompt, hold-to-commit, calm notices. Tested against a real service over IPC. |
| `crates/cloister-app` | `cloister-app`, the GPUI desktop app and menu-bar item. A thin skin over `cloister-app-model`. |

```sh
cargo test            # rules scenarios + unprivileged service, DNS filter, hosts and pf tests
cargo clippy --all-targets
```

## Desktop app

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer   # Metal shader compiler (full Xcode)
cargo run -p cloister-enforcer --bin cloisterd -- run --dry-run \
  --dns 127.0.0.1:5353 --socket /tmp/cloister.sock --state /tmp/cloister-state.json &
CLOISTER_SOCKET=/tmp/cloister.sock cargo run -p cloister-app
```

Without `CLOISTER_SOCKET` the app talks to the installed service on `/var/run/cloister.sock`.
It starts Blocks and records Depth Ratings only through the service's IPC, so closing or
quitting it changes nothing, and reopening it shows the running Deep Session. GPUI is pulled
in through `gpui-kit` (pinned exactly; bump deliberately). `cargo test -p cloister-app-model`
covers the app's behaviour without compiling GPUI.

Assets in `crates/cloister-app/assets`: the logo and app icon exported from the design canvas
(`logo/*.svg`, plus a 512px PNG of the icon), and the bundled OFL fonts with their licences
(`fonts/`: EB Garamond from octaviopardo/EBGaramond12, Source Sans 3 and Source Code Pro from
adobe-fonts, static instances).

## macOS enforcement

`cloisterd run` (root, launchd daemon `com.cloister.enforcer`, `RunAtLoad` + `KeepAlive`):

1. Loads the state from `/Library/Application Support/Cloister/state.json`.
2. Runs the DNS filter on `127.0.0.1:53`, forwarding allowed names to `1.1.1.1` / `9.9.9.9`
   (`--upstream` to change). Blocked domains, their subdomains and related domains get NXDOMAIN.
3. Serves IPC on `/var/run/cloister.sock` (JSON lines). Only commands that tighten
   enforcement (`start_block`, `extend_block`) are accepted; anything else is refused with
   `loosening_not_accepted`. There is no request that sets state.
4. Every 5 seconds asks the rules engine for the Enforcement and re-applies it: every network
   service's DNS -> `127.0.0.1`, the `pf` anchor `com.apple/cloister` (DNS/DoT only to the
   filter, resolved IPs of blocked domains dropped), and a marked section in `/etc/hosts`.

Time: until sync supplies server time, the service trusts the wall clock only as far as a
monotonic clock (which keeps counting through sleep) allows, so moving the clock forward
doesn't end a Block while the service runs. A clock moved forward while the Mac is off is
not detected yet; server time from the backend closes that gap.

Known limits (SelfControl-level, ADR 0002): DNS-over-HTTPS in browsers that ignore system DNS,
VPNs and proxies are not blocked. Firefox's DoH is switched off via its canary domain.

### Running without root

```sh
cargo run -p cloister-enforcer --bin cloisterd -- run --dry-run \
  --dns 127.0.0.1:5353 --socket /tmp/cloister.sock --state /tmp/cloister-state.json
CLOISTER_SOCKET=/tmp/cloister.sock cargo run -p cloister-cli -- block start --minutes 5
dig @127.0.0.1 -p 5353 m.youtube.com    # NXDOMAIN
```

### Install and the root smoke test (Seam B)

```sh
sudo scripts/macos/install.sh
sudo scripts/macos/smoke-test.sh 3   # starts a real 3-minute Block that can't be cancelled
```

The smoke test checks that blocked domains and subdomains don't resolve (filter and system
resolver), that DNS to another resolver is dropped by `pf`, that the hosts fallback and DNS
settings are in place, that IPC refuses `cancel_block`, and that all of it still holds after
`launchctl kickstart -k`. Surviving a reboot is a manual step it prints at the end.
