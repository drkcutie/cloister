# Backend

The Supabase project: Postgres schema, row-level security and the rules as triggers (ADR 0004),
auth (email magic link, Google) and Realtime. Devices talk to it through
[`cloister-sync`](../desktop/crates/cloister-sync).

```
backend/
  supabase/config.toml          local stack (ports 5642x so it can run beside other projects)
  supabase/migrations/          schema, Block rules, built-in Blocklists
  scripts/test.sh               start the stack, reset the database, run every backend test
  scripts/catalog-migration.sh  regenerate the Blocklist catalog migration from shared/blocklists.json
```

## Running it locally

Needs Docker and the Supabase CLI.

```sh
backend/scripts/test.sh          # start + reset + run the Postgres fixtures, sign-in and sync tests
backend/scripts/test.sh --stop   # same, then stop the containers
scripts/demo-two-devices.sh      # a Block started on Desktop A enforced on Desktop B
(cd backend && supabase stop)    # free the containers
```

`supabase status -o env` prints the API URL, anon key and database URL. Sign-in emails land in
Mailpit at http://127.0.0.1:56424.

## What the database enforces

- `public.blocks` is read-only to clients. Every change goes through `public.command(cmd jsonb)`,
  which takes a command in the shared fixture form (`fixtures/README.md`) and returns
  `{"outcome": "accepted" | "rejected", "reason": <code>}`.
- Triggers on `blocks` guard every write, including privileged ones: a Block starts at server
  time, is at most 24 hours long, needs confirmation over 4 hours, always covers the Distraction
  List, never moves its end earlier, never changes its Blocklists, and can't be deleted (only
  deleting the account removes its Blocks). Violations raise SQLSTATE `CL001` with the reason
  code as the message.
- Server time is `now()`. Only the database owner may pin it (`cloister.server_time`), which the
  fixture runner uses. A client's own clock never counts.
- Row-level security limits every row in `accounts`, `devices` and `blocks` to its owner.
- `public.state()` and `public.enforcement()` give the signed-in user's state and Enforcement in
  the rules engine's JSON form. The fixture runner checks both against `fixtures/rules`.

## Sign-in setup

### Email magic link

This works locally with no setup: emails go to Mailpit. Desktop sign-in uses PKCE with a
loopback redirect (`http://127.0.0.1:<port>/auth/callback`), so the link must be opened on the
same computer. The email also carries a 6-digit code for signing in on another computer
(`cloister-device sign-in --email ... --code 123456`).

For a hosted project, set up SMTP under Authentication → Emails, and add
`http://127.0.0.1:*/**` and `cloister://auth` to Authentication → URL Configuration → Redirect URLs.

### Google

Google sign-in needs OAuth credentials that only the project owner can create:

1. In Google Cloud Console → APIs & Services → Credentials, create an **OAuth client ID** of type
   **Web application**.
2. Under Authorised redirect URIs, add the Supabase callback:
   - hosted: `https://<project-ref>.supabase.co/auth/v1/callback`
   - local: `http://127.0.0.1:56421/auth/v1/callback`
3. Configure the OAuth consent screen (External; add yourself as a test user while in testing).
4. Hosted: Authentication → Sign In / Providers → Google, paste the client ID and secret.
   Local: set `enabled = true` under `[auth.external.google]` in `supabase/config.toml`, then
   ```sh
   export SUPABASE_AUTH_EXTERNAL_GOOGLE_CLIENT_ID=...apps.googleusercontent.com
   export SUPABASE_AUTH_EXTERNAL_GOOGLE_SECRET=...
   supabase stop && supabase start
   ```
5. `cloister-device sign-in --google` opens the browser, and the loopback listener completes
   sign-in.
   Android will use Credential Manager and an ID token (`grant_type=id_token`). That needs an
   additional **Android** OAuth client (package name + signing SHA-1), with the Web client ID as
   the server client ID.

## Changing the rules

See `fixtures/README.md`. A rule is done when the Rust engine, the Kotlin engine and these
triggers all pass the same fixtures. Add a new migration for each change; never edit an applied one.
