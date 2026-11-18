#!/bin/sh
# Seam B smoke test for Linux: given an Enforcement, blocked domains and their subdomains don't
# resolve for any non-root process (nftables redirects all DNS to the filter), DNS over TCP and
# DNS-over-TLS can't bypass it, the hosts fallback is written, a blocked app is killed, tampering
# with the firewall is undone, and all of it still holds after the service restarts.
#
# Usage (as root):  scripts/linux/smoke-test.sh [MINUTES]     (default 3)
#   Installed with systemd (`cloisterd install --app ...`): restarts it with systemctl.
#   Without systemd (a container; see docker-smoke-test.sh): CLOISTER_SPAWN=1 starts
#   $CLOISTERD itself and restarts it by killing and starting it again, as systemd would.
# Env: CLOISTERD (default /usr/local/libexec/cloisterd), CLOISTER (CLI, default `cloister`),
#      YES=1 to skip the confirmation.
#
# It starts a real Block of MINUTES with the Distraction List, which CAN'T be cancelled.
# Needs: dig (bind-tools/dnsutils), nft, socat, nc.
set -u

MINUTES=${1:-3}
CLOISTERD=${CLOISTERD:-/usr/local/libexec/cloisterd}
CLI=${CLOISTER:-cloister}
SOCKET=/var/run/cloister.sock
BLOCKED="youtube.com www.youtube.com m.youtube.com youtu.be rr1---sn-abc.googlevideo.com old.reddit.com x.com"
ALLOWED="example.com wikipedia.org"
NS=$(awk '/^nameserver/ { print $2; exit }' /etc/resolv.conf)
failures=0

[ "$(id -u)" -eq 0 ] || { echo "Run as root." >&2; exit 1; }
for tool in dig nft socat nc; do
  command -v $tool >/dev/null || { echo "Missing $tool." >&2; exit 1; }
done

pass() { echo "  ok    $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }
as_user() { su nobody -s /bin/sh -c "$*"; }

start_service() {
  if [ "${CLOISTER_SPAWN:-0}" = 1 ]; then
    "$CLOISTERD" run --tick 2 >>/tmp/cloisterd.log 2>&1 &
    echo $! >/tmp/cloisterd.pid
  else
    systemctl restart cloisterd.service
  fi
  for _ in $(seq 1 40); do [ -S $SOCKET ] && "$CLI" status >/dev/null 2>&1 && return; sleep 0.25; done
  echo "The service didn't come up." >&2; exit 1
}

restart_service() {
  if [ "${CLOISTER_SPAWN:-0}" = 1 ]; then
    kill -9 "$(cat /tmp/cloisterd.pid)" 2>/dev/null; sleep 1
  fi
  start_service
}

ipc() { echo "$1" | socat - UNIX-CONNECT:$SOCKET; }

check() {
  echo "== $1"
  for name in $BLOCKED; do
    if dig +time=2 +tries=1 @127.0.0.1 "$name" | grep -q 'status: NXDOMAIN'; then
      pass "filter: $name is NXDOMAIN"
    else
      fail "filter: $name is not NXDOMAIN"
    fi
    # A non-root process asking the system's resolver ($NS) is redirected to the filter.
    if as_user "dig +time=2 +tries=1 @$NS $name" | grep -q 'status: NXDOMAIN'; then
      pass "redirect: $name via $NS is NXDOMAIN for a user"
    else
      fail "redirect: $name via $NS is not NXDOMAIN for a user"
    fi
  done
  for name in $ALLOWED; do
    if as_user "dig +short +time=3 +tries=2 @$NS $name A" | grep -qE '^[0-9.]+$'; then
      pass "redirect: $name resolves for a user"
    else
      fail "redirect: $name doesn't resolve for a user"
    fi
  done
  if as_user "dig +short +time=2 +tries=1 @1.1.1.1 youtube.com" | grep -qE '^[0-9.]+$'; then
    fail "nftables: a user's UDP DNS to 1.1.1.1 got an answer for youtube.com"
  else
    pass "nftables: a user's UDP DNS to 1.1.1.1 is answered by the filter"
  fi
  if as_user "dig +tcp +time=2 +tries=1 @1.1.1.1 example.com" | grep -q 'status: NOERROR'; then
    fail "nftables: DNS over TCP to 1.1.1.1 got through"
  else
    pass "nftables: DNS over TCP to 1.1.1.1 is dropped"
  fi
  if as_user "nc -z -w 2 1.1.1.1 853" 2>/dev/null; then
    fail "nftables: DNS-over-TLS to 1.1.1.1 got through"
  else
    pass "nftables: DNS-over-TLS to 1.1.1.1 is dropped"
  fi
  if nft list table inet cloister >/dev/null 2>&1; then pass "nftables: table loaded"; else fail "nftables: no cloister table"; fi
  if grep -q "$(printf '0.0.0.0\tyoutube.com')" /etc/hosts; then pass "hosts: fallback entries present"; else fail "hosts: no fallback entries"; fi
}

if [ "${YES:-0}" != 1 ]; then
  echo "This starts a $MINUTES-minute Block that can't be cancelled. Continue? [y/N]"
  read -r answer
  [ "$answer" = y ] || exit 1
fi

[ "${CLOISTER_SPAWN:-0}" = 1 ] && start_service
"$CLI" block start --minutes "$MINUTES" --plain || { echo "Couldn't start a Block." >&2; exit 1; }
sleep 3
check "while the Block is in force"

echo "== restarting the service"
restart_service
sleep 3
check "after restarting the service"

echo "== trying to cancel over IPC (must be refused)"
id=$("$CLI" status | sed -n 's/^Block \(blk-[^ ]*\) ends.*/\1/p' | head -1)
reply=$(ipc "{\"request\":\"command\",\"command\":{\"type\":\"cancel_block\",\"block_id\":\"$id\"}}")
case $reply in *loosening_not_accepted*) pass "IPC refused cancel_block" ;; *) fail "IPC replied: $reply" ;; esac

echo "== app blocker: a copy of sleep named SmokeApp stands in for a blocked app"
sleep_bin=$(readlink -f "$(command -v sleep)")
mkdir -p /tmp/cloister-smoke && cp "$sleep_bin" /tmp/cloister-smoke/SmokeApp
chmod 755 /tmp/cloister-smoke /tmp/cloister-smoke/SmokeApp
# GNU coreutils built as one multi-call binary (Alpine) needs to be told which program to be.
case $sleep_bin in */coreutils) prog=--coreutils-prog=sleep ;; *) prog= ;; esac
as_user "/tmp/cloister-smoke/SmokeApp $prog 300 >/dev/null 2>&1 &"
sleep 1
if pgrep -f /tmp/cloister-smoke/SmokeApp >/dev/null; then
  ipc "{\"request\":\"command\",\"command\":{\"type\":\"start_block\",\"block_id\":\"smoke-apps-$$\",\"minutes\":2,\"deep\":false,\"extra_apps\":[\"SmokeApp\"]}}" >/dev/null
  sleep 8
  if pgrep -f /tmp/cloister-smoke/SmokeApp >/dev/null; then fail "a blocked app kept running"; else pass "a blocked app was killed"; fi
else
  fail "the stand-in app didn't start (busybox's sleep can't be renamed; install coreutils)"
fi
rm -rf /tmp/cloister-smoke

echo "== tampering: deleting the nftables table (it must come back within ~35 s)"
nft delete table inet cloister
back=0
for _ in $(seq 1 45); do
  if nft list table inet cloister >/dev/null 2>&1; then back=1; break; fi
  sleep 1
done
if [ $back = 1 ]; then pass "the table was loaded again"; else fail "the table stayed deleted"; fi

"$CLI" status
echo
if [ $failures -eq 0 ]; then
  echo "PASS"
  [ "${CLOISTER_SPAWN:-0}" = 1 ] || echo "Manual step: reboot, then check 'cloister status' and 'dig @127.0.0.1 youtube.com' before the Block ends."
else
  echo "FAILED: $failures check(s)."
  exit 1
fi
