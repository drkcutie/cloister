#!/bin/bash
# Build and install the Cloister enforcement service as a root launchd daemon.
# Usage: sudo scripts/macos/install.sh
set -euo pipefail

LABEL=com.cloister.enforcer
PLIST=/Library/LaunchDaemons/$LABEL.plist
DAEMON=/usr/local/libexec/cloisterd
CLI=/usr/local/bin/cloister
REPO=$(cd "$(dirname "$0")/../.." && pwd)

[[ $EUID -eq 0 ]] || { echo "Run with sudo." >&2; exit 1; }

# Build as the invoking user so target/ isn't owned by root.
BUILD_USER=${SUDO_USER:-root}
sudo -u "$BUILD_USER" -H bash -lc "cd '$REPO' && source \"\$HOME/.cargo/env\" 2>/dev/null; cargo build --release -p cloister-enforcer -p cloister-cli"
TARGET_DIR=$(sudo -u "$BUILD_USER" -H bash -lc "cd '$REPO' && source \"\$HOME/.cargo/env\" 2>/dev/null; cargo metadata --format-version 1 --no-deps" | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')

install -d -o root -g wheel /usr/local/libexec /usr/local/bin
install -o root -g wheel -m 755 "$TARGET_DIR/release/cloisterd" "$DAEMON"
install -o root -g wheel -m 755 "$TARGET_DIR/release/cloister" "$CLI"
"$DAEMON" plist --binary "$DAEMON" > "$PLIST"
chown root:wheel "$PLIST"
chmod 644 "$PLIST"

launchctl bootout system "$PLIST" 2>/dev/null || true
launchctl bootstrap system "$PLIST"
launchctl enable "system/$LABEL"

for _ in $(seq 1 20); do
  [[ -S /var/run/cloister.sock ]] && break
  sleep 0.5
done
"$CLI" status
echo "Installed. System DNS now points at the Cloister filter (127.0.0.1)."
