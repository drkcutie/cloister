package dev.cloister.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.cloister.app.CloisterApp
import dev.cloister.core.Block
import java.time.Duration
import java.time.Instant
import kotlinx.coroutines.delay

/** Trusted now (server-anchored; the phone's clock can't move it), ticking once a second (no animation). */
@Composable
fun rememberNow(): Instant {
    val device = CloisterApp.instance.device
    var now by remember { mutableStateOf(device.serverTime()) }
    LaunchedEffect(Unit) {
        while (true) {
            now = device.serverTime()
            delay(1000 - (now.toEpochMilli() % 1000))
        }
    }
    return now
}

/**
 * Shown over a blocked app: calm, a reminder rather than a scolding. The Intent and the time
 * left, and one way out (home).
 */
@Composable
fun BlockedAppOverlay(block: Block?, lockdown: dev.cloister.core.Lockdown? = null, nextWorkdayStart: Instant? = null, onHome: () -> Unit) {
    val c = Day
    val now = rememberNow()
    CloisterTheme {
        Column(
            Modifier.fillMaxSize().background(c.ground).systemBarsPadding().padding(horizontal = 28.dp, vertical = 40.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Masthead(
                when {
                    block?.deep == true -> "Deep Session"
                    block != null -> "Block"
                    lockdown == dev.cloister.core.Lockdown.RestDay -> "Rest Day"
                    else -> "Evening Lockdown"
                },
            )
            Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(24.dp)) {
                Text("This app is blocked.", style = Type.heading, color = c.ink2, modifier = Modifier.semantics { heading() })
                block?.intent?.let { Text(it, style = Type.intent, color = c.ink) }
                if (block == null && nextWorkdayStart != null) {
                    Text("Work starts again at ${Copy.clock(nextWorkdayStart)}.", style = Type.body, color = c.ink2)
                }
                block?.let {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(Copy.countdown(Duration.between(now, it.endsAt)), style = Type.timer.copy(fontSize = Type.timer.fontSize * 0.78f), color = c.ink)
                        Text("Ends at ${Copy.clock(it.endsAt)}", style = Type.detail, color = c.ink2)
                    }
                }
            }
            CommitButton("Go to the home screen", onClick = onHome)
        }
    }
}

/**
 * The lock overlay: the VPN was turned off during a Block. The Breach is recorded; the way
 * back is to turn the VPN on again. Emergency calls stay reachable.
 */
@Composable
fun LockOverlay(block: Block?, onTurnVpnOn: () -> Unit, onEmergencyCall: () -> Unit) {
    val c = Night
    val now = rememberNow()
    CloisterTheme(night = true) {
        Column(
            Modifier.fillMaxSize().background(c.ground).systemBarsPadding().padding(horizontal = 28.dp, vertical = 40.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Masthead("Block")
            Column(verticalArrangement = Arrangement.spacedBy(24.dp)) {
                Text("The VPN is off.", style = Type.display, color = c.ink, modifier = Modifier.semantics { heading() })
                Text(
                    "A Breach was recorded. Blocked sites and apps stay blocked until the Block ends.",
                    style = Type.body, color = c.ink2,
                )
                block?.let {
                    Text(Copy.countdown(Duration.between(now, it.endsAt)), style = Type.timer.copy(fontSize = Type.timer.fontSize * 0.78f), color = c.ink)
                    Text("Ends at ${Copy.clock(it.endsAt)}", style = Type.detail, color = c.ink2)
                }
            }
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                CommitButton("Turn the VPN back on", onClick = onTurnVpnOn)
                QuietButton("Emergency call", Modifier.fillMaxWidth(), onClick = onEmergencyCall)
                Spacer(Modifier)
            }
        }
    }
}

/**
 * The ritual lock overlay: the Shutdown Ritual is due, so the phone isn't the way out. One way
 * forward (the ritual, in Cloister) and emergency calls, which always stay open.
 */
@Composable
fun RitualOverlay(lockdown: dev.cloister.core.Lockdown?, nextWorkdayStart: Instant?, onOpenRitual: () -> Unit, onEmergencyCall: () -> Unit) {
    val c = Night
    CloisterTheme(night = true) {
        Column(
            Modifier.fillMaxSize().background(c.ground).systemBarsPadding().padding(horizontal = 28.dp, vertical = 40.dp),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Masthead("Shutdown Ritual")
            Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(24.dp)) {
                Text("The workday has ended.", style = Type.display, color = c.ink, modifier = Modifier.semantics { heading() })
                Text(
                    "The Shutdown Ritual is due. Capture your Loose Ends and plan tomorrow. Completing it here unlocks every Device.",
                    style = Type.body, color = c.ink2,
                )
                if (lockdown == dev.cloister.core.Lockdown.EveningLockdown && nextWorkdayStart != null) {
                    Text("Workday Start ${Copy.clock(nextWorkdayStart)}.", style = Type.detail, color = c.ink2)
                }
            }
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                CommitButton("Open the Shutdown Ritual", onClick = onOpenRitual)
                QuietButton("Emergency call", Modifier.fillMaxWidth(), onClick = onEmergencyCall)
                Spacer(Modifier)
            }
        }
    }
}
