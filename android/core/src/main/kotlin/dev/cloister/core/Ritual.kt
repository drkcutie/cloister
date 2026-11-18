package dev.cloister.core

import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneOffset
import java.time.temporal.IsoFields
import kotlinx.serialization.json.JsonElement

/** An open task or commitment, captured so it stays out of your head overnight. */
data class LooseEnd(val id: String, val text: String, val capturedAt: Instant) {
    fun toJson() = obj("id" to str(id), "text" to str(text), "captured_at" to time(capturedAt))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "loose end").run {
            LooseEnd(string("id"), string("text"), instant("captured_at")).also { done() }
        }
    }
}

/** A completed Shutdown Ritual, by the Workday it closes. */
data class Ritual(val date: LocalDate, val completedAt: Instant) {
    fun toJson() = obj("date" to str(date.toString()), "completed_at" to time(completedAt))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "ritual").run {
            Ritual(date("date"), instant("completed_at")).also { done() }
        }
    }
}

/** A used Shutdown Delay, by the Workday whose shutdown it postponed. */
data class ShutdownDelay(val date: LocalDate, val usedAt: Instant) {
    fun toJson() = obj("date" to str(date.toString()), "used_at" to time(usedAt))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "shutdown delay").run {
            ShutdownDelay(date("date"), instant("used_at")).also { done() }
        }
    }
}

/** The latest ritual due by some time. */
data class RitualStatus(val date: LocalDate, val dueAt: Instant, val lapsesAt: Instant?, val completedAt: Instant?)

/**
 * The Shutdown Ritual, Shutdown Delays, Missed Rituals and Loose Ends (the Kotlin twin of
 * `cloister-core/src/ritual.rs`). Only completions are stored; a Missed Ritual is a Workday whose
 * ritual lapsed at the next Workday Start with none.
 */
object Rituals {
    val SHUTDOWN_DELAY: Duration = Duration.ofMinutes(30)
    const val SHUTDOWN_COMPLETE = "Shutdown complete"
    const val MAX_LOOSE_END = 500
    private const val LOOKBACK_DAYS = 15L

    private fun zone(state: State) = Blocklists.zone(state.timeZone) ?: ZoneOffset.UTC
    private fun localDate(state: State, at: Instant) = at.atZone(zone(state)).toLocalDate()
    private fun isoWeek(d: LocalDate) = d.get(IsoFields.WEEK_BASED_YEAR) to d.get(IsoFields.WEEK_OF_WEEK_BASED_YEAR)
    private fun delayFor(state: State, date: LocalDate) = state.shutdownDelays.firstOrNull { it.date == date }

    /** When the ritual of [date] becomes due, as seen at [now]; null if it isn't a Workday. */
    fun ritualDueAt(state: State, date: LocalDate, now: Instant): Instant? {
        val bounds = WorkdaySettings.workday(state, date, now) as? Workday.Bounds ?: return null
        return if (delayFor(state, date) != null) bounds.end.plus(SHUTDOWN_DELAY) else bounds.end
    }

    /** The latest ritual due at or before [now]. */
    fun ritual(state: State, now: Instant): RitualStatus? {
        val today = localDate(state, now)
        for (n in 0 until LOOKBACK_DAYS) {
            val date = today.minusDays(n)
            val due = ritualDueAt(state, date, now)?.takeIf { !it.isAfter(now) } ?: continue
            return RitualStatus(
                date, due, WorkdaySettings.nextWorkdayStart(state, due),
                state.rituals.firstOrNull { it.date == date }?.completedAt,
            )
        }
        return null
    }

    fun screen(state: State, now: Instant): RitualScreen {
        val today = localDate(state, now)
        val delay = delayFor(state, today)
        val todayDue = ritualDueAt(state, today, now)
        if (delay != null && todayDue != null && !delay.usedAt.isAfter(now) && now.isBefore(todayDue)) {
            return RitualScreen.Delayed
        }
        val status = ritual(state, now) ?: return RitualScreen.None
        val lapsed = status.lapsesAt != null && !status.lapsesAt.isAfter(now)
        return when {
            status.completedAt != null && !lapsed -> RitualScreen.Completed
            status.completedAt != null -> RitualScreen.None
            !lapsed -> RitualScreen.Due
            else -> RitualScreen.Missed
        }
    }

    fun shutdownDelaysLeft(state: State, now: Instant): Int {
        val week = isoWeek(localDate(state, now))
        return if (state.shutdownDelays.any { isoWeek(it.date) == week }) 0 else 1
    }

    /** A Shutdown Delay holds back Evening Lockdown from Shutdown Time until 30 minutes later (local wall time). */
    internal fun delayHoldsLockdown(state: State, t: LocalDateTime, shutdown: LocalTime): Boolean =
        delayFor(state, t.toLocalDate()) != null && t.isBefore(t.toLocalDate().atTime(shutdown).plus(SHUTDOWN_DELAY))

    internal fun captureLooseEnd(state: State, id: String, text: String, now: Instant): State {
        val trimmed = text.trim()
        if (trimmed.isEmpty() || trimmed.codePointCount(0, trimmed.length) > MAX_LOOSE_END || id.trim().isEmpty()) {
            throw Rejection(Reason.InvalidLooseEnd)
        }
        if (state.looseEnds.any { it.id == id }) throw Rejection(Reason.DuplicateLooseEnd)
        return state.copy(
            looseEnds = (state.looseEnds + LooseEnd(id, trimmed, now)).sortedWith(compareBy({ it.capturedAt }, { it.id })),
        )
    }

    internal fun closeLooseEnd(state: State, id: String): State {
        if (state.looseEnds.none { it.id == id }) throw Rejection(Reason.UnknownLooseEnd)
        return state.copy(looseEnds = state.looseEnds.filter { it.id != id })
    }

    internal fun useShutdownDelay(state: State, now: Instant): State {
        WorkdaySettings.settingsAt(state, now) ?: throw Rejection(Reason.NoShutdownTime)
        WorkdaySettings.lockdown(state, now)?.let { throw Rejection(it.reason) }
        if (shutdownDelaysLeft(state, now) == 0) throw Rejection(Reason.ShutdownDelayUsed)
        return state.copy(
            shutdownDelays = (state.shutdownDelays + ShutdownDelay(localDate(state, now), now)).sortedBy { it.date },
        )
    }

    internal fun completeRitual(state: State, typed: String, now: Instant): State {
        val status = ritual(state, now) ?: throw Rejection(Reason.RitualNotDue)
        if (status.completedAt != null) throw Rejection(Reason.RitualAlreadyCompleted)
        if (status.lapsesAt != null && !status.lapsesAt.isAfter(now)) throw Rejection(Reason.RitualNotDue)
        if (state.looseEnds.none { !it.capturedAt.isBefore(status.dueAt) }) throw Rejection(Reason.LooseEndRequired)
        status.lapsesAt?.let { tomorrowIsPlanned(state, localDate(state, it)) }
        if (typed != SHUTDOWN_COMPLETE) throw Rejection(Reason.ConfirmationMismatch)
        return state.copy(rituals = (state.rituals + Ritual(status.date, now)).sortedBy { it.date })
    }

    /** The next Workday has a saved Day Plan with a deep Slot, whose Slots cover it end to end. */
    private fun tomorrowIsPlanned(state: State, date: LocalDate) {
        val plan = state.dayPlans.firstOrNull { it.date == date } ?: throw Rejection(Reason.DayPlanRequired)
        if (plan.slots.none { it.deep }) throw Rejection(Reason.DeepSlotRequired)
        var reached = plan.startsAt
        for (slot in plan.slots) {
            if (slot.startsAt != reached) throw Rejection(Reason.WorkdayNotCovered)
            reached = slot.endsAt
        }
        if (reached != plan.endsAt) throw Rejection(Reason.WorkdayNotCovered)
    }
}
