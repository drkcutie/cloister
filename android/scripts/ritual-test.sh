#!/bin/sh
# The Shutdown Ritual on Android (emulator only):
#   - while the ritual is due, the ritual overlay covers other apps (emergency calls stay open);
#   - Cloister itself opens on the ritual flow (the Weekly Review joins on the last Workday);
#   - completing the ritual through the account (as any Device would) lifts the overlay.
# Screenshots of the overlay, the ritual steps and the Week tab go to $SHOTS (default /tmp).
#
# Needs the local Supabase running with this worktree's migrations (take the shared lock),
# JDK 17, the Android SDK with the android-35 Google Play image, and psql. Creates a throwaway
# AVD and deletes it afterwards (KEEP_AVD=1 keeps it). Never point this at a physical phone.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
export JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@17}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
adb="$ANDROID_HOME/platform-tools/adb"
# Its own emulator port, so other emulators (other worktrees) are left alone.
export ANDROID_SERIAL="${ANDROID_SERIAL:-emulator-5570}"
avd=cloister_ritual
pkg=dev.cloister
other_app="${OTHER_APP:-com.google.android.deskclock}"
shots="${SHOTS:-/tmp}"

say() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*"; exit 1; }

cd "$root/backend"
eval "$(supabase status -o env 2>/dev/null | grep -E '^(API_URL|ANON_KEY|DB_URL)=')"
[ -n "${ANON_KEY:-}" ] || fail "local Supabase isn't running"

now_hm=$(date -u +%H:%M)
[ "$now_hm" \> "00:10" ] || fail "too close to midnight (UTC) for a Workday that has ended today; run later"
shutdown=$(date -u -v-2M +%H:%M 2>/dev/null || date -u -d '-2 min' +%H:%M)
isodow=$(date -u +%u)
workdays=$(seq 1 "$isodow" | paste -sd, -)  # today is the last Workday of the week: the Weekly Review joins
next_monday=$(date -u -v+$((8 - isodow))d +%Y-%m-%d 2>/dev/null || date -u -d "+$((8 - isodow)) days" +%Y-%m-%d)

say "Build the debug APK against the local stack (10.0.2.2 from the emulator)"
cd "$root/android"
./gradlew -q :app:assembleDebug -Pcloister.apiUrl=http://10.0.2.2:56421 -Pcloister.anonKey="$ANON_KEY"
apk="$root/android/app/build/outputs/apk/debug/app-debug.apk"

say "Boot a headless emulator"
if ! "$adb" devices | grep -q "^$ANDROID_SERIAL[[:space:]]"; then
  mkdir -p "$HOME/.android/avd/$avd.avd"
  cat > "$HOME/.android/avd/$avd.ini" <<EOF
avd.ini.encoding=UTF-8
path=$HOME/.android/avd/$avd.avd
path.rel=avd/$avd.avd
target=android-35
EOF
  cat > "$HOME/.android/avd/$avd.avd/config.ini" <<EOF
AvdId=$avd
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
  "$ANDROID_HOME/emulator/emulator" -avd "$avd" -port "${ANDROID_SERIAL#emulator-}" -no-window -no-audio -no-boot-anim -no-snapshot \
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
"$adb" shell settings put global window_animation_scale 0
"$adb" shell settings put global transition_animation_scale 0
"$adb" shell settings put global animator_duration_scale 0

say "Install and turn on the app blocker"
"$adb" logcat -c
"$adb" uninstall $pkg >/dev/null 2>&1 || true
"$adb" install -r -t "$apk" >/dev/null
for _ in $(seq 1 10); do
  "$adb" shell settings put secure enabled_accessibility_services $pkg/$pkg.app.enforce.AppWatcher
  "$adb" shell settings put secure accessibility_enabled 1
  sleep 2
  "$adb" shell dumpsys accessibility | grep -q 'label=Cloister app blocker' && break
done
"$adb" shell am start -W -n $pkg/.app.MainActivity >/dev/null

say "Sign the phone in"
email="ritual-$(date +%s)@cloister.test"
password="correct horse battery"
"$adb" shell am broadcast -n $pkg/.app.devtools.DebugReceiver -a dev.cloister.DEBUG_SIGN_IN \
  --es email "$email" --es password "'$password'" >/dev/null
for _ in $(seq 1 30); do
  "$adb" logcat -d -s CloisterDebug | grep -q 'DEBUG_SIGN_IN: ok' && break; sleep 1
done
"$adb" logcat -d -s CloisterDebug | grep -q 'DEBUG_SIGN_IN: ok' || fail "sign-in on the phone"

token=$(curl -sf "$API_URL/auth/v1/token?grant_type=password" -H "apikey: $ANON_KEY" \
  -H 'Content-Type: application/json' -d "{\"email\":\"$email\",\"password\":\"$password\"}" |
  sed -E 's/.*"access_token":"([^"]+)".*/\1/')
command() {
  out=$(curl -sf "$API_URL/rest/v1/rpc/command" -H "apikey: $ANON_KEY" -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' -d "{\"cmd\":$1}")
  echo "$out"
  case "$out" in *accepted*) ;; *) fail "command $1: $out" ;; esac
}
sync_phone() {
  "$adb" shell am broadcast -n $pkg/.app.devtools.DebugReceiver -a dev.cloister.DEBUG_SYNC >/dev/null
  sleep 3
}
tap() {  # tap the first on-screen element whose text is exactly $1
  "$adb" shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
  b=$("$adb" exec-out cat /sdcard/ui.xml | tr '>' '\n' | grep -F "text=\"$1\"" | head -1 |
    sed -E 's/.*bounds="\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]".*/\1 \2 \3 \4/')
  [ -n "$b" ] || fail "no \"$1\" on screen"
  set -- $b
  "$adb" shell input tap $((($1 + $3) / 2)) $((($2 + $4) / 2))
  sleep 2
}
shot() { "$adb" exec-out screencap -p > "$shots/cloister-ritual-$1.png"; echo "screenshot $shots/cloister-ritual-$1.png"; }

say "Make the ritual due: Workdays Monday to today, Shutdown Time $shutdown (UTC), a week of history"
command "{\"type\":\"change_settings\",\"workdays\":[$workdays],\"workday_start\":\"00:00\",\"shutdown_time\":\"$shutdown\",\"time_zone\":\"UTC\"}"
command '{"type":"set_weekly_target","hours":20}'
command '{"type":"record_breach","breach_id":"ritual-test","kind":"vpn_turned_off"}'
# Settings made now have no ritual before them: date them back, and add two rated Deep Sessions
# from earlier today (history, written as the database owner).
psql -q "$DB_URL" <<SQL
set session_replication_role = replica;
update public.settings s set settings_since = s.settings_since - interval '7 days'
  from public.accounts a where a.user_id = s.user_id and a.email = '$email';
insert into public.blocks (user_id, id, starts_at, ends_at, blocklists, deep, intent, depth_rating)
select a.user_id, 'ritual-test-' || n, date_trunc('day', now()) + (n || ' hours')::interval,
       date_trunc('day', now()) + (n || ' hours')::interval + interval '90 minutes',
       array['forums','news','shopping','social','video'], true,
       case n when 1 then 'Draft the chapter' else 'Review the proofs' end, n + 2
from public.accounts a, generate_series(1, 2) n where a.email = '$email';
SQL
sync_phone

say "The ritual overlay covers other apps"
"$adb" shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$other_app" >/dev/null
for _ in $(seq 1 15); do
  "$adb" logcat -d -s CloisterAppWatcher | grep -q "overlay shown: Ritual" && break; sleep 1
done
"$adb" logcat -d -s CloisterAppWatcher | grep -q "overlay shown: Ritual" || fail "no ritual overlay"
sleep 2; shot overlay
echo "ok: ritual overlay shown"

say "Emergency call stays open"
# uiautomator can't see an accessibility overlay: tap by position (1080x2400 layout, bottom buttons).
"$adb" shell input tap 540 2200
sleep 3
fg=$("$adb" shell dumpsys activity activities | grep -m1 -E 'topResumedActivity|mResumedActivity' || true)
echo "$fg"
echo "$fg" | grep -qiE 'dialer|phone|emergency' || fail "the emergency dialer didn't open"
shot emergency
echo "ok: emergency dialer in front, no overlay over it"
"$adb" shell input keyevent KEYCODE_HOME
sleep 2

say "Cloister opens on the ritual"
"$adb" shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$other_app" >/dev/null
sleep 3
"$adb" shell input tap 540 2042   # "Open the Shutdown Ritual" on the overlay
sleep 3
shot step1
command '{"type":"capture_loose_end","loose_end_id":"ritual-test-l1","text":"Send the invoice"}'
command "{\"type\":\"write_day_plan\",\"date\":\"$next_monday\",\"starts_at\":\"${next_monday}T00:00:00Z\",\"ends_at\":\"${next_monday}T${shutdown}:00Z\",\"slots\":[{\"id\":\"ritual-test-s1\",\"starts_at\":\"${next_monday}T00:00:00Z\",\"ends_at\":\"${next_monday}T${shutdown}:00Z\",\"label\":\"Deep work\",\"deep\":true}]}"
sync_phone
shot step1-captured
for n in 2 3 4 5; do tap "Next"; shot "step$n"; done
"$adb" logcat -d -s CloisterAppWatcher > "$shots/cloister-ritual-watcher.log"
if "$adb" logcat -d -s CloisterAppWatcher | grep -qE 'overlay shown: Ritual over (dev.cloister|null)'; then
  fail "the ritual overlay covered Cloister"
fi

say "Completing the ritual through the account lifts the overlay"
command '{"type":"complete_ritual","typed":"Shutdown complete","weekly_target":22}'
sync_phone
"$adb" shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$other_app" >/dev/null
sleep 3
"$adb" logcat -c
"$adb" shell input keyevent KEYCODE_HOME
sleep 1
"$adb" shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$other_app" >/dev/null
sleep 3
if "$adb" logcat -d -s CloisterAppWatcher | grep -q "overlay shown: Ritual"; then fail "the ritual overlay is still shown"; fi
shot after
echo "ok: no ritual overlay once completed"

say "The Week tab"
"$adb" shell am start -W -n $pkg/.app.MainActivity >/dev/null
sleep 2
tap "Week"
shot week

say "The Shutdown Ritual on Android passed"
