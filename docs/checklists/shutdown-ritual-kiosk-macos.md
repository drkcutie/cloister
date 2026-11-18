# Manual checklist: the Shutdown Ritual kiosk on macOS

The kiosk can't be tested automatically (it takes over the screen and the keyboard). Run this
on a test Mac, or on your own with a **way out ready first**.

## Before you start: your way out

- Keep a second machine with `ssh` access to this Mac (Remote Login on), or an open terminal
  you can reach. `pkill -9 -x cloister-app` ends the kiosk at once: macOS drops presentation
  options when the process that set them exits.
- To stop it coming back, remove the flag and the agent:
  `sudo rm /var/run/cloister-ritual-due; launchctl bootout gui/$(id -u)/com.cloister.ritual`.
- A **debug** build has an escape hatch: Control-Option-Command-Escape quits, and the kiosk lifts
  itself after `CLOISTER_KIOSK_ESCAPE_SECS` (default 120 s). The checklist below is about the
  **release** build, which has neither. Try your way out once with a debug build first.

## Setup

1. Build release: `cargo build --release -p cloister-app -p cloister-enforcer -p cloister-cli`.
2. Install: `sudo CLOISTER_APP=<target>/release/cloister-app scripts/macos/install.sh`.
   It writes `/Library/LaunchAgents/com.cloister.ritual.plist` (root-owned) and the daemon.
3. Sign the app in to an account (`cloister-device sign-in`) so the ritual can be completed.
4. Set settings so Shutdown Time is a few minutes away:
   `cloister-device settings --workdays 1,2,3,4,5,6,7 --workday-start 00:00 --shutdown-time HH:MM`.
   (Changing them later only tightens today; pick a time you can live with.)
5. Connect a second display if you have one.

## Warnings and the Shutdown Delay

- [ ] At T−15 and T−5 minutes a notification says "Shutdown Time in 15 minutes, at HH:MM." (and 5).
- [ ] The menu-bar menu shows "1 Shutdown Delay left this week." Use it: the notice says
      "Shutdown delayed to HH:MM. Logged." and the warnings move 30 minutes later.
- [ ] The menu now says "No Shutdown Delay left this week." and the item is disabled. Trying it
      from another Device (`cloister-device`) is refused the same way.
- [ ] Work tools stay reachable during the 30 minutes; at the delayed time Evening Lockdown starts.

## The kiosk at Shutdown Time

- [ ] Within about 5 s of Shutdown Time (the daemon's tick) the ritual covers **every display**:
      the steps on the main display, "The ritual is on the main display." on the others.
- [ ] The Dock is hidden and doesn't appear when the pointer touches the screen edge.
- [ ] The menu bar is hidden and doesn't appear at the top edge.
- [ ] Command-Tab does nothing (no app switcher).
- [ ] Command-Option-Escape (Force Quit Applications) does not open.
- [ ] Command-H (hide), Command-M (minimize), Command-W (close) and Command-Q do nothing.
- [ ] Mission Control (F3, Control-Up, three-finger swipe up) and Spaces switching
      (Control-Left/Right, four-finger swipe) don't reveal another app or desktop.
      Note what happens: the window is on every Space, so it should stay in front.
- [ ] Clicking a hot corner, a notification banner or a Spotlight shortcut (Command-Space)
      doesn't bring another app forward for more than a second (the window re-asserts itself
      once a second). Record anything that stays in front.
- [ ] Unplugging and re-plugging the second display: within a few seconds the window is back
      in front on the main display.

## It comes back

- [ ] From ssh: `pkill -9 -x cloister-app`. The ritual is back within about 1 s
      (launchd `KeepAlive.PathState` with `ThrottleInterval` 1).
- [ ] `launchctl bootout gui/$(id -u)/com.cloister.ritual` (as the user): the daemon loads the
      agent again on its next tick and the window returns within about 5 s.
- [ ] Log out and back in (or restart): the ritual is the first thing shown after login.
- [ ] `sudo launchctl kill SIGKILL system/com.cloister.enforcer`: the daemon is relaunched and
      the ritual stays.

## Completing it

- [ ] Step 01 refuses to continue without a Loose End captured tonight ("Capture at least one
      Loose End.").
- [ ] Step 02 shows today's Deep Sessions with Intents and Depth Ratings, and Deep Hours this week.
- [ ] Step 03 shows tomorrow's Schedule-filled draft.
- [ ] Step 04 refuses without a saved plan, a deep Slot, or with a gap in the Workday.
- [ ] Step 05: the hold is disabled until "Shutdown complete" is typed exactly; the hold takes
      two seconds (pointer, or Space/Return held).
- [ ] After completing, the kiosk lifts within about 1 s on this Mac, and the ritual drops on
      every other Device within seconds (Realtime).
- [ ] Completing on another Device instead lifts the kiosk here within a few seconds.

## Missed Ritual

- [ ] Leave the ritual undone past the next Workday Start: the kiosk lifts by itself at Workday
      Start, and the day runs on its Schedule (or the saved plan).

## Known limits (record whether they hold)

- On a notched MacBook a black strip may remain above the window where the menu bar was. Nothing
  there is usable while the menu bar is hidden.
- Notification Center banners can still appear above the window (they can't take focus).
- Without sign-in, the ritual can be shown but not completed; it lapses at Workday Start.
- Turning the Mac off or logging out is still possible (no `DisableSessionTermination`); the
  ritual is shown first at the next login.

Result: ______ (date, macOS version, build, who ran it, anything that escaped)
