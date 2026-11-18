package dev.cloister.app.ui

import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.cloister.app.enforce.EnforcementNow
import dev.cloister.core.HardcoreGuide
import dev.cloister.core.HardcoreStep
import dev.cloister.core.Onboarding
import dev.cloister.core.OnboardingStep
import dev.cloister.core.PhoneProgress
import dev.cloister.core.WorkdaysForm
import dev.cloister.sync.CommandOutcome
import dev.cloister.sync.Device
import java.time.ZoneId
import kotlinx.coroutines.launch

/** Whether onboarding has been finished on this phone. */
object OnboardingMarker {
    private fun prefs(context: Context) = context.getSharedPreferences("onboarding", Context.MODE_PRIVATE)
    fun isDone(context: Context) = prefs(context).getBoolean("done", false)
    fun markDone(context: Context) = prefs(context).edit().putBoolean("done", true).apply()
}

/**
 * First run: philosophy, sign-in, then VPN, app blocker and Device Admin one at a time, the
 * same Workdays settings as the desktop, and the optional Hardcore Mode guide. What to show
 * comes from [Onboarding] in `core`; steps already done are skipped.
 */
@Composable
fun OnboardingScreen(device: Device, now: EnforcementNow, version: Long, configured: Boolean, actions: Actions, finished: () -> Unit) {
    val progress = PhoneProgress(
        signedIn = device.signedIn,
        vpnOn = now.vpnRunning,
        appBlockerOn = now.appBlockerOn,
        adminActive = now.adminActive,
        settingsSet = remember(version) { device.state().settings != null },
        deviceOwner = now.deviceOwner,
    )
    val flow = remember { Onboarding() }
    var step by remember { mutableStateOf(flow.step) }
    val next = { step = flow.next(progress) ?: run { finished(); step } }
    // Signing in is how the sign-in step ends (by code, link or Google).
    LaunchedEffect(progress.signedIn, step) {
        if (step == OnboardingStep.SignIn && progress.signedIn) next()
    }
    when (val s = step) {
        is OnboardingStep.Philosophy -> {
            val page = Onboarding.PHILOSOPHY[s.page]
            StepPage("Why Cloister is strict, ${s.page + 1} of ${Onboarding.PHILOSOPHY.size}", page.title, page.body) {
                Nav(back = if (s.page > 0) ({ step = flow.back() }) else null, next = next)
            }
        }
        OnboardingStep.SignIn -> if (configured) {
            SignInScreen(device, configured, actions)
        } else {
            StepPage("Sign in", "One account, every Device", "This build has no backend key, so this phone enforces on its own for now.") {
                Nav(back = { step = flow.back() }, next = next, label = "Not now")
            }
        }
        OnboardingStep.Vpn -> GrantStep(
            "1 of 3", "The VPN filter",
            "Blocked sites don't load in any app. Cloister's VPN only answers name lookups; your traffic doesn't go through it.",
            on = now.vpnRunning, action = "Turn on the VPN", onAction = actions::turnOnVpn,
            back = { step = flow.back() }, next = next,
        )
        OnboardingStep.AppBlocker -> GrantStep(
            "2 of 3", "The app blocker",
            "Blocked apps are covered by a calm overlay with your Intent. In Accessibility, choose Cloister app blocker and turn it on. " +
                "On Android 13 and later, a sideloaded app first needs App info, the three-dot menu, Allow restricted settings.",
            on = now.appBlockerOn, action = "Open Accessibility", onAction = actions::openAccessibilitySettings,
            back = { step = flow.back() }, next = next,
        )
        OnboardingStep.DeviceAdmin -> GrantStep(
            "3 of 3", "Device Admin",
            "Cloister can't be uninstalled while a Block runs.",
            on = now.adminActive, action = "Turn on Device Admin", onAction = actions::turnOnDeviceAdmin,
            back = { step = flow.back() }, next = next,
        )
        OnboardingStep.Workdays -> WorkdaysStep(device, back = { step = flow.back() }, next = next)
        OnboardingStep.Hardcore -> HardcoreStepScreen(now.deviceOwner, back = { step = flow.back() }, finish = next)
    }
}

@Composable
private fun StepPage(eyebrow: String, title: String, body: String, content: @Composable () -> Unit) {
    val c = LocalColors.current
    Page {
        Masthead(eyebrow)
        Text(title, style = Type.display, color = c.ink, modifier = Modifier.semantics { heading() })
        if (body.isNotEmpty()) Text(body, style = Type.body, color = c.ink2)
        content()
    }
}

@Composable
private fun Nav(back: (() -> Unit)?, next: () -> Unit, label: String = "Continue", enabled: Boolean = true) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        if (back != null) QuietButton("Back", onClick = back)
        CommitButton(label, Modifier.weight(1f), enabled = enabled, onClick = next)
    }
}

@Composable
private fun GrantStep(
    eyebrow: String, title: String, detail: String, on: Boolean, action: String, onAction: () -> Unit,
    back: () -> Unit, next: () -> Unit,
) {
    StepPage("Enforcement on this phone, $eyebrow", title, detail) {
        Status(if (on) "On" else "Off", on)
        if (!on) QuietButton(action, Modifier.fillMaxWidth(), onClick = onAction)
        Nav(back, next, label = if (on) "Continue" else "Not now")
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun WorkdaysStep(device: Device, back: () -> Unit, next: () -> Unit) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var form by remember { mutableStateOf(WorkdaysForm(timeZone = ZoneId.systemDefault().id)) }
    var notice by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    StepPage(
        "Workdays", "When work happens, and when it stops",
        "At Shutdown Time the Shutdown Ritual begins and Evening Lockdown closes work apps until Workday Start. " +
            "Days that aren't Workdays are Rest Days. Later, anything looser takes effect the next day.",
    ) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (day in 1..7) {
                val on = day in form.workdays
                QuietButton((if (on) "■ " else "□ ") + WorkdaysForm.dayName(day)) { form = form.toggle(day) }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(24.dp)) {
            Column(Modifier.weight(1f)) {
                LineField(form.workdayStart, { form = form.copy(workdayStart = it) }, "Workday Start", Type.figure, KeyboardOptions(keyboardType = KeyboardType.Number))
            }
            Column(Modifier.weight(1f)) {
                LineField(form.shutdownTime, { form = form.copy(shutdownTime = it) }, "Shutdown Time", Type.figure, KeyboardOptions(keyboardType = KeyboardType.Number))
            }
        }
        Text("${form.summary()} Times are in ${form.timeZone}.", style = Type.detail, color = c.ink2)
        notice?.let { Notice(it) }
        Nav(back, enabled = !saving, next = {
            val command = form.command()
            command.exceptionOrNull()?.let { notice = it.message; return@Nav }
            // Nothing to save to without an account (a build with no backend): go on, as on the desktop.
            if (!device.signedIn) return@Nav next()
            saving = true
            scope.launch {
                val outcome = runCatching { device.send(command.getOrThrow()) }
                saving = false
                when (val o = outcome.getOrNull()) {
                    CommandOutcome.Accepted -> next()
                    is CommandOutcome.Rejected -> notice = Copy.reason(o.reason)
                    null -> notice = "Settings couldn't be saved: the backend isn't reachable. Try again."
                }
            }
        })
    }
}

@Composable
private fun HardcoreStepScreen(deviceOwner: Boolean, back: () -> Unit, finish: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    var guide by remember { mutableStateOf<HardcoreGuide?>(null) }
    val g = guide
    if (g == null) {
        StepPage(
            "Optional", "Hardcore Mode",
            "Without it, turning the VPN or the app blocker off during a Block is possible, and recorded as a Breach. " +
                "With it, the VPN and Cloister can't be turned off or uninstalled during a Block. It needs a computer with adb, " +
                "and every account removed from the phone for a few minutes.",
        ) {
            if (deviceOwner) Status("Hardcore Mode is on.", on = true)
            else QuietButton("Set up Hardcore Mode", Modifier.fillMaxWidth()) { guide = HardcoreGuide() }
            Nav(back, finish, label = if (deviceOwner) "Finish" else "Not now, finish")
        }
        return
    }
    val (number, title, body) = when (g.step) {
        HardcoreStep.RemoveAccounts -> Triple(
            "Step 1 of 4", "Remove every account for now",
            "Android only lets an app become Device Owner on a phone with no accounts. In Settings, Passwords and accounts, " +
                "remove each account (Google and others). Your data stays in the accounts; you add them back in step 4.",
        )
        HardcoreStep.RunAdb -> Triple(
            "Step 2 of 4", "Run one command from a computer",
            "Turn on USB debugging (Developer options), connect the phone to a computer with adb, and run:",
        )
        HardcoreStep.Verify -> Triple(
            "Step 3 of 4", "Check it worked",
            "Cloister asks Android whether it is the Device Owner now.",
        )
        HardcoreStep.AddAccountsBack -> Triple(
            "Step 4 of 4", "Hardcore Mode is on",
            "Add your accounts back in Settings, Passwords and accounts. The VPN and Cloister can no longer be turned off during a Block, " +
                "and your Devices show this phone as enforcing with Hardcore Mode.",
        )
    }
    StepPage("Hardcore Mode, $number", title, body) {
        when (g.step) {
            HardcoreStep.RemoveAccounts ->
                QuietButton("Open account settings", Modifier.fillMaxWidth()) { context.startActivity(Intent(Settings.ACTION_SYNC_SETTINGS)) }
            HardcoreStep.RunAdb -> {
                Text(HardcoreGuide.ADB_COMMAND, style = Type.figure, color = c.ink)
                TextAction("Copy the command") { clipboard.setText(AnnotatedString(HardcoreGuide.ADB_COMMAND)) }
            }
            HardcoreStep.AddAccountsBack -> {
                Status("Device Owner", on = true)
                QuietButton("Open account settings", Modifier.fillMaxWidth()) { context.startActivity(Intent(Settings.ACTION_SYNC_SETTINGS)) }
            }
            HardcoreStep.Verify -> {}
        }
        g.notice?.let { Notice(it) }
        Nav(
            back = { guide = if (g.step == HardcoreStep.RemoveAccounts) null else HardcoreGuide(HardcoreStep.entries[g.step.ordinal - 1]) },
            next = { if (g.finished) finish() else guide = g.next(deviceOwner) },
            label = when (g.step) {
                HardcoreStep.Verify -> "Check"
                HardcoreStep.AddAccountsBack -> "Finish"
                else -> "Continue"
            },
        )
    }
}
