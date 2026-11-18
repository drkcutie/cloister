package dev.cloister.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.unit.dp
import dev.cloister.core.Command
import dev.cloister.core.DayPlan
import dev.cloister.core.DayPlans
import dev.cloister.core.Outcome
import dev.cloister.core.Review
import dev.cloister.core.Rituals
import dev.cloister.core.RulesEngine
import dev.cloister.core.SlotInput
import dev.cloister.core.State
import dev.cloister.core.Workday
import dev.cloister.core.WorkdaySettings
import dev.cloister.sync.CommandOutcome
import dev.cloister.sync.Device
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.UUID
import kotlinx.coroutines.launch

/** The steps of the Shutdown Ritual on the phone. The Weekly Review joins on the last Workday of the week. */
enum class RitualStep(val title: String) {
    LooseEnds("Loose Ends"),
    Review("Review today"),
    Draft("Tomorrow's draft"),
    Plan("Plan tomorrow"),
    WeeklyReview("Weekly Review"),
    Commit("Shutdown complete"),
}

/** Which steps a refusal of `complete_ritual` holds (the same gating as the desktop ritual). */
private fun holds(step: RitualStep, code: String) = when (step) {
    RitualStep.LooseEnds -> code == "loose_end_required"
    RitualStep.Plan -> code in setOf("day_plan_required", "deep_slot_required", "workday_not_covered")
    RitualStep.WeeklyReview -> code in setOf("weekly_review_required", "invalid_weekly_target")
    else -> false
}

private fun zone(state: State) = runCatching { ZoneId.of(state.timeZone) }.getOrDefault(ZoneOffset.UTC)

/** The date of the ritual in question (the latest due), or today. */
private fun ritualDate(state: State, now: Instant): LocalDate =
    Rituals.ritual(state, now)?.date ?: now.atZone(zone(state)).toLocalDate()

/** The Workday the ritual plans: the day of the next Workday Start. */
private fun tomorrow(state: State, now: Instant): LocalDate? =
    (Rituals.ritual(state, now)?.lapsesAt ?: WorkdaySettings.nextWorkdayStart(state, now))
        ?.atZone(zone(state))?.toLocalDate()

/** One Slot in the compact editor: contiguous Slots always cover the Workday end to end. */
private data class EditSlot(val id: String, val start: Instant, val end: Instant, val label: String, val deep: Boolean)

private const val STEP_MINUTES = 15L

/** The draft as contiguous Slots from Workday Start to Shutdown Time (gaps become "Open" Slots). */
private fun editable(draft: DayPlan?, start: Instant, end: Instant): List<EditSlot> {
    val out = mutableListOf<EditSlot>()
    var at = start
    for (s in draft?.slots.orEmpty().sortedBy { it.startsAt }) {
        val a = maxOf(s.startsAt, start)
        val b = minOf(s.endsAt, end)
        if (!b.isAfter(a)) continue
        if (a.isAfter(at)) out += EditSlot("phone-${UUID.randomUUID()}", at, a, "Open", false)
        out += EditSlot(s.id, maxOf(a, at), b, s.label, s.deep)
        at = b
    }
    if (end.isAfter(at)) {
        out += EditSlot("phone-${UUID.randomUUID()}", at, end, if (out.isEmpty()) "Deep work" else "Open", out.isEmpty())
    }
    return out
}

@Composable
fun RitualFlowScreen(device: Device, state: State, now: Instant, onDone: () -> Unit) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    val date = ritualDate(state, now)
    val weekly = remember(state, date) { Review.isLastWorkdayOfWeek(state, date, now) }
    val steps = RitualStep.entries.filter { weekly || it != RitualStep.WeeklyReview }
    var index by rememberSaveable { mutableStateOf(0) }
    val step = steps[index.coerceIn(0, steps.lastIndex)]
    var notice by remember { mutableStateOf<Pair<String, Boolean>?>(null) }
    val summary = remember(state, now.epochSecond / 60) { Review.weekSummary(state, now) }
    var target by rememberSaveable { mutableStateOf(summary.nextWeekTargetHours ?: summary.targetHours ?: 20) }
    var typed by rememberSaveable { mutableStateOf("") }
    var sending by remember { mutableStateOf(false) }

    fun holding(): String? {
        val cmd = Command.CompleteRitual(Rituals.SHUTDOWN_COMPLETE, target.takeIf { weekly })
        return (RulesEngine.decide(state, cmd, now).outcome as? Outcome.Rejected)?.reason?.code
    }

    fun send(cmd: Command, after: (CommandOutcome) -> Unit = {}) {
        sending = true
        scope.launch {
            notice = runCatching { device.send(cmd) }.fold(
                { outcome -> after(outcome); (outcome as? CommandOutcome.Rejected)?.let { Copy.reason(it.reason) to false } },
                { "Offline. The ritual stays until the phone reaches the backend." to false },
            )
            sending = false
        }
    }

    Page {
        Masthead("Shutdown Ritual")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Bottom) {
            Text("%02d".format(index + 1), style = Type.figure, color = c.brass)
            Text(step.title, style = Type.title, color = c.ink, modifier = Modifier.semantics { heading() })
        }
        Text("Step ${index + 1} of ${steps.size}", style = Type.label, color = c.ink2)

        when (step) {
            RitualStep.LooseEnds -> LooseEndsStep(state, now, sending) { send(it) }
            RitualStep.Review -> ReviewStep(state, now, date, summary)
            RitualStep.Draft -> DraftStep(state, now)
            RitualStep.Plan -> PlanStep(state, now, sending) { send(it) }
            RitualStep.WeeklyReview -> WeeklyReviewStep(summary, target) { target = it }
            RitualStep.Commit -> {
                Text("Type \"${Rituals.SHUTDOWN_COMPLETE}\" to end the workday.", style = Type.body, color = c.ink2)
                LineField(typed, { typed = it }, "Shutdown complete")
                HoldToCommit("Hold to complete the Shutdown", enabled = !sending && typed == Rituals.SHUTDOWN_COMPLETE) {
                    send(Command.CompleteRitual(typed, target.takeIf { weekly })) { outcome ->
                        if (outcome is CommandOutcome.Accepted) onDone()
                    }
                }
            }
        }

        notice?.let { (text, recorded) -> Notice(text, recorded) }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (index > 0) QuietButton("Back", Modifier.weight(1f)) { notice = null; index-- }
            if (step != RitualStep.Commit) {
                CommitButton("Next", Modifier.weight(1f)) {
                    val code = holding()
                    if (code != null && holds(step, code)) {
                        notice = Copy.reason(code) to false
                    } else {
                        notice = null
                        index++
                    }
                }
            }
        }
        ShutdownDelayAction(device, state, now)
    }
}

/** "Delay 30 min (1 left this week)": usable until 30 minutes after Shutdown Time (follow-up 9). */
@Composable
fun ShutdownDelayAction(device: Device, state: State, now: Instant) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var notice by remember { mutableStateOf<Pair<String, Boolean>?>(null) }
    if (WorkdaySettings.settingsAt(state, now) == null) return
    val left = Rituals.shutdownDelaysLeft(state, now)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (left == 0) {
            Text("No Shutdown Delay left this week.", style = Type.detail, color = c.ink2)
        } else {
            TextAction("Delay 30 min ($left left this week)") {
                scope.launch {
                    notice = runCatching { device.send(Command.UseShutdownDelay) }.fold(
                        { outcome ->
                            when (outcome) {
                                CommandOutcome.Accepted -> {
                                    val today = now.atZone(zone(state)).toLocalDate()
                                    val until = Rituals.ritualDueAt(device.state(), today, device.serverTime())
                                    (until?.let { "Shutdown delayed to ${Copy.clock(it)}. Logged." } ?: "Shutdown delayed. Logged.") to true
                                }
                                is CommandOutcome.Rejected -> Copy.reason(outcome.reason) to false
                            }
                        },
                        { "Offline. Shutdown Time stands." to false },
                    )
                }
            }
        }
        notice?.let { (text, recorded) -> Notice(text, recorded) }
    }
}

@Composable
private fun LooseEndsStep(state: State, now: Instant, sending: Boolean, send: (Command) -> Unit) {
    val c = LocalColors.current
    var text by rememberSaveable { mutableStateOf("") }
    Text("Capture at least one Loose End, so nothing open stays in your head overnight.", style = Type.body, color = c.ink2)
    LineField(text, { text = it.take(500) }, "A Loose End")
    QuietButton("Capture", Modifier.fillMaxWidth(), enabled = !sending && text.isNotBlank()) {
        send(Command.CaptureLooseEnd("phone-${UUID.randomUUID()}", text.trim()))
        text = ""
    }
    val due = Rituals.ritual(state, now)?.dueAt
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        for (l in state.looseEnds) {
            Row(
                Modifier.fillMaxWidth().background(c.surface).padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    Text(l.text, style = Type.body, color = c.ink)
                    if (due != null && !l.capturedAt.isBefore(due)) Text("Captured tonight", style = Type.label, color = c.ink2)
                }
                TextAction("Close") { send(Command.CloseLooseEnd(l.id)) }
            }
        }
    }
}

@Composable
private fun ReviewStep(state: State, now: Instant, date: LocalDate, summary: dev.cloister.core.WeekSummary) {
    val c = LocalColors.current
    val z = zone(state)
    val sessions = (DayPlans.dayPlan(state, date)?.slots.orEmpty().filter { it.deep && !it.startsAt.isAfter(now) }
        .map { Triple(it.label, it.startsAt to it.endsAt, it.depthRating) } +
        state.blocks.filter { it.deep && it.startsAt.atZone(z).toLocalDate() == date && !it.startsAt.isAfter(now) }
            .map { Triple(it.intent ?: "Deep Session", it.startsAt to it.endsAt, it.depthRating) })
        .sortedBy { it.second.first }
    if (sessions.isEmpty()) Text("No Deep Session today.", style = Type.body, color = c.ink2)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        for ((intent, span, rating) in sessions) {
            Row(
                Modifier.fillMaxWidth().background(Depth.d1).padding(12.dp).semantics(mergeDescendants = true) {},
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    Text(intent, style = Type.body, color = Depth.d5)
                    Text("${Copy.clock(span.first)} to ${Copy.clock(span.second)}", style = Type.label, color = Depth.d5)
                }
                Text(rating?.let { "Depth $it" } ?: "Not rated", style = Type.label, color = Depth.d5)
            }
        }
    }
    DeepHoursWell(summary.deepMinutes, summary.targetHours)
}

@Composable
private fun DraftStep(state: State, now: Instant) {
    val c = LocalColors.current
    val day = tomorrow(state, now)
    val draft = day?.let { DayPlans.dayPlan(state, it) }
    if (day == null || draft == null) {
        Text("No draft for tomorrow yet. Write it in the next step.", style = Type.body, color = c.ink2)
        return
    }
    val saved = state.dayPlans.any { it.date == day }
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(DateTimeFormatter.ofPattern("EEEE d MMMM", Locale.getDefault()).format(day), style = Type.heading, color = c.ink)
        if (!saved) Tag("From Schedule", c.sunk, c.ink2)
    }
    SlotList(draft.slots.map { EditSlot(it.id, it.startsAt, it.endsAt, it.label, it.deep) })
}

@Composable
private fun SlotList(slots: List<EditSlot>) {
    val c = LocalColors.current
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        for (s in slots) {
            Row(
                Modifier.fillMaxWidth().background(if (s.deep) Depth.d1 else c.surface).padding(12.dp)
                    .semantics(mergeDescendants = true) {},
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("${Copy.clock(s.start)}–${Copy.clock(s.end)}", style = Type.figure, color = if (s.deep) Depth.d5 else c.ink2)
                Text(s.label, style = Type.body, color = if (s.deep) Depth.d5 else c.ink, modifier = Modifier.weight(1f))
                if (s.deep) Tag("Deep")
            }
        }
    }
}

/**
 * The compact Day Plan editor: Slots stay contiguous from Workday Start to Shutdown Time, so the
 * whole Workday is always covered. Split a Slot, move a boundary by 15 minutes, merge with the
 * next, mark deep, rename.
 */
@Composable
private fun PlanStep(state: State, now: Instant, sending: Boolean, send: (Command) -> Unit) {
    val c = LocalColors.current
    val day = tomorrow(state, now)
    val bounds = day?.let { WorkdaySettings.workday(state, it, now) as? Workday.Bounds }
    if (day == null || bounds == null) {
        Text("No Workday follows. Nothing to plan.", style = Type.body, color = c.ink2)
        return
    }
    var slots by remember(day, state.dayPlans) {
        mutableStateOf(editable(DayPlans.dayPlan(state, day), bounds.start, bounds.end))
    }
    var selected by remember { mutableStateOf(0) }
    val saved = state.dayPlans.firstOrNull { it.date == day }
    val deepMinutes = slots.filter { it.deep }.sumOf { Duration.between(it.start, it.end).toMinutes() }
    Text(
        "${Copy.clock(bounds.start)} to ${Copy.clock(bounds.end)}. At least one deep Slot. Planned Deep Minutes: ${Copy.minutes(deepMinutes)}.",
        style = Type.detail, color = c.ink2,
    )
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        slots.forEachIndexed { i, s ->
            val chosen = i == selected
            Row(
                Modifier.fillMaxWidth().background(if (s.deep) Depth.d1 else c.surface)
                    .padding(horizontal = 12.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Choice("${Copy.clock(s.start)}–${Copy.clock(s.end)}", chosen) { selected = i }
                Text(s.label, style = Type.body, color = if (s.deep) Depth.d5 else c.ink, modifier = Modifier.weight(1f))
                if (s.deep) Tag("Deep")
            }
        }
    }
    val s = slots.getOrNull(selected) ?: return
    fun replace(f: (MutableList<EditSlot>) -> Unit) { slots = slots.toMutableList().also(f) }
    val step = Duration.ofMinutes(STEP_MINUTES)
    LineField(s.label, { v -> replace { it[selected] = s.copy(label = v.take(80)) } }, "Label")
    SquareCheck("Deep", s.deep) { v -> replace { it[selected] = s.copy(deep = v) } }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        val next = slots.getOrNull(selected + 1)
        QuietButton("Shorter", Modifier.weight(1f), enabled = next != null && Duration.between(s.start, s.end) > step) {
            replace { it[selected] = s.copy(end = s.end.minus(step)); it[selected + 1] = next!!.copy(start = next.start.minus(step)) }
        }
        QuietButton("Longer", Modifier.weight(1f), enabled = next != null && Duration.between(next.start, next.end) > step) {
            replace { it[selected] = s.copy(end = s.end.plus(step)); it[selected + 1] = next!!.copy(start = next.start.plus(step)) }
        }
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        val half = Duration.between(s.start, s.end).toMinutes() / 2 / STEP_MINUTES * STEP_MINUTES
        QuietButton("Split", Modifier.weight(1f), enabled = half >= STEP_MINUTES) {
            val mid = s.start.plus(Duration.ofMinutes(half))
            replace {
                it[selected] = s.copy(end = mid)
                it.add(selected + 1, EditSlot("phone-${UUID.randomUUID()}", mid, s.end, "Open", false))
            }
        }
        QuietButton("Merge with next", Modifier.weight(1f), enabled = selected < slots.lastIndex) {
            val next = slots[selected + 1]
            replace { it[selected] = s.copy(end = next.end, deep = s.deep || next.deep); it.removeAt(selected + 1) }
        }
    }
    CommitButton("Save tomorrow's Day Plan", enabled = !sending && slots.all { it.label.isNotBlank() }) {
        send(
            Command.WriteDayPlan(
                day, bounds.start, bounds.end,
                slots.map { SlotInput(it.id, it.start, it.end, it.label.trim(), it.deep) },
            ),
        )
    }
    if (saved != null) Text("Saved. Planned Deep Minutes only go up.", style = Type.label, color = c.ink2)
}

@Composable
private fun WeeklyReviewStep(summary: dev.cloister.core.WeekSummary, target: Int, onTarget: (Int) -> Unit) {
    val c = LocalColors.current
    Text("The last Workday of the week. Look back, then set next week's target.", style = Type.body, color = c.ink2)
    DeepHoursWell(summary.deepMinutes, summary.targetHours)
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
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Eyebrow("Next week's target", c.ink2)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            QuietButton("Fewer", enabled = target > Review.WEEKLY_TARGET_HOURS.first) { onTarget(target - 1) }
            Text("$target Deep Hours", style = Type.figure.copy(fontSize = Type.figure.fontSize * 1.4f), color = c.ink)
            QuietButton("More", enabled = target < Review.WEEKLY_TARGET_HOURS.last) { onTarget(target + 1) }
        }
        Text("Raising applies now. Lowering takes effect next week.", style = Type.label, color = c.ink2)
    }
}
