# Cloister

A cross-device blocker built on Cal Newport's *Deep Work*. It makes focused work unavoidable during the day, and once the day ends it makes not working unavoidable too.

> Status: early. The rules engine and macOS enforcement service exist; there is no GUI yet.

## What it does

- **Deep Sessions.** Before you start, you write an Intent: what this stretch of work will produce. Distracting sites and apps are then blocked on every Device until the timer ends. There's no stop button, and quitting the app, deleting it or rebooting doesn't end the block. Strictness is on a par with [SelfControl](https://selfcontrolapp.com/). When it ends, you give a Depth Rating, and the time adds to your weekly **Deep Hours**.
- **Blocked by default.** The built-in Distraction List covers social media, news, video, shopping and forums. Blocking applies to whole domains, including subdomains, and to desktop and Android apps.
- **Day Plan.** Each evening you divide tomorrow into Slots. Deep Slots automatically become Deep Sessions. You can move them during the day, but the day's planned deep minutes can only go up.
- **Shutdown Ritual.** At Shutdown Time an unskippable full-screen ritual takes over every Device: capture Loose Ends, review today, plan tomorrow, then type *"Shutdown complete"*. You get one 30-minute delay per week.
- **Evening Lockdown and Rest Days.** After Shutdown Time, and on non-workdays, your work tools (chat, email, code hosting) are blocked as well. Rest is enforced too.
- **Quit-Social Block.** A 30-day experiment in going without social media.
- **Weekly Review.** Part of the last ritual of each week: Deep Hours against target, depth trend, missed rituals, delays used, and breaches.
- **Tighten now, loosen tomorrow.** Any change that makes a rule stricter applies immediately. Any change that loosens one waits until the next day.

## Platforms

| Platform | Stack | Enforcement |
| --- | --- | --- |
| macOS, Windows, Linux | Rust + [GPUI](https://www.gpui.rs/) | Root service: local DNS filter, firewall, app blocker, kiosk-mode ritual window |
| Android | Kotlin + Jetpack Compose | VPN DNS filter, accessibility-based app blocker, lock overlay, optional Device Owner *Hardcore Mode* |
| Sync | Supabase | Postgres enforces the rules on the server; Devices keep enforcing offline |

## Repository layout

| Path | Contents |
| --- | --- |
| `desktop/` | Rust workspace crates: rules engine, enforcement service, developer CLI. See [`desktop/README.md`](desktop/README.md). |
| `android/` | Kotlin app (placeholder). |
| `backend/` | Supabase project (placeholder). |
| `fixtures/rules/` | Shared JSON rule scenarios run by every rules engine. See [`fixtures/README.md`](fixtures/README.md). |
| `shared/` | Data shared by every platform, such as the built-in Blocklists. |
| `scripts/` | Install scripts and root-only smoke tests. |

## Docs

- [`GLOSSARY.md`](GLOSSARY.md): the project's vocabulary. Please use these terms.
- [`docs/adr/`](docs/adr/): architecture decisions and the reasoning behind them.

## License

[MIT](LICENSE)
