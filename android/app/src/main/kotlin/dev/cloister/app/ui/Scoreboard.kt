package dev.cloister.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.cloister.core.Review
import dev.cloister.core.State
import dev.cloister.core.WeekSummary
import java.time.Instant
import java.time.format.DateTimeFormatter
import java.time.format.TextStyle
import java.util.Locale

/** "12 h 30 min of 20 Deep Hours", or just the hours without a target. Never a percentage. */
fun deepHoursLine(minutes: Long, targetHours: Int?): String =
    if (targetHours == null) "${Copy.minutes(minutes)} of Deep Hours" else "${Copy.minutes(minutes)} of $targetHours Deep Hours"

/**
 * The Deep Hours well: a vertical fill toward the weekly target. No percentage, no streak; past
 * the target the well is simply full.
 */
@Composable
fun DeepHoursWell(minutes: Long, targetHours: Int?, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val fraction = if (targetHours == null || targetHours == 0) 0f
    else (minutes.toFloat() / (targetHours * 60f)).coerceIn(0f, 1f)
    Row(
        modifier.clearAndSetSemantics { contentDescription = deepHoursLine(minutes, targetHours) },
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.Bottom,
    ) {
        Box(
            Modifier.width(56.dp).height(168.dp).background(c.sunk).border(1.dp, c.line),
            contentAlignment = Alignment.BottomCenter,
        ) {
            Box(Modifier.fillMaxWidth().fillMaxHeight(fraction).background(Depth.d4))
        }
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(Copy.minutes(minutes), style = Type.figure.copy(fontSize = Type.figure.fontSize * 1.8f), color = c.ink)
            Text(
                targetHours?.let { "of $it Deep Hours this week" } ?: "Deep Hours this week. No target set.",
                style = Type.detail, color = c.ink2,
            )
        }
    }
}

/** Deep minutes per day, Monday first, as bars with the figure under each (never colour alone). */
@Composable
fun DayBars(summary: WeekSummary) {
    val c = LocalColors.current
    val max = (summary.deepMinutesByDay.maxOrNull() ?: 0L).coerceAtLeast(60L)
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.Bottom) {
        summary.deepMinutesByDay.forEachIndexed { i, m ->
            val day = summary.weekStart.plusDays(i.toLong())
            val name = day.dayOfWeek.getDisplayName(TextStyle.SHORT, Locale.getDefault())
            Column(
                Modifier.weight(1f).clearAndSetSemantics {
                    contentDescription = "${day.dayOfWeek.getDisplayName(TextStyle.FULL, Locale.getDefault())}: ${Copy.minutes(m)}"
                },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Box(Modifier.fillMaxWidth().height(96.dp), contentAlignment = Alignment.BottomCenter) {
                    Box(
                        Modifier.fillMaxWidth().fillMaxHeight(m.toFloat() / max)
                            .background(if (m > 0) Depth.d3 else Color.Transparent),
                    )
                }
                Box(Modifier.fillMaxWidth().height(1.dp).background(c.line))
                Text(name, style = Type.label, color = c.ink2)
                Text(if (m == 0L) "–" else Copy.minutes(m).replace(" min", "m").replace(" h", "h"), style = Type.label, color = c.ink)
            }
        }
    }
}

/** The depth trend: recent Depth Ratings as squares on the depth ramp, each with its number. */
@Composable
fun DepthTrend(ratings: List<Int>) {
    val c = LocalColors.current
    if (ratings.isEmpty()) {
        Text("No Depth Ratings yet.", style = Type.detail, color = c.ink2)
        return
    }
    Row(
        Modifier.clearAndSetSemantics { contentDescription = "Recent Depth Ratings, oldest first: ${ratings.joinToString(", ")}" },
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.Bottom,
    ) {
        for (r in ratings) {
            Box(
                Modifier.width(24.dp).height((12 + r * 8).dp).background(Depth.all[(r - 1).coerceIn(0, 4)]),
                contentAlignment = Alignment.BottomCenter,
            ) {
                Text("$r", style = Type.label, color = if (r >= 3) Color.White else Depth.d5)
            }
        }
    }
}

/** The week's records: Missed Rituals, Shutdown Delays used and Breaches, in brass. No penalty. */
@Composable
fun WeekRecords(summary: WeekSummary) {
    val c = LocalColors.current
    val date = DateTimeFormatter.ofPattern("EEE d MMM", Locale.getDefault())
    if (summary.missedRituals.isEmpty() && summary.shutdownDelays.isEmpty() && summary.breaches.isEmpty()) {
        Notice("No Missed Ritual, Shutdown Delay or Breach this week.", recorded = true)
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (d in summary.missedRituals) RecordRow("Missed Ritual", date.format(d))
        for (d in summary.shutdownDelays) RecordRow("Shutdown Delay used", date.format(d))
        for (b in summary.breaches) {
            val what = if (b.kind == dev.cloister.core.BreachKind.VpnTurnedOff) "VPN turned off" else "App blocker turned off"
            RecordRow("Breach", "$what, ${date.format(b.occurredAt.atZone(java.time.ZoneId.systemDefault()).toLocalDate())} ${Copy.clock(b.occurredAt)}")
        }
    }
    Text("Recorded as they happened. Nothing else follows.", style = Type.label, color = c.ink2)
}

@Composable
private fun RecordRow(tag: String, detail: String) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().background(c.brassTint).padding(horizontal = 12.dp, vertical = 10.dp)
            .semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Tag(tag, c.brassTint, c.brass)
        Text(detail, style = Type.detail, color = c.ink)
    }
}

/** The scoreboard screen: this week's Deep Hours well, per-day bars, depth trend and records. */
@Composable
fun ScoreboardScreen(state: State, now: Instant) {
    val c = LocalColors.current
    val summary = remember(state, now.epochSecond / 60) { Review.weekSummary(state, now) }
    Page {
        Eyebrow("Deep Hours")
        Text(
            "Week of ${DateTimeFormatter.ofPattern("d MMMM", Locale.getDefault()).format(summary.weekStart)}",
            style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() },
        )
        DeepHoursWell(summary.deepMinutes, summary.targetHours)
        if (summary.nextWeekTargetHours != summary.targetHours && summary.nextWeekTargetHours != null) {
            Notice("Next week's target: ${summary.nextWeekTargetHours} Deep Hours.")
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Eyebrow("By day", c.ink2)
            DayBars(summary)
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Eyebrow("Depth trend", c.ink2)
            DepthTrend(summary.depthTrend)
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Eyebrow("Recorded", c.ink2)
            WeekRecords(summary)
        }
    }
}
