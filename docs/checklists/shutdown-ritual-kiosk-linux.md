# Manual checklist: the Shutdown Ritual window on Linux

Wayland gives no app the power to lock the screen, so on Linux the ritual **keeps reappearing**
instead: it asks to be full screen and in front once a second, and its systemd user unit starts
it again one second after it exits. Run this on a test machine or VM (GNOME on Wayland, and if
you can also KDE Plasma and an X11 session). Run `scripts/linux/smoke-test.sh` first.

## Before you start: your way out

- A virtual console (Ctrl-Alt-F3) or ssh: `sudo rm /run/cloister-ritual-due` and
  `systemctl --user -M $USER@ stop cloister-ritual.service` end it (the service writes the flag
  back on its next tick while the ritual is due; stop `cloisterd.service` too to stop that).
- A **debug** build lifts itself after `CLOISTER_KIOSK_ESCAPE_SECS` (default 120 s).

## Setup

1. Build release: `cargo build --release -p cloister-app -p cloister-enforcer -p cloister-cli`.
2. Install: `sudo target/release/cloisterd install --app $PWD/target/release/cloister-app`
   (or let onboarding do it: one `pkexec` prompt). It writes
   `/etc/systemd/system/cloisterd.service`, `/etc/systemd/user/cloister-ritual.{service,path}`,
   enables the service and, for every user, the path unit.
3. Log out and back in once, so your user manager picks up the globally enabled path unit.
4. Sign in and set settings so Shutdown Time is a few minutes away (as on macOS).

## The window at Shutdown Time

- [ ] Within about 5 s of Shutdown Time `/run/cloister-ritual-due` exists and the ritual window
      appears full screen on the main display (others show "The ritual is on the main display.").
- [ ] Alt-Tab / Super to another app: within about 1 s the ritual is back in front, or (GNOME
      Wayland, which refuses focus stealing) a "Cloister is ready" notice appears. Record which.
- [ ] Super+D / Show Desktop, minimising: the window comes back within about 1 s.
- [ ] Moving to another workspace: record whether the window follows or is waiting on return.
- [ ] Closing the window (Alt-F4, the compositor's close): it is back within about 1–2 s.
- [ ] `pkill -9 cloister-app`: back within about 1–2 s (`Restart=always`, `RestartSec=1`).
- [ ] `systemctl --user stop cloister-ritual.service` and `systemctl --user disable --now
      cloister-ritual.path`: the service starts it again on its next tick (about 5 s).
- [ ] Log out and back in: the ritual is the first window after login.
- [ ] Repeat the "keeps reappearing" checks on X11 (where activation is not refused).

## Completing it

- [ ] Steps 01–05 behave as in the macOS checklist.
- [ ] After completing, the window goes within about 1 s and the flag file is removed.

## Known limits (record whether they hold)

- Wayland: no keyboard grab and no always-on-top; another app can be used between reappearances.
- `systemctl --user mask cloister-ritual.service` stops the service from starting it (a mask is
  the user's own setting). Record it as an escape route; the ritual still lapses only at the
  next Workday Start and Evening Lockdown still blocks.
- Root can stop `cloisterd.service`, as on every OS.

Result: ______ (date, distribution, desktop and session type, build, who ran it, what escaped)
