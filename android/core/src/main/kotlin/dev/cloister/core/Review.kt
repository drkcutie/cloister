package dev.cloister.core

import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneOffset
import java.time.ZonedDateTime
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive

/** The weekly Deep Hours target in force (as of the last change). */
data class WeeklyTarget(val hours: Int) {
    fun toJson() = obj("hours" to JsonPrimitive(hours))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "weekly target").run {
            WeeklyTarget(long("hours").toInt()).also { done() }
        }
    }
}

/** A lower target, in force from [effectiveAt] (the next week boundary when it was set). */
data class PendingWeeklyTarget(val hours: Int, val effectiveAt: Instant) {
    fun toJson() = obj("hours" to JsonPrimitive(hours), "effective_at" to time(effectiveAt))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "pending weekly target").run {
            PendingWeeklyTarget(long("hours").toInt(), instant("effective_at")).also { done() }
        }
    }
}

/** The scoreboard and Weekly Review for one ISO week. Minutes, not percentages; no streaks. */
data class WeekSummary(
    /** The Monday the week starts on (local). */
    val weekStart: LocalDate,
    val targetHours: Int?,
    val nextWeekTargetHours: Int?,
    /** Deep minutes per local day, Monday first (7 entries); only time already worked counts. */
    val deepMinutesByDay: List<Long>,
    val deepMinutes: Long,
    /** Recent Depth Ratings, oldest first, at most [Review.DEPTH_TREND_LENGTH]. */
    val depthTrend: List<Int>,
    val missedRituals: List<LocalDate>,
    val shutdownDelays: List<LocalDate>,
    val breaches: List<Breach>,
) {
    fun toJson() = kotlinx.serialization.json.JsonObject(
        linkedMapOf(
            "week_start" to str(weekStart.toString()),
            "target_hours" to (targetHours?.let(::JsonPrimitive) ?: JsonNull),
            "next_week_target_hours" to (nextWeekTargetHours?.let(::JsonPrimitive) ?: JsonNull),
            "deep_minutes_by_day" to arr(deepMinutesByDay) { JsonPrimitive(it) },
            "deep_minutes" to JsonPrimitive(deepMinutes),
            "depth_trend" to arr(depthTrend) { JsonPrimitive(it) },
            "missed_rituals" to arr(missedRituals) { str(it.toString()) },
            "shutdown_delays" to arr(shutdownDelays) { str(it.toString()) },
            "breaches" to arr(breaches) { it.toJson() },
        ),
    )
}

/**
 * The weekly Deep Hours target, the Weekly Review and the scoreboard (the Kotlin twin of
 * `cloister-core/src/review.rs`). Raising the target applies now; lowering waits for the next
 * local Monday midnight.
 */
object Review {
    val WEEKLY_TARGET_HOURS = 1..100
    const val DEPTH_TREND_LENGTH = 10

    private fun zone(state: State) = Blocklists.zone(state.timeZone) ?: ZoneOffset.UTC
    private fun localDate(state: State, at: Instant) = at.atZone(zone(state)).toLocalDate()

    /** The start of local [date]; a skipped midnight maps to the first instant after it. */
    private fun dayStart(state: State, date: LocalDate): Instant =
        ZonedDateTime.ofLocal(date.atTime(LocalTime.MIDNIGHT), zone(state), null).withEarlierOffsetAtOverlap().toInstant()

    fun weekStart(date: LocalDate): LocalDate = date.minusDays((date.dayOfWeek.value - 1).toLong())

    /** The first local Monday midnight after [now]. */
    fun nextWeekBoundary(state: State, now: Instant): Instant =
        dayStart(state, weekStart(localDate(state, now)).plusDays(7))

    /** The target in force at [at]: the pending one once its week has come. */
    fun weeklyTargetAt(state: State, at: Instant): Int? {
        val p = state.pendingWeeklyTarget
        return if (p != null && !p.effectiveAt.isAfter(at)) p.hours else state.weeklyTarget?.hours
    }

    internal fun setWeeklyTarget(state: State, hours: Int, now: Instant): State {
        if (hours !in WEEKLY_TARGET_HOURS) throw Rejection(Reason.InvalidWeeklyTarget)
        val inForce = weeklyTargetAt(state, now)?.let { maxOf(it, hours) } ?: hours
        return state.copy(
            weeklyTarget = WeeklyTarget(inForce),
            pendingWeeklyTarget = if (inForce != hours) PendingWeeklyTarget(hours, nextWeekBoundary(state, now)) else null,
        )
    }

    /** Whether [date] is the last Workday of its ISO week, as seen at [now]: its ritual holds the Weekly Review. */
    fun isLastWorkdayOfWeek(state: State, date: LocalDate, now: Instant): Boolean {
        val sunday = weekStart(date).plusDays(6)
        return generateSequence(date.plusDays(1)) { it.plusDays(1) }.takeWhile { !it.isAfter(sunday) }
            .none { WorkdaySettings.workday(state, it, now) is Workday.Bounds }
    }

    internal fun checkWeeklyReview(state: State, date: LocalDate, weeklyTarget: Int?, now: Instant) {
        val last = isLastWorkdayOfWeek(state, date, now)
        when {
            last && weeklyTarget == null -> throw Rejection(Reason.WeeklyReviewRequired)
            !last && weeklyTarget != null -> throw Rejection(Reason.WeeklyReviewNotDue)
            weeklyTarget != null && weeklyTarget !in WEEKLY_TARGET_HOURS -> throw Rejection(Reason.InvalidWeeklyTarget)
        }
    }

    private data class DeepSession(val id: String, val start: Instant, val end: Instant, val rating: Int?)

    /** Deep Blocks, and deep Slots of saved or Schedule-filled Day Plans around [from, to). */
    private fun deepSessions(state: State, from: Instant, to: Instant): List<DeepSession> {
        val found = state.blocks.filter { it.deep }.map { DeepSession(it.id, it.startsAt, it.endsAt, it.depthRating) }
            .toMutableList()
        var date = localDate(state, from).minusDays(1)
        val last = localDate(state, to)
        while (!date.isAfter(last)) {
            DayPlans.dayPlan(state, date)?.slots?.filter { it.deep }
                ?.forEach { found += DeepSession(it.id, it.startsAt, it.endsAt, it.depthRating) }
            date = date.plusDays(1)
        }
        return found.sortedWith(compareBy({ it.start }, { it.id })).distinctBy { it.id }
    }

    /** The scoreboard for the ISO week (local) holding [now], counting time up to [now]. */
    fun weekSummary(state: State, now: Instant): WeekSummary {
        val monday = weekStart(localDate(state, now))
        val days = (0L until 7L).map { monday.plusDays(it) }
        val bounds = (0L..7L).map { dayStart(state, monday.plusDays(it)) }
        val deepSessionsInWeek = deepSessions(state, bounds[0], bounds[7])
        val byDay = (0 until 7).map { i ->
            val to = minOf(bounds[i + 1], now)
            deepSessionsInWeek.sumOf { s ->
                Duration.between(maxOf(s.start, bounds[i]), minOf(s.end, to)).toMinutes().coerceAtLeast(0)
            }
        }
        val since = now.minus(Duration.ofDays(30))
        val trend = deepSessions(state, since, now)
            .filter { !it.start.isBefore(since) && !it.start.isAfter(now) }.mapNotNull { it.rating }
            .takeLast(DEPTH_TREND_LENGTH)
        return WeekSummary(
            weekStart = monday,
            targetHours = weeklyTargetAt(state, now),
            nextWeekTargetHours = weeklyTargetAt(state, bounds[7]),
            deepMinutesByDay = byDay,
            deepMinutes = byDay.sum(),
            depthTrend = trend,
            missedRituals = days.filter { isMissed(state, it, now) },
            shutdownDelays = state.shutdownDelays.filter { it.date in days }.map { it.date },
            breaches = state.breaches.filter { !it.occurredAt.isBefore(bounds[0]) && it.occurredAt.isBefore(bounds[7]) },
        )
    }

    /** Whether the ritual of [date] lapsed, by [now], with no completion. */
    private fun isMissed(state: State, date: LocalDate, now: Instant): Boolean {
        val due = Rituals.ritualDueAt(state, date, now) ?: return false
        if (due.isAfter(now) || state.rituals.any { it.date == date }) return false
        return Rituals.lapsesAt(state, due)?.let { !it.isAfter(now) } ?: false
    }
}
