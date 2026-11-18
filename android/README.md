# Android

Kotlin + Jetpack Compose (ADR 0003). Three Gradle modules:

| Module | What | Test |
| --- | --- | --- |
| `core` (JVM) | Rules engine (same interface as `cloister-core`: `RulesEngine.decide` / `enforcement`, JSON forms, reason codes), the tighten-only `MirrorMerge`, and the VPN's `DnsFilter`. Loads [`shared/blocklists.json`](../shared/blocklists.json) from the classpath. | `./gradlew :core:test` runs every shared scenario in [`fixtures/rules`](../fixtures/README.md) in place (Seam A), plus DNS packet and merge tests. CI: `.github/workflows/android.yml`. |
| `sync` (JVM) | supabase-kt `Device`: sign-in (magic link + code, Google ID token), Device registration, commands via `rpc/command`, `rpc/state` pulled into a JSON mirror (enforced offline), Realtime, Breaches queued offline. | `backend/scripts/test.sh` runs `:sync:test` (two phones on one account) against the local Supabase. |
| `app` | `FilterVpn` (DNS-only VPN), `AppWatcher` (Accessibility: blocked-app overlay, lock overlay, Shutdown Ritual overlay, uninstall guard), `AdminReceiver` (Device Admin / Device Owner), `Enforcer`, Compose screens. | `scripts/smoke-test.sh` (Seam B, emulator), `scripts/ritual-test.sh` (Shutdown Ritual, emulator). |

## Build

```sh
export JAVA_HOME=/opt/homebrew/opt/openjdk@17 ANDROID_HOME=$HOME/Library/Android/sdk
./gradlew :core:test                      # fixtures, seconds, no Android SDK needed
./gradlew :app:assembleDebug -Pcloister.anonKey=<anon key> [-Pcloister.apiUrl=http://10.0.2.2:56421] \
    [-Pcloister.googleServerClientId=<web client id>]
```

compileSdk is 37 (supabase-kt's transitive AndroidX/OkHttp need it; AGP downloads the platform), targetSdk 35,
minSdk 26.

## Enforcement on the phone

- **VPN:** only the fake resolver `10.111.222.2` (and `fd00:111:222::2`) is routed into the tun, so only DNS
  arrives; blocked names (and subdomains) get NXDOMAIN, the rest go to the network's resolver over a protected
  socket. Blocks, planned Deep Sessions, Evening Lockdown and Rest Days all come from `RulesEngine.enforcement`.
- **App blocker:** package names in `blocked_apps` get a calm overlay (Intent, time left, Home).
- **Breach:** the VPN revoked, or the Accessibility service turned off, during a Block records `record_breach`
  (synced, idempotent) and the lock overlay shows until the VPN is back. Nothing else happens.
- **Uninstalling:** Device Admin keeps the app installed; during a Block the app blocker leaves the deactivation
  screen and Cloister's uninstall dialog. Hardcore Mode (Device Owner via
  `adb shell dpm set-device-owner dev.cloister/.app.enforce.AdminReceiver`, no accounts on the phone) adds
  always-on VPN with lockdown, no VPN/Private DNS changes, and `setUninstallBlocked` during Blocks.
- **Shutdown Ritual:** while `enforcement.ritual` is `Due`, everything but Cloister, the dialers and the emergency
  dialer is covered by the ritual overlay ("Open the Shutdown Ritual", "Emergency call"). Cloister opens on a
  compact flow: Loose Ends, review today (Deep Hours against the target), tomorrow's Schedule-filled draft, a
  contiguous Day Plan editor (always covers the Workday), the Weekly Review on the last Workday, and
  "Shutdown complete" with a hold. Steps are gated by `RulesEngine.decide(CompleteRitual)` locally; completing
  sends `complete_ritual`. "Delay 30 min (N left this week)" is on the ritual and on Today.
- **Week tab:** the Deep Hours well, deep minutes by day, the depth trend and the week's records, from
  `Review.weekSummary`. No percentages, no streaks.
- Sideloaded APKs on Android 13+ need "Allow restricted settings" in App info before Accessibility can be enabled.

Debug builds are `testOnly` (Device Owner can be removed with `dpm remove-active-admin`) and carry a
`DebugReceiver` for the emulator scripts (password sign-in against the local stack).

`scripts/ritual-test.sh` (throwaway AVD on its own port, deleted afterwards; needs the local stack and the shared
lock) makes the ritual due by backdating `settings_since` as the database owner, then checks the overlay, the
emergency dialer, the flow's steps and that completing lifts the overlay. Set `SHOTS=dir` for screenshots.

## Onboarding

First run (`ui/Onboarding.kt`, model `core/.../Onboarding.kt` with `OnboardingTest`): the philosophy, sign-in,
the VPN, the app blocker and Device Admin one at a time (each skipped when already on), Workdays / Workday Start /
Shutdown Time (`change_settings`, same checks as the desktop), then the optional Hardcore Mode guide: remove
accounts, run `adb shell dpm set-device-owner dev.cloister/.app.enforce.AdminReceiver`, verify (the app asks
Android whether it is Device Owner), add the accounts back. The phone reports `hardcore` with its Device row, and
desktop Devices screens show it as "Enforcing with Hardcore Mode".
