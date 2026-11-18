package dev.cloister.app.ui

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.cloister.app.enforce.Enforcer
import dev.cloister.core.Block
import dev.cloister.core.DayPlans
import dev.cloister.sync.Device
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId

/** What the screens can ask of the Activity (permissions, system settings, Credential Manager). */
interface Actions {
    fun turnOnVpn()
    fun openAccessibilitySettings()
    fun turnOnDeviceAdmin()
    fun allowNotifications()
    val googleConfigured: Boolean
    suspend fun signInWithGoogle()
}

enum class Tab(val label: String) { Today("Today"), DayPlan("Day Plan"), Week("Week"), Setup("Setup") }

@Composable
fun App(device: Device, enforcer: Enforcer, configured: Boolean, actions: Actions) {
    val now by enforcer.now.collectAsState()
    val version by device.version.collectAsState()
    val status by device.status.collectAsState()
    var tab by rememberSaveable { mutableStateOf(Tab.Today) }
    var ratingLater by rememberSaveable { mutableStateOf(setOf<String>()) }
    val state = remember(version) { device.state() }
    val serverNow = now.serverTime.takeIf { it != Instant.EPOCH } ?: device.serverTime()
    val today = LocalDate.now(ZoneId.systemDefault())
    val plan = remember(version, today) { DayPlans.dayPlan(state, today) }
    // Day (Workday) or Night (after Shutdown Time / before Workday Start), driven by time.
    var tomorrow by rememberSaveable { mutableStateOf(false) }
    val shownDate = if (tomorrow) today.plusDays(1) else today
    val shownPlan = remember(version, shownDate) { DayPlans.dayPlan(state, shownDate) }
    val night = now.lockdown != null ||
        (now.lockdown == null && state.settings == null && plan != null &&
            (serverNow.isBefore(plan.startsAt) || !serverNow.isBefore(plan.endsAt)))

    CloisterTheme(night = night) {
        val c = LocalColors.current
        Box(Modifier.fillMaxSize().background(c.ground).systemBarsPadding().imePadding()) {
            val signedIn = device.signedIn
            val unrated = remember(version, serverNow.epochSecond / 60) {
                unratedDeepSession(state, serverNow)?.takeIf { it.id !in ratingLater }
            }
            var ritualDone by remember { mutableStateOf(false) }
            val ritualDue = now.enforcement.ritual == dev.cloister.core.RitualScreen.Due
            if (!ritualDue) ritualDone = false
            val screen = when {
                !signedIn -> "sign-in"
                ritualDue && !ritualDone -> "ritual"
                now.session != null -> "active"
                unrated != null && tab == Tab.Today -> "rating"
                else -> tab.name
            }
            Crossfade(screen, animationSpec = tween(240, easing = FastOutSlowInEasing), label = "screen") { s ->
                when (s) {
                    "sign-in" -> SignInScreen(device, configured, actions)
                    "ritual" -> RitualFlowScreen(device, state, serverNow) { ritualDone = true }
                    "active" -> ActiveScreen(now.session!!, now.serverTime, now.vpnRunning && now.appBlockerOn) { tab = Tab.Setup }
                    "rating" -> DepthRatingScreen(device, unrated!!) { ratingLater = ratingLater + unrated.id }
                    else -> Column(Modifier.fillMaxSize()) {
                        TabRow(tab) { tab = it }
                        when (tab) {
                            Tab.Today -> if (now.lockdown != null) {
                                LockdownScreen(now) { tomorrow = true; tab = Tab.DayPlan }
                            } else {
                                BeginScreen(device, now, status.label) { tab = Tab.Setup }
                            }
                            Tab.DayPlan -> DayPlanScreen(
                                shownPlan, saved = state.dayPlans.any { it.date == shownDate }, serverNow,
                                tomorrow = tomorrow, onToggleDay = { tomorrow = !tomorrow },
                            )
                            Tab.Week -> ScoreboardScreen(state, serverNow)
                            Tab.Setup -> SetupScreen(device, now, actions)
                        }
                    }
                }
            }
        }
    }
}

/** The latest ended Deep Session (Block or deep Slot) without a Depth Rating, within a day. */
fun unratedDeepSession(state: dev.cloister.core.State, now: Instant): Block? =
    (state.blocks + DayPlans.plannedDeepSessions(state, now))
        .filter { it.deep && it.depthRating == null && !it.endsAt.isAfter(now) && Duration.between(it.endsAt, now) < Duration.ofHours(24) }
        .maxByOrNull { it.endsAt }

@Composable
private fun TabRow(selected: Tab, onSelect: (Tab) -> Unit) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(20.dp)) {
        for (t in Tab.entries) {
            Column(horizontalAlignment = Alignment.Start) {
                TextLabel(t.label, active = t == selected) { onSelect(t) }
            }
        }
    }
    Box(Modifier.fillMaxWidth().padding(horizontal = 20.dp).height(1.dp).background(c.line))
}

@Composable
private fun TextLabel(text: String, active: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    Column(
        Modifier
            .clickable(role = Role.Tab, onClick = onClick)
            .semantics { selected = active }
            .heightIn(min = 48.dp),
        verticalArrangement = Arrangement.Bottom,
    ) {
        Text(
            text,
            style = Type.detail.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold),
            color = if (active) c.ink else c.ink2,
            modifier = Modifier.padding(top = 12.dp, bottom = 8.dp),
        )
        // The active tab is marked by a rule as well as by ink, never by colour alone.
        Box(Modifier.width(24.dp).height(2.dp).background(if (active) Depth.d4 else Color.Transparent))
    }
}

/** A centred column of at most 560 dp with 28 dp gutters, scrollable. */
@Composable
fun Page(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
        Column(
            Modifier.widthIn(max = 560.dp).fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(horizontal = 28.dp, vertical = 24.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) { content() }
    }
}
