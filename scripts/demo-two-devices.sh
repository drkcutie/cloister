#!/bin/sh
# Demo: a Block started on Desktop A is enforced on Desktop B without user action.
#
# Runs against the local Supabase (started if needed). Both Devices are `cloister-device`
# instances with their own mirrors on this Mac; B hands its mirror to its own `cloisterd`
# (dry run, DNS filter on 127.0.0.1:5399, no root needed). Magic links are clicked through
# Mailpit so the demo runs unattended.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
. "$HOME/.cargo/env"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/cloister-target}"
cd "$root/backend"
supabase status >/dev/null 2>&1 || supabase start -x studio,storage-api,imgproxy,edge-runtime,logflare,vector,supavisor,postgres-meta </dev/null
eval "$(supabase status -o env 2>/dev/null | grep -E '^(API_URL|ANON_KEY|MAILPIT_URL|INBUCKET_URL)=')"
export CLOISTER_API_URL="$API_URL" CLOISTER_ANON_KEY="$ANON_KEY"
mailpit="${MAILPIT_URL:-$INBUCKET_URL}"
cd "$root"
cargo build -q -p cloister-sync -p cloister-enforcer --bins
bin="$CARGO_TARGET_DIR/debug"
tmp=$(mktemp -d)
sock="/tmp/cl-demo-b-$$.sock"
email="demo-$(date +%s)@devices.test"
pids=""
cleanup() { for p in $pids; do kill "$p" 2>/dev/null || true; done; rm -rf "$tmp" "$sock"; }
trap cleanup EXIT

click_link() { # wait for the newest email to $1 and open its link like a browser would
  for _ in $(seq 50); do
    link=$(curl -s "$mailpit/api/v1/search?query=to:%22$1%22" | python3 -c '
import json, re, sys, urllib.request
msgs = json.load(sys.stdin).get("messages") or []
if msgs:
    m = json.load(urllib.request.urlopen(sys.argv[1] + "/api/v1/message/" + msgs[0]["ID"]))
    print(re.search(r"https?://\S+?(?=[\s)\]])", m["Text"]).group(0))' "$mailpit")
    if [ -n "$link" ]; then curl -s -o /dev/null -L "$link"; return; fi
    sleep 0.2
  done
  echo "no email for $1" >&2; exit 1
}

sign_in() { # $1 = mirror, $2 = name
  "$bin/cloister-device" sign-in --email "$email" --mirror "$1" --name "$2" &
  p=$!
  click_link "$email"
  wait $p
  # Each sign-in link is used once; the next one gets a fresh email.
  curl -s -X DELETE "$mailpit/api/v1/messages" >/dev/null
  sleep 1.1
}

echo "== Signing in Desktop A and Desktop B as $email"
sign_in "$tmp/a.sqlite" "Desktop A"
sign_in "$tmp/b.sqlite" "Desktop B"

echo "== Desktop B: enforcement service (dry run) and sync"
"$bin/cloisterd" run --dry-run --state "$tmp/b-state.json" --socket "$sock" --dns 127.0.0.1:5399 --tick 1 2>"$tmp/cloisterd.log" &
pids="$pids $!"
until [ -S "$sock" ]; do sleep 0.1; done
"$bin/cloister-device" run --mirror "$tmp/b.sqlite" --name "Desktop B" --enforcer "$sock" >"$tmp/b.log" 2>&1 &
pids="$pids $!"
sleep 3
echo "B resolves youtube.com before: $(dig +short @127.0.0.1 -p 5399 youtube.com | head -1)"

echo "== Desktop A: start a 25-minute Deep Session"
"$bin/cloister-device" start-block --minutes 25 --mirror "$tmp/a.sqlite" --name "Desktop A"
start=$(date +%s)
until grep -q "^Blocking" "$tmp/b.log"; do
  [ $(( $(date +%s) - start )) -lt 15 ] || { echo "B never picked it up"; cat "$tmp/b.log"; exit 1; }
  sleep 0.2
done
echo "== Desktop B, $(( $(date +%s) - start ))s later:"
cut -c1-120 "$tmp/b.log"
sleep 1.5
echo "B resolves youtube.com after: $(dig @127.0.0.1 -p 5399 youtube.com | grep -o 'status: [A-Z]*')"
echo "== Devices"
"$bin/cloister-device" devices --mirror "$tmp/a.sqlite" --name "Desktop A"
