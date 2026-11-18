# Shared rules scenarios (Seam A)

Every file in `fixtures/rules/*.json` is a scenario for the **rules engine**. The same files are run
against every implementation of the rules:

| Implementation | Runner |
| --- | --- |
| Rust (`desktop/crates/cloister-core`) | `desktop/crates/cloister-core/tests/fixtures.rs` (`cargo test -p cloister-core`) |
| Kotlin (Android) | to be added; must read these files unchanged |
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

`blocklists` is sorted. Blocks are kept after they end (they are history, and later rules need them).

**Commands** (`type` selects the command; unknown fields are errors)

| `type` | Fields | Notes |
| --- | --- | --- |
| `start_block` | `block_id`, `minutes`, `deep` (default `true`), `blocklists` (extra lists, default `[]`), `confirm_long` (default `false`), `client_time` (optional, ignored), `intent` (optional) | Starts at `server_time`. The Distraction List is always included. The Intent is trimmed; a blank one is dropped. |
| `extend_block` | `block_id`, `ends_at` | Moves the end of an active Block later. The 24-hour cap still applies to the whole Block. |
| `cancel_block` | `block_id` | Always rejected: a Block can't be cancelled. |
| `record_depth_rating` | `block_id`, `rating` | Records a Deep Session's Depth Rating (1 to 5), once, at or after its end. |

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

## Adding a scenario

1. Name the rule in one sentence; that is the `description`.
2. Write the file in `fixtures/rules/<name>.json` with expected values worked out by hand from the
   spec, never by running the engine.
3. Run every runner. A new command or reason must be added to the tables above and to every
   engine in the same change.
