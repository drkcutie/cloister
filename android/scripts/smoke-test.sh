#!/bin/sh
# Seam B on Android (emulator only): a Block started through the backend (as the desktop would)
# reaches the phone over Realtime, then
#   - blocked domains and their subdomains don't resolve through the VPN, others do;
#   - opening a blocked app shows the blocked-app overlay.
#
# Needs the local Supabase running (backend/scripts/test.sh starts it; take the shared lock
# described in the notes if other worktrees use the stack), JDK 17 and the Android SDK with the
# android-35 Google Play system image. Creates a throwaway AVD and deletes it afterwards
# (KEEP_AVD=1 keeps it). Never point this at a physical phone.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
export JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@17}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
adb="$ANDROID_HOME/platform-tools/adb"
avd=cloister_smoke
pkg=dev.cloister
blocked_app="${BLOCKED_APP:-com.google.android.deskclock}"

say() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*"; exit 1; }

cd "$root/backend"
eval "$(supabase status -o env 2>/dev/null | grep -E '^(API_URL|ANON_KEY)=')"
[ -n "${ANON_KEY:-}" ] || fail "local Supabase isn't running"

say "Build the debug APK against the local stack (10.0.2.2 from the emulator)"
cd "$root/android"
./gradlew -q :app:assembleDebug -Pcloister.apiUrl=http://10.0.2.2:56421 -Pcloister.anonKey="$ANON_KEY"
apk="$root/android/app/build/outputs/apk/debug/app-debug.apk"

say "Boot a headless emulator"
if ! "$adb" devices | grep -q '^emulator-'; then
  mkdir -p "$HOME/.android/avd/$avd.avd"
  cat > "$HOME/.android/avd/$avd.ini" <<EOF
avd.ini.encoding=UTF-8
path=$HOME/.android/avd/$avd.avd
path.rel=avd/$avd.avd
target=android-35
EOF
  cat > "$HOME/.android/avd/$avd.avd/config.ini" <<'EOF'
AvdId=cloister_smoke
abi.type=arm64-v8a
hw.cpu.arch=arm64
hw.cpu.ncore=2
hw.ramSize=2048
disk.dataPartition.size=3G
image.sysdir.1=system-images/android-35/google_apis_playstore/arm64-v8a/
tag.id=google_apis_playstore
PlayStore.enabled=true
hw.lcd.width=1080
hw.lcd.height=2400
hw.lcd.density=420
hw.gpu.enabled=yes
hw.gpu.mode=swiftshader_indirect
EOF
  "$ANDROID_HOME/emulator/emulator" -avd "$avd" -no-window -no-audio -no-boot-anim -no-snapshot \
    -gpu swiftshader_indirect >/tmp/cloister-emulator.log 2>&1 &
  started_emulator=1
fi
cleanup() {
  if [ "${started_emulator:-}" = 1 ] && [ -z "${KEEP_AVD:-}" ]; then
    "$adb" emu kill >/dev/null 2>&1 || true
    sleep 3
    rm -rf "$HOME/.android/avd/$avd.avd" "$HOME/.android/avd/$avd.ini"
  fi
}
trap cleanup EXIT
"$adb" wait-for-device
until [ "$("$adb" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do sleep 5; done
"$adb" shell pm list packages | grep -q "package:$blocked_app\$" || fail "$blocked_app isn't installed on the image"

say "Install, grant the VPN and turn on the app blocker (what onboarding asks the user to do)"
"$adb" logcat -c
"$adb" install -r -t "$apk" >/dev/null
"$adb" shell appops set $pkg ACTIVATE_VPN allow
# Right after an install the setting can be dropped before the service is registered: retry.
for _ in $(seq 1 10); do
  "$adb" shell settings put secure enabled_accessibility_services $pkg/$pkg.app.enforce.AppWatcher
  "$adb" shell settings put secure accessibility_enabled 1
  sleep 2
  "$adb" shell dumpsys accessibility | grep -q 'label=Cloister app blocker' && break
done
"$adb" shell am start -W -n $pkg/.app.MainActivity >/dev/null
# The VPN starts once the user has granted it; tap "Turn on" equivalently:
"$adb" shell am start -W -n $pkg/.app.MainActivity -a dev.cloister.TURN_ON_VPN >/dev/null
vpn_up() { "$adb" shell dumpsys connectivity | grep -q 'VPN CONNECTED extra: VPN:dev.cloister'; }
for _ in $(seq 1 30); do vpn_up && break; sleep 1; done
vpn_up || fail "the VPN didn't come up"

say "Sign the phone in (local email + password provider, debug build only)"
email="smoke-$(date +%s)@cloister.test"
password="correct horse battery"
"$adb" shell am broadcast -n $pkg/.app.devtools.DebugReceiver -a dev.cloister.DEBUG_SIGN_IN \
  --es email "$email" --es password "'$password'" >/dev/null
for _ in $(seq 1 30); do
  "$adb" logcat -d -s CloisterDebug | grep -q 'DEBUG_SIGN_IN: ok' && break; sleep 1
done
"$adb" logcat -d -s CloisterDebug | grep -q 'DEBUG_SIGN_IN: ok' || fail "sign-in on the phone: $("$adb" logcat -d -s CloisterDebug)"

say "Start a Block from 'the desktop' (the same account over the REST API)"
token=$(curl -sf "$API_URL/auth/v1/token?grant_type=password" -H "apikey: $ANON_KEY" \
  -H 'Content-Type: application/json' -d "{\"email\":\"$email\",\"password\":\"$password\"}" |
  sed -E 's/.*"access_token":"([^"]+)".*/\1/')
curl -sf "$API_URL/rest/v1/rpc/command" -H "apikey: $ANON_KEY" -H "Authorization: Bearer $token" \
  -H 'Content-Type: application/json' \
  -d "{\"cmd\":{\"type\":\"start_block\",\"block_id\":\"smoke-$(date +%s)\",\"minutes\":30,\"intent\":\"Smoke test the phone\",\"extra_apps\":[\"$blocked_app\"]}}"
echo

say "Blocked domains don't resolve; others do"
resolves() { "$adb" shell ping -c 1 -W 3 "$1" 2>&1 | grep -q '^PING '; }
# Wait for the Block to reach the phone before resolving anything: Android caches positive answers.
reached() { "$adb" logcat -d | grep -E 'CloisterVpn.*blocking [1-9][0-9]* domains' >/dev/null; }
for _ in $(seq 1 60); do reached && break; sleep 1; done
reached || fail "the Block didn't reach the phone"
for d in youtube.com m.youtube.com youtu.be reddit.com; do
  if resolves "$d"; then fail "$d still resolves"; else echo "ok: $d doesn't resolve"; fi
done
for d in example.com; do
  if resolves "$d"; then echo "ok: $d resolves"; else fail "$d doesn't resolve (upstream DNS through the VPN broken)"; fi
done

say "A blocked app gets the overlay"
"$adb" shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$blocked_app" >/dev/null
for _ in $(seq 1 15); do
  "$adb" logcat -d -s CloisterAppWatcher | grep -q "overlay shown: BlockedApp over $blocked_app" && break; sleep 1
done
"$adb" logcat -d -s CloisterAppWatcher | grep -q "overlay shown: BlockedApp over $blocked_app" || fail "no overlay over $blocked_app"
"$adb" exec-out screencap -p > /tmp/cloister-smoke-overlay.png
echo "ok: overlay shown (screenshot /tmp/cloister-smoke-overlay.png)"

say "Seam B on Android passed"
