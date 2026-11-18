# Manual checklist: the Shutdown Ritual kiosk on Windows

The kiosk can't be tested automatically (it takes over the screen and the keyboard). Run this on
a test PC or VM (a Hyper-V or UTM VM with a snapshot is ideal) with a **way out ready first**.
Run `scripts/windows/smoke-test.ps1` first: the enforcement must pass before the kiosk is worth
checking.

## Before you start: your way out

- Ctrl-Alt-Del can't be hooked. From its screen, open Task Manager and end `cloister-app.exe`;
  the service starts it again within about 5 s while the ritual is due.
- To stop it coming back: from an elevated prompt (another machine with `Enter-PSSession`, or
  Ctrl-Alt-Del, Sign out, then sign in as another admin): `sc.exe stop Cloister`.
- A **debug** build of the app has the escape hatch (Ctrl-Alt-Win-Esc is swallowed by the
  hook, so rely on `CLOISTER_KIOSK_ESCAPE_SECS`, default 120 s). The checklist below is about
  the **release** build.

## Setup

1. Build release on Windows: `cargo build --release -p cloister-app -p cloister-enforcer -p cloister-cli`.
2. Install from an elevated PowerShell (or let onboarding do it: one UAC prompt):
   `target\release\cloisterd.exe install --app "$PWD\target\release\cloister-app.exe"`.
   The service `Cloister` runs `C:\Program Files\Cloister\cloisterd.exe service --app ...` and
   restarts one second after it exits (`sc.exe qfailure Cloister`).
3. Sign the app in to an account (`cloister-device sign-in`, or onboarding).
4. Set settings so Shutdown Time is a few minutes away:
   `cloister-device settings --workdays 1,2,3,4,5,6,7 --workday-start 00:00 --shutdown-time HH:MM`.
5. Connect a second display if you have one.

## Warnings and the Shutdown Delay

- [ ] At T−15 and T−5 a notification says "Shutdown Time in 15 minutes, at HH:MM." (and 5).
- [ ] The tray menu shows the Shutdown Delays left; using one moves the ritual 30 minutes later.

## The kiosk at Shutdown Time

- [ ] Within about 5 s of Shutdown Time (the service's tick) the ritual window appears in the
      signed-in user's session, full screen and above the taskbar, on **every display**.
- [ ] The Windows key and Win+D, Win+Tab, Win+E, Win+R do nothing.
- [ ] Alt-Tab and Alt-Esc do nothing.
- [ ] Ctrl-Esc doesn't open Start; Ctrl-Shift-Esc doesn't open Task Manager.
- [ ] Alt-F4 doesn't close the window.
- [ ] Typing in the ritual still works: Tab moves between fields, Ctrl-V pastes, Shift types capitals.
- [ ] Clicking a toast notification, or an app that steals focus (start one from Task Scheduler),
      doesn't stay in front for more than a second (the window re-asserts topmost once a second).
- [ ] Unplugging and re-plugging the second display: within a few seconds the window is back in front.

## It comes back

- [ ] Ctrl-Alt-Del → Task Manager → end `cloister-app.exe`: the ritual is back within about 5 s.
- [ ] `Stop-Process -Name cloisterd -Force` (elevated): the service is restarted within about
      1 s and the ritual stays.
- [ ] Sign out and back in (or restart): the ritual is shown within about 5 s of sign-in.
- [ ] Fast user switching to a second account: the ritual follows the console session (the
      service starts it for whoever is at the console).

## Completing it

- [ ] Steps 01–05 behave as in the macOS checklist (Loose End required, review, Schedule-filled
      draft, Day Plan rules, "Shutdown complete" plus the two-second hold).
- [ ] After completing, the kiosk lifts within about 1 s, and on every other Device within seconds.
- [ ] Completing on another Device instead lifts it here within a few seconds.

## Known limits (record whether they hold)

- Ctrl-Alt-Del, Win+L (lock) and the power menu can't be blocked; the ritual returns afterwards.
- An administrator can stop the service (`sc.exe stop Cloister`); as on macOS, an admin can
  always undo a root service. Breaking Blocks this way needs deliberate effort.
- DNS to the service's upstreams (1.1.1.1, 9.9.9.9) is allowed for every program, because
  Windows Firewall block rules can't exclude one program. A program that hard-codes them can
  bypass the filter (the IP rule and hosts file still apply).
- If something else already listens on 127.0.0.1:53 (Internet Connection Sharing, a local DNS
  server), the filter can't start; the service log says so.

Result: ______ (date, Windows version, build, who ran it, anything that escaped)
