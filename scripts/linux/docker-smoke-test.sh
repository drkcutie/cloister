#!/bin/sh
# Runs the Linux Seam B smoke test (smoke-test.sh) in a privileged Alpine container, so it needs
# no Linux machine and changes nothing on the host. Builds cloisterd and the CLI in a container
# (cached in the docker volume cloister-linux-build; remove it with
# `docker volume rm cloister-linux-build`).
#
# Usage: scripts/linux/docker-smoke-test.sh [MINUTES]
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
VOLUME=cloister-linux-build

docker volume create $VOLUME >/dev/null
docker run --rm -v "$REPO":/src:ro -v $VOLUME:/build alpine:3.24 sh -ec '
  apk add -q rust cargo
  cd /src
  CARGO_HOME=/build/cargo CARGO_TARGET_DIR=/build/target \
    cargo build --locked -q -p cloister-enforcer -p cloister-cli
'
docker run --rm --privileged -v "$REPO/scripts/linux":/scripts:ro -v $VOLUME:/build:ro alpine:3.24 sh -c "
  set -e
  apk add -q libgcc coreutils nftables bind-tools socat
  install -m 755 /build/target/debug/cloisterd /usr/local/libexec/cloisterd 2>/dev/null \
    || { mkdir -p /usr/local/libexec; install -m 755 /build/target/debug/cloisterd /usr/local/libexec/cloisterd; }
  install -m 755 /build/target/debug/cloister /usr/local/bin/cloister
  set +e
  CLOISTER_SPAWN=1 YES=1 sh /scripts/smoke-test.sh ${1:-3}
  status=\$?
  echo '--- cloisterd log'; tail -20 /tmp/cloisterd.log
  exit \$status
"
