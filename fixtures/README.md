# Shared rules scenarios (Seam A)

Every file in `fixtures/rules/*.json` is a scenario for the **rules engine**. The same files are run
against every implementation of the rules:

| Implementation | Runner |
| --- | --- |
| Rust (`desktop/crates/cloister-core`) | `desktop/crates/cloister-core/tests/fixtures.rs` (`cargo test -p cloister-core`) |
| Kotlin (Android) | `android/core/src/test/kotlin/dev/cloister/core/SharedFixturesTest.kt` (`cd android && ./gradlew :core:test`) |
| Postgres triggers (Supabase) | `desktop/crates/cloister-sync/tests/postgres_fixtures.rs` (`backend/scripts/test.sh`, needs Docker) |

A rule is done when every runner agrees on every scenario. Don't copy fixtures into other
directories: point each runner at this directory.

The built-in Blocklists and the related-domain map that the scenarios rely on live in
[`shared/blocklists.json`](../shared/blocklists.json); every engine loads the same file.

## Scenario format

```jsonc
{
  "name": "block-cannot-be-shortened",      // unique, matches the file name
  "description": "One sentence: the rule this scenario pins down.",
  "initial_state": { "blocks": [] },         // a State, in the engine's JSON form (below)
  "steps": [
    {
      "server_time": "2026-01-05T09:00:00Z", // RFC 3339, UTC. The only clock the engine sees.
      "command": { "type": "start_block", "block_id": "b1", "minutes": 60 }, // optional
      "expect": {
        "outcome": "accepted",               // or "rejected"; required when there is a command
        "reason": null,                      // the rejection code when "rejected"
        "state": { "blocks": [ ... ] },      // optional: the whole state after this step
        "enforcement": { ... }               // optional: see below
      }
    },
    {
      "server_time": "2026-01-05T10:00:00Z", // a step with no command just asks
      "expect": { "enforcement": { "blocked_domains": [] } } // "what is enforced now?"
    }
  ]
}
```

Runner semantics, which every runner must follow:

1. Start from `initial_state`.
2. For each step in order: if it has a `command`, call `decide(state, command, server_time)`,
   check `outcome`/`reason`, and carry the returned state forward. Without a command, call
   `enforcement(state, server_time)`.
3. A rejected command must leave the state unchanged (runners assert this on every rejection).
4. `expect.state` is compared as JSON, exactly (key order doesn't matter).
5. Unknown keys anywhere in a scenario are an error, so typos fail loudly.

### Enforcement expectations

All keys are optional; only the ones present are checked.

| Key | Meaning |
| --- | --- |
| `blocked_domains` | Exact set of blocked domains (order ignored). |
| `blocked_domains_include` | These domains must be in the set. |
| `blocked_domains_exclude` | These domains must not be in the set. |
| `blocked_apps` | Exact set of blocked apps (order ignored). |
| `ritual` | Ritual screen state: `none`, `due`, `delayed`, `completed` or `missed`. |

A blocked domain always covers its subdomains, so `youtube.com` in the set blocks
`m.youtube.com`. Related domains (`youtu.be`, `googlevideo.com`...) are listed explicitly.

## JSON forms

**State**

```json
{
  "blocks": [
    {
      "id": "b1",
      "starts_at": "2026-01-05T09:00:00Z",
      "ends_at": "2026-01-05T10:00:00Z",
      "blocklists": ["forums", "news", "shopping", "social", "video"],
      "deep": true,
      "intent": "Draft chapter 3",
      "depth_rating": 4
    }
  ]
}
```

`intent` and `depth_rating` are omitted when absent (a plain Block, or a Deep Session not yet rated).

`day_plans` and `schedules` are omitted when empty, so a state with only Blocks reads as above:

```json
{
  "blocks": [],
  "day_plans": [
    {
      "date": "2026-01-05",
      "starts_at": "2026-01-05T08:00:00Z",
      "ends_at": "2026-01-05T17:00:00Z",
      "planned_deep_minutes": 150,
      "slots": [
        { "id": "s1", "starts_at": "2026-01-05T08:00:00Z", "ends_at": "2026-01-05T10:30:00Z",
          "label": "Draft chapter 3", "deep": true, "depth_rating": 4 },
        { "id": "s2", "starts_at": "2026-01-05T11:00:00Z", "ends_at": "2026-01-05T12:00:00Z",
          "label": "Email and chat", "deep": false }
      ]
    }
  ],
  "schedules": [
    {
      "id": "weekdays", "time_zone": "UTC", "weekdays": [1, 2, 3, 4, 5],
      "starts": "08:00", "ends": "17:00",
      "slots": [ { "starts": "08:00", "ends": "11:00", "label": "Deep work", "deep": true } ]
    }
  ]
}
```

- Day Plans are sorted by `date`, their Slots by start; Schedules by `id`, their Slots by start
  and `weekdays` ascending (ISO: 1 = Monday ... 7 = Sunday). Labels are trimmed.
- A Day Plan's `starts_at`/`ends_at` are the Workday's bounds. Until Workday Start and Shutdown
  Time exist as settings they are given with the plan.
- `planned_deep_minutes` is the total length of the deep Slots, in minutes.
- A deep Slot **is** a Deep Session from its start to its end: it blocks the Distraction List
  (like a deep Block with no extra Blocklists) and takes a Depth Rating by its Slot id. It is
  not copied into `blocks`.
- A date with no saved Day Plan runs on the Schedule covering its weekday. Times in a Schedule
  are local to its `time_zone` (IANA); the Schedule's plan for a date has Slot ids
  `<schedule id>-<date>-<n>` (n from 1, in order of start). Enforcement uses, for each
  Schedule, the current date in its time zone.

`blocklists` is sorted. Blocks are kept after they end (they are history, and later rules need them).

A Block may also have `extra_domains` and `extra_apps` (sorted; omitted when empty) and `quit_social: true`
(omitted when false). The state may also have (each omitted when empty / UTC):

```json
{
  "time_zone": "America/New_York",
  "blocklist_entries": [{ "blocklist": "work", "kind": "domain", "entry": "figma.com" }],
  "blocklist_removals": [{ "blocklist": "forums", "kind": "domain", "entry": "reddit.com", "effective_at": "2026-01-06T05:00:00Z" }],
  "blocklist_opt_outs": [{ "blocklist": "news", "effective_at": "2026-01-06T05:00:00Z" }]
}
```

Breaches are listed as `"breaches": [{ "id": "phone-1", "kind": "vpn_turned_off", "occurred_at": "..." }]`
(omitted when empty), sorted by `occurred_at`, then `id`.

Entries and removals are sorted by `(blocklist, kind, entry)` in byte order (`app` before `domain`), opt-outs by
`blocklist`. Removals and opt-outs stay after they take effect; adding the entry again (or opting in) deletes them.

**Commands** (`type` selects the command; unknown fields are errors)

| `type` | Fields | Notes |
| --- | --- | --- |
| `start_block` | `block_id`, `minutes`, `deep` (default `true`), `blocklists` (extra lists, default `[]`), `confirm_long` (default `false`), `client_time` (optional, ignored), `intent` (optional) | Starts at `server_time`. The Distraction List is always included. The Intent is trimmed; a blank one is dropped. |
| `extend_block` | `block_id`, `ends_at` | Moves the end of an active Block later. The 24-hour cap still applies to the whole Block. |
| `cancel_block` | `block_id` | Always rejected: a Block can't be cancelled. |
| `record_depth_rating` | `block_id`, `rating` | Records a Deep Session's Depth Rating (1 to 5), once, at or after its end. `block_id` may be a deep Slot's id. |
| `write_day_plan` | `date`, `starts_at`, `ends_at`, `slots` (each `id`, `starts_at`, `ends_at`, `label`, `deep` (default `false`)) | Saves the Day Plan for `date`, replacing a saved one. Depth Ratings of unchanged Slots are kept. |
| `put_schedule` | `schedule_id`, `time_zone`, `weekdays`, `starts`, `ends`, `slots` (each `starts`, `ends`, `label`, `deep` (default `false`)) | Creates or replaces a Schedule. If a day is running on the old Schedule (its start passed, its end not, no saved plan), that day's plan is first saved as its Day Plan, so the change applies from the next day. |
| `add_blocklist_entry` | `blocklist`, `kind` (`domain` or `app`), `entry` | Adds to a Blocklist. Applies immediately, also to running Blocks. Cancels a removal of the same entry. Adding an entry already listed changes nothing. |
| `remove_blocklist_entry` | `blocklist`, `kind`, `entry` | Stores a removal that takes effect at the next day boundary. Removing an entry whose removal is pending changes nothing (the date stays). |
| `opt_out_of_blocklist` | `blocklist` | Distraction List categories only. From the next day boundary, new Blocks leave the category out. A running Block keeps it. |
| `opt_in_to_blocklist` | `blocklist` | Drops an opt-out, pending or in force. Applies immediately, also to running Blocks. |
| `start_quit_social_block` | `block_id`, `confirm_long` (default `false`, must be `true`) | A 30-day plain Block of the Social list only (`quit_social: true`): the one exception to the 24-hour cap. Can't be extended (any end past 24 hours is `block_too_long`). |
| `record_breach` | `breach_id`, `kind` (`vpn_turned_off` or `app_blocker_turned_off`), `client_time` (optional, ignored) | Records a Breach at server time. The same `breach_id` again changes nothing, so a Device can resend it. Enforcement is unchanged: recording is the only consequence. |

`start_block` also takes `extra_domains` and `extra_apps` (default `[]`): sites and apps blocked for that Block only.
Domains are trimmed, lower-cased and lose a trailing dot; they need two or more labels of `a-z`, `0-9` and inner `-`
(at most 63 characters each, 253 in all). Apps are trimmed and 1 to 200 characters. Extras are deduplicated and sorted.

**Day boundary.** A loosening change made at server time `t` takes effect at the first local midnight after `t` in
the state's `time_zone` (IANA, default `UTC`). A midnight skipped by a clock change maps to the first instant after it.

**What a Block enforces.** For each active Block: its `blocklists`, plus (unless it is the Quit-Social Block) every
Distraction List category not opted out at that moment, plus its extras. For each of those Blocklists: the built-in
entries and `blocklist_entries`, minus removals whose `effective_at` has passed. Then related domains are added. A running deep Slot counts as a
deep Block started at the Slot's start: the categories not opted out then (and, like any Block, those not opted out now).

**The ratchet** (`write_day_plan`). The plan in force for the date is the saved one, or the
Schedule's once its `starts_at` has passed (before that the Schedule is only a draft). Against
it, checked in this order after the shape checks: every deep Slot that has started must appear
unchanged (`deep_session_has_started`); a deep Slot starting before server time must be an
unchanged deep Slot (`slot_in_the_past`); Planned Deep Minutes must not go down
(`slot_cannot_be_removed`).

**Rejection reasons** (stable codes; UI copy is mapped from these, never shown raw)

| Code | When |
| --- | --- |
| `invalid_length` | `minutes` is zero or negative. |
| `block_too_long` | The Block would be longer than 24 hours. |
| `confirmation_required` | The Block is longer than 4 hours and `confirm_long` is not `true`. |
| `duplicate_block` | A Block with this `block_id` already exists. |
| `unknown_block` | No Block has this `block_id`. |
| `unknown_blocklist` | An extra Blocklist id isn't in the catalog. |
| `block_cannot_be_shortened` | `extend_block` would move the end earlier. The same end is accepted and changes nothing. |
| `block_has_ended` | `extend_block` on a Block that has already ended. |
| `block_cannot_be_cancelled` | Any `cancel_block`. |
| `not_a_deep_session` | `record_depth_rating` on a plain Block. |
| `deep_session_not_ended` | `record_depth_rating` before the Deep Session's end. |
| `invalid_depth_rating` | `rating` is outside 1 to 5. |
| `depth_rating_already_recorded` | The Deep Session already has a Depth Rating. |
| `invalid_entry` | A Blocklist entry or a Block extra isn't a valid domain or app. Checked after `unknown_blocklist`, before `duplicate_block` / `unknown_entry`. |
| `unknown_entry` | `remove_blocklist_entry` of an entry the Blocklist doesn't hold now (never listed, or already removed). |
| `cannot_opt_out` | `opt_out_of_blocklist` of a Blocklist that isn't a Distraction List category (the Work List). |
| `invalid_day_plan` | The Day Plan ends at or before its start, or is longer than 24 hours. |
| `invalid_slot` | A Slot's label is blank, it ends at or before its start, or it lies outside its day (for a Schedule: a time that isn't `HH:MM`). |
| `slots_overlap` | Two Slots overlap (touching is fine). |
| `duplicate_slot` | A Slot id repeats, or is used by a Slot of another day or by a Block. (`start_block` with a Slot's id is `duplicate_block`.) |
| `deep_session_has_started` | A deep Slot that has started would change or go. |
| `slot_in_the_past` | A new or changed deep Slot would start before server time. |
| `slot_cannot_be_removed` | Planned Deep Minutes would go down. |
| `invalid_schedule` | Weekdays outside 1 to 7 or repeated, `starts`/`ends` not `HH:MM` (00:00 to 23:59), or the day ends at or before its start. |
| `unknown_time_zone` | `time_zone` isn't an IANA time zone. |
| `schedules_overlap` | Another Schedule already covers one of the weekdays. |

Checks run in this order: day bounds, Slots, overlap, ids, then the ratchet; for Schedules:
time zone, weekdays and times, Slots, overlap, other Schedules.

## Adding a scenario

1. Name the rule in one sentence; that is the `description`.
2. Write the file in `fixtures/rules/<name>.json` with expected values worked out by hand from the
   spec, never by running the engine.
3. Run every runner. A new command or reason must be added to the tables above and to every
   engine in the same change.
