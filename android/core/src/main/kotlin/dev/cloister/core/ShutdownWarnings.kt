package dev.cloister.core

import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset

/** A heads-up [minutesBefore] the Shutdown Ritual of [date] is due at [dueAt]. */
data class ShutdownWarning(val date: LocalDate, val minutesBefore: Long, val at: Instant, val dueAt: Instant)

/**
 * Shutdown warnings: 15 and 5 minutes before the next Shutdown Ritual is due (Shutdown Time, or 30
 * minutes later after a Shutdown Delay). Pure: a Device schedules [next] and shows [current].
 */
object ShutdownWarnings {
    /** Minutes before the ritual is due, latest warning last. */
    val LEADS = listOf(15L, 5L)
    private const val LOOKAHEAD_DAYS = 15L

    /** The warnings of the next ritual still ahead of [now] (today's, else the next Workday's), in time order. */
    fun forNextRitual(state: State, now: Instant): List<ShutdownWarning> {
        val today = now.atZone(Blocklists.zone(state.timeZone) ?: ZoneOffset.UTC).toLocalDate()
        for (n in 0 until LOOKAHEAD_DAYS) {
            val date = today.plusDays(n)
            val due = Rituals.ritualDueAt(state, date, now)?.takeIf { it.isAfter(now) } ?: continue
            if (state.rituals.any { it.date == date }) continue
            return LEADS.map { m -> ShutdownWarning(date, m, due.minus(Duration.ofMinutes(m)), due) }
        }
        return emptyList()
    }

    /** The next warning to schedule: the first whose time is after [now]. */
    fun next(state: State, now: Instant): ShutdownWarning? = forNextRitual(state, now).firstOrNull { it.at.isAfter(now) }

    /** The warning to show at [now]: the latest whose time has come, with its ritual still ahead. */
    fun current(state: State, now: Instant): ShutdownWarning? = forNextRitual(state, now).lastOrNull { !it.at.isAfter(now) }

    /** Whole minutes until [warning]'s ritual is due, rounded up (at least 1). */
    fun minutesLeft(warning: ShutdownWarning, now: Instant): Long {
        val seconds = Duration.between(now, warning.dueAt).seconds
        return ((seconds + 59) / 60).coerceAtLeast(1)
    }
}
