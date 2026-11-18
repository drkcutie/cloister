#!/bin/bash
# Seam B smoke test for macOS: given an Enforcement, blocked domains and their subdomains
# don't resolve, DNS can't bypass the filter, and this still holds after the service restarts.
#
# Usage: sudo scripts/macos/smoke-test.sh [MINUTES]   (default 3)
#
# It starts a real Block of MINUTES with the Distraction List. That Block CAN'T be cancelled:
# YouTube, Reddit, etc. stay blocked until it ends. Install first with scripts/macos/install.sh.
set -uo pipefail

MINUTES=${1:-3}
LABEL=com.cloister.enforcer
CLI=/usr/local/bin/cloister
BLOCKED=(youtube.com www.youtube.com m.youtube.com youtu.be rr1---sn-abc.googlevideo.com old.reddit.com x.com)
ALLOWED=(example.com apple.com)
failures=0

[[ $EUID -eq 0 ]] || { echo "Run with sudo." >&2; exit 1; }
[[ -x $CLI ]] || { echo "Install first: sudo scripts/macos/install.sh" >&2; exit 1; }

pass() { echo "  ok    $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }

resolves() { # name -> 0 if the system resolver returns an address
  [[ -n $(dscacheutil -q host -a name "$1" 2>/dev/null | grep -E '^ip(v6)?_address') ]]
}

check() {
  echo "== $1"
  for name in "${BLOCKED[@]}"; do
    if dig +time=2 +tries=1 @127.0.0.1 "$name" | grep -q 'status: NXDOMAIN'; then
      pass "filter: $name is NXDOMAIN"
    else
      fail "filter: $name is not NXDOMAIN"
    fi
    if resolves "$name"; then fail "system resolver: $name resolves"; else pass "system resolver: $name doesn't resolve"; fi
  done
  for name in "${ALLOWED[@]}"; do
    if resolves "$name"; then pass "system resolver: $name resolves"; else fail "system resolver: $name doesn't resolve"; fi
  done
  # A non-root process can't reach another resolver directly (pf).
  if sudo -u nobody dig +time=2 +tries=1 @1.1.1.1 youtube.com >/dev/null 2>&1; then
    fail "pf: DNS to 1.1.1.1 got through"
  else
    pass "pf: DNS to 1.1.1.1 is dropped"
  fi
  if grep -q '0.0.0.0	youtube.com' /etc/hosts; then pass "hosts: fallback entries present"; else fail "hosts: no fallback entries"; fi
  while IFS= read -r svc; do
    svc=${svc#\*}
    dns=$(networksetup -getdnsservers "$svc")
    if [[ $dns == 127.0.0.1 ]]; then pass "dns settings: $svc -> 127.0.0.1"; else fail "dns settings: $svc -> $dns"; fi
  done < <(networksetup -listallnetworkservices | tail -n +2)
}

echo "This starts a $MINUTES-minute Block that can't be cancelled. Continue? [y/N]"
read -r answer
[[ $answer == y ]] || exit 1

"$CLI" block start --minutes "$MINUTES" --plain || { echo "Couldn't start a Block." >&2; exit 1; }
sleep 2
check "while the Block is in force"

echo "== restarting the service"
launchctl kickstart -k "system/$LABEL"
sleep 4
check "after restarting the service"

echo "== trying to cancel over IPC (must be refused)"
id=$("$CLI" status | sed -n 's/^Block \(blk-[^ ]*\) ends.*/\1/p' | head -1)
reply=$(echo "{\"request\":\"command\",\"command\":{\"type\":\"cancel_block\",\"block_id\":\"$id\"}}" | nc -U /var/run/cloister.sock)
if [[ $reply == *block_cannot_be_cancelled* ]]; then pass "IPC refused cancel_block"; else fail "IPC replied: $reply"; fi

echo "== app blocker: Calculator stands in for a blocked app"
# Calculator is an Apple app, which is never quit, so block a copy with its own bundle id.
copy=/Applications/CloisterSmokeApp.app
rm -rf "$copy" && cp -R /System/Applications/Calculator.app "$copy"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier com.example.cloister-smoke" "$copy/Contents/Info.plist"
codesign --force --deep -s - "$copy" >/dev/null 2>&1
# A 2-minute extra for one Block, so nothing stays on the Blocklists afterwards.
echo "{\"request\":\"command\",\"command\":{\"type\":\"start_block\",\"block_id\":\"smoke-apps-$$\",\"minutes\":2,\"deep\":false,\"extra_apps\":[\"com.example.cloister-smoke\"]}}" | nc -U /var/run/cloister.sock >/dev/null
sudo -u "${SUDO_USER:-$USER}" open "$copy"
sleep 8
if pgrep -f "$copy/Contents/MacOS" >/dev/null; then fail "a blocked app kept running"; else pass "a blocked app was force-quit"; fi
rm -rf "$copy"

"$CLI" status
echo
if [[ $failures -eq 0 ]]; then
  echo "PASS. Manual step: reboot now, then run 'cloister status' and 'dig @127.0.0.1 youtube.com'"
  echo "before the Block ends; youtube.com must still be NXDOMAIN."
else
  echo "FAILED: $failures check(s)."
  exit 1
fi
