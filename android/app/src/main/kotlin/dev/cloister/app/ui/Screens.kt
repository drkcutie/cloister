package dev.cloister.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.cloister.app.enforce.EnforcementNow
import dev.cloister.core.Block
import dev.cloister.core.Command
import dev.cloister.core.DayPlan
import dev.cloister.sync.CommandOutcome
import dev.cloister.sync.Device
import java.time.Duration
import java.time.Instant
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.UUID
import kotlinx.coroutines.launch

// --- Sign-in -----------------------------------------------------------------------------

@Composable
fun SignInScreen(device: Device, configured: Boolean, actions: Actions) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var email by rememberSaveable { mutableStateOf("") }
    var code by rememberSaveable { mutableStateOf("") }
    var sent by rememberSaveable { mutableStateOf(false) }
    var notice by remember { mutableStateOf<String?>(null) }
    Page {
        Masthead("Cloister")
        Text("Sign in", style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() })
        Text(
            "Blocks started on any Device are enforced on this phone too.",
            style = Type.body, color = c.ink2,
        )
        if (!configured) Notice("This build has no backend key. Build with -Pcloister.anonKey=... to sign in.")
        LineField(email, { email = it.trim() }, "Email", keyboard = KeyboardOptions(keyboardType = KeyboardType.Email))
        if (!sent) {
            CommitButton("Send sign-in email", enabled = configured && "@" in email) {
                scope.launch {
                    notice = runCatching { device.sendMagicLink(email) }
                        .fold({ sent = true; "The email has a link and a 6-digit code. Either signs you in." }, { "The email didn't go out. Check the connection." })
                }
            }
        } else {
            LineField(code, { code = it.filter(Char::isDigit).take(6) }, "6-digit code", style = Type.figure, keyboard = KeyboardOptions(keyboardType = KeyboardType.NumberPassword))
            CommitButton("Sign in", enabled = code.length == 6) {
                scope.launch {
                    notice = runCatching { device.verifyEmailCode(email, code) }
                        .fold({ null }, { "That code didn't sign you in. Codes last one hour." })
                }
            }
            TextAction("Send another email") { sent = false; code = "" }
        }
        if (actions.googleConfigured) {
            QuietButton("Continue with Google", Modifier.fillMaxWidth(), enabled = configured) {
                scope.launch {
                    notice = runCatching { actions.signInWithGoogle() }.fold({ null }, { "Google sign-in didn't complete." })
                }
            }
        }
        notice?.let { Notice(it) }
    }
}

// --- Begin a Block -----------------------------------------------------------------------

private val LENGTHS = listOf(25L, 50L, 90L, 120L, 180L)

@Composable
fun BeginScreen(device: Device, now: EnforcementNow, syncLabel: String, openSetup: () -> Unit) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var intent by rememberSaveable { mutableStateOf("") }
    var minutes by rememberSaveable { mutableStateOf(90L) }
    var deep by rememberSaveable { mutableStateOf(true) }
    var notice by remember { mutableStateOf<String?>(null) }
    var sending by remember { mutableStateOf(false) }
    Page {
        Masthead(if (deep) "Begin a Deep Session" else "Begin a Block")
        if (!now.vpnRunning || !now.appBlockerOn) {
            Notice("Enforcement isn't fully set up on this phone.")
            TextAction("Open Setup", onClick = openSetup)
        }
        if (deep) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Eyebrow("Intent", c.ink2)
                IntentField(intent, { intent = it.take(200) }, "What will this produce?")
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Eyebrow("Length", c.ink2)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (m in LENGTHS) Choice(Copy.minutes(m), m == minutes) { minutes = m }
            }
        }
        SquareCheck("Deep Session", deep) { deep = it }
        Text(
            "Ends at ${Copy.clock(device.serverTime().plus(Duration.ofMinutes(minutes)))}. A Block can't be cancelled once it starts.",
            style = Type.detail, color = c.ink2,
        )
        HoldToCommit(if (deep) "Hold to begin the Deep Session" else "Hold to begin the Block", enabled = !sending && (!deep || intent.isNotBlank())) {
            sending = true
            scope.launch {
                val cmd = Command.StartBlock(
                    blockId = "phone-${UUID.randomUUID()}",
                    minutes = minutes,
                    deep = deep,
                    intent = intent.trim().takeIf { deep && it.isNotEmpty() },
                    clientTime = Instant.now(),
                )
                notice = runCatching { device.send(cmd) }.fold(
                    { if (it is CommandOutcome.Rejected) Copy.reason(it.reason) else null },
                    { "Offline. A Block starts once the phone reaches the backend." },
                )
                sending = false
            }
        }
        notice?.let { Notice(it) }
        Status(syncLabel, on = syncLabel == "Online")
    }
}

// --- Evening Lockdown / Rest Day ---------------------------------------------------------

@Composable
fun LockdownScreen(now: EnforcementNow, openTomorrow: () -> Unit) {
    val c = LocalColors.current
    val rest = now.lockdown == dev.cloister.core.Lockdown.RestDay
    Column(
        Modifier.fillMaxSize().padding(horizontal = 28.dp, vertical = 40.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Masthead(if (rest) "Rest Day" else "Evening Lockdown")
        Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(48.dp)) {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    if (rest) "Today is a Rest Day." else "The workday has ended.",
                    style = Type.display, color = c.ink, modifier = Modifier.semantics { heading() },
                )
                Text("Work and distractions are blocked.", style = Type.body, color = c.ink2)
            }
            now.nextWorkdayStart?.let { start ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(Copy.clock(start), style = Type.timer.copy(fontSize = Type.timer.fontSize * 0.78f), color = c.ink)
                    Text("Workday Start, ${Copy.day(start, now.serverTime)}", style = Type.detail, color = c.ink2)
                }
            }
        }
        QuietButton("Tomorrow's Day Plan", Modifier.fillMaxWidth(), onClick = openTomorrow)
    }
}

// --- Deep Session in progress --------------------------------------------------------------

@Composable
fun ActiveScreen(session: Block, serverTime: Instant, enforcing: Boolean, openSetup: () -> Unit) {
    val c = LocalColors.current
    val offset = serverTime.toEpochMilli() - System.currentTimeMillis()
    val now = rememberNow(offset)
    val total = Duration.between(session.startsAt, session.endsAt).seconds.coerceAtLeast(1)
    val elapsed = Duration.between(session.startsAt, now).seconds
    val fraction = (elapsed.toFloat() / total).coerceIn(0f, 1f)
    Column(
        Modifier.fillMaxSize().padding(horizontal = 28.dp, vertical = 40.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Masthead(if (session.quitSocial) "Quit-Social Block" else if (session.deep) "Deep Session" else "Block")
        }
        Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(48.dp)) {
            session.intent?.let {
                Text(it, style = Type.intent.copy(fontSize = Type.intent.fontSize * 1.06f), color = c.ink)
            }
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Text(Copy.countdown(Duration.between(now, session.endsAt)), style = Type.timer, color = c.ink)
                Progress(fraction, "${(fraction * 100).toInt()} percent of this ${if (session.deep) "Deep Session" else "Block"} has elapsed")
                Text("Ends at ${Copy.clock(session.endsAt)}", style = Type.detail, color = c.ink2)
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (!enforcing) TextAction("Enforcement isn't fully set up. Open Setup.", onClick = openSetup)
            Text("This Block can't be cancelled.", style = Type.detail, color = c.ink3)
        }
    }
}

// --- Depth Rating --------------------------------------------------------------------------

@Composable
fun DepthRatingScreen(device: Device, session: Block, later: () -> Unit) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var chosen by remember { mutableStateOf<Int?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    Page {
        Masthead("Depth Rating")
        Text("How deep was it?", style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() })
        session.intent?.let { Text(it, style = Type.intent.copy(fontSize = Type.intent.fontSize * 0.8f), color = c.ink2) }
        Text("${Copy.clock(session.startsAt)} to ${Copy.clock(session.endsAt)}", style = Type.figure, color = c.ink2)
        DepthRatingSquares(chosen) { chosen = it }
        Row(Modifier.width(312.dp), horizontalArrangement = Arrangement.SpaceBetween) {
            Text("1 shallow", style = Type.label, color = c.ink2)
            Text("5 deepest", style = Type.label, color = c.ink2)
        }
        CommitButton("Record the Depth Rating", enabled = chosen != null) {
            val rating = chosen ?: return@CommitButton
            scope.launch {
                notice = runCatching { device.send(Command.RecordDepthRating(session.id, rating)) }.fold(
                    { if (it is CommandOutcome.Rejected) Copy.reason(it.reason) else null },
                    { "Offline. The Depth Rating can be recorded once the phone is online." },
                )
            }
        }
        TextAction("Later", onClick = later)
        notice?.let { Notice(it) }
    }
}

// --- Today's Day Plan (read-only) ----------------------------------------------------------

@Composable
fun DayPlanScreen(plan: DayPlan?, saved: Boolean, now: Instant, tomorrow: Boolean = false, onToggleDay: () -> Unit = {}) {
    val c = LocalColors.current
    val dateFmt = DateTimeFormatter.ofPattern("EEEE d MMMM", Locale.getDefault())
    Page {
        Eyebrow(if (tomorrow) "Tomorrow's Day Plan" else "Day Plan")
        TextAction(if (tomorrow) "Show today" else "Show tomorrow", onClick = onToggleDay)
        if (plan == null) {
            Text(if (tomorrow) "No Day Plan tomorrow." else "No Day Plan today.", style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() })
            Notice("Write tomorrow's Day Plan on the desktop during the Shutdown Ritual.")
            return@Page
        }
        Text(dateFmt.format(plan.date), style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() })
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("${Copy.clock(plan.startsAt)} to ${Copy.clock(plan.endsAt)}", style = Type.figure, color = c.ink2)
            if (!saved) Tag("From Schedule", c.sunk, c.ink2)
        }
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(Copy.minutes(plan.plannedDeepMinutes), style = Type.figure.copy(fontSize = Type.figure.fontSize * 1.6f), color = Depth.d4)
            Text("Planned Deep Minutes", style = Type.label, color = c.ink2, modifier = Modifier.padding(bottom = 4.dp))
        }
        if (plan.slots.isEmpty()) Notice("No Slots on this Day Plan.")
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (slot in plan.slots) {
                val current = !now.isBefore(slot.startsAt) && now.isBefore(slot.endsAt)
                Row(
                    Modifier.fillMaxWidth()
                        .background(if (slot.deep) Depth.d1 else c.surface)
                        .padding(horizontal = 12.dp, vertical = 12.dp)
                        .semantics(mergeDescendants = true) {},
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    Column(Modifier.width(64.dp)) {
                        Text(Copy.clock(slot.startsAt), style = Type.figure, color = if (slot.deep) Depth.d5 else c.ink)
                        Text(Copy.clock(slot.endsAt), style = Type.figure, color = if (slot.deep) Depth.d4 else c.ink2)
                    }
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(slot.label, style = Type.body, color = if (slot.deep) Depth.d5 else c.ink)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            if (slot.deep) Tag("Deep", Depth.d4, androidx.compose.ui.graphics.Color.White)
                            if (current) Status("Now", on = true)
                            slot.depthRating?.let { Text("Depth $it", style = Type.label, color = Depth.d5) }
                        }
                    }
                }
            }
        }
        Text("Day Plans are written on the desktop. Here they are read-only.", style = Type.detail, color = c.ink3)
    }
}

// --- Setup ---------------------------------------------------------------------------------

@Composable
fun SetupScreen(device: Device, now: EnforcementNow, actions: Actions) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    Page {
        Eyebrow("Setup")
        Text("Enforcement on this phone", style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() })
        Step("01", "VPN filter", "Blocked sites don't load in any app.", now.vpnRunning, "Turn on", actions::turnOnVpn)
        Step("02", "App blocker", "Blocked apps are covered by an overlay with your Intent. In Accessibility, choose Cloister app blocker.", now.appBlockerOn, "Open Accessibility", actions::openAccessibilitySettings)
        Step("03", "Device Admin", "Cloister can't be uninstalled during a Block.", now.adminActive, "Turn on", actions::turnOnDeviceAdmin)
        Step("04", "Notifications", "Shows that enforcement is running.", null, "Allow", actions::allowNotifications)
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Hardcore Mode", style = Type.heading, color = c.ink)
            if (now.deviceOwner) {
                Status("On. The VPN can't be turned off and Cloister can't be uninstalled during a Block.", on = true)
            } else {
                Text(
                    "Optional. Makes turning enforcement off impossible rather than recorded. It needs a computer: " +
                        "remove every account from the phone for a moment, then run " +
                        "adb shell dpm set-device-owner dev.cloister/.app.enforce.AdminReceiver, then add the accounts back.",
                    style = Type.detail, color = c.ink2,
                )
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Account", style = Type.heading, color = c.ink)
            Text(device.email ?: "Signed in", style = Type.detail, color = c.ink2)
            if (now.session == null) TextAction("Sign out") { scope.launch { device.signOut() } }
        }
        val breaches = device.state().breaches
        if (breaches.isNotEmpty()) {
            Notice("${breaches.size} Breach${if (breaches.size == 1) "" else "es"} recorded. Last at ${Copy.clock(breaches.last().occurredAt)}.", recorded = true)
        }
    }
}

@Composable
private fun Step(number: String, title: String, detail: String, on: Boolean?, action: String, onAction: () -> Unit) {
    val c = LocalColors.current
    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(number, style = Type.figure, color = c.brass)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(title, style = Type.heading, color = c.ink)
            Text(detail, style = Type.detail, color = c.ink2)
            if (on != null) Status(if (on) "On" else "Off", on)
            if (on != true) QuietButton(action, onClick = onAction)
        }
    }
}
