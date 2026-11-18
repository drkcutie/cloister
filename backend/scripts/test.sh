#!/bin/sh
# Run the backend tests against a local Supabase:
#   - the shared rules fixtures through the Postgres triggers (Seam A),
#   - sign-in (magic link via Mailpit) and sync between two simulated Devices,
#   - the Android sync layer (android/sync, Kotlin) between two simulated phones (SKIP_ANDROID=1 skips it).
# Starts the local stack if needed and resets the database to the migrations.
# Pass --stop to stop the stack afterwards.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root/backend"
if ! supabase status >/dev/null 2>&1; then
  supabase start -x studio,storage-api,imgproxy,edge-runtime,logflare,vector,supavisor,postgres-meta </dev/null
fi
supabase db reset </dev/null
eval "$(supabase status -o env 2>/dev/null | grep -E '^(API_URL|ANON_KEY|DB_URL|MAILPIT_URL|INBUCKET_URL)=')"
export CLOISTER_DB_URL="$DB_URL" CLOISTER_API_URL="$API_URL" CLOISTER_ANON_KEY="$ANON_KEY"
export CLOISTER_MAILPIT_URL="${MAILPIT_URL:-$INBUCKET_URL}"
cd "$root"
command -v cargo >/dev/null || . "$HOME/.cargo/env"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/cloister-target}"
status=0
cargo test -p cloister-sync -- --ignored --test-threads=1 || status=$?
# The Android sync layer (supabase-kt) against the same stack. Needs JDK 17.
if [ -z "${SKIP_ANDROID:-}" ]; then
  export JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@17}"
  (cd "$root/android" && ./gradlew :sync:test --rerun-tasks -q) || status=$?
fi
if [ "${1:-}" = "--stop" ]; then (cd "$root/backend" && supabase stop); fi
exit $status
