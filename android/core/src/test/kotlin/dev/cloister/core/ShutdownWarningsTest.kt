package dev.cloister.core

import java.time.Instant
import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Shutdown warnings come 15 and 5 minutes before the next ritual is due. */
class ShutdownWarningsTest {
    private val t = { s: String -> Instant.parse(s) }
    private val d = { s: String -> LocalDate.parse(s) }

    // Monday 2026-01-05 in UTC; Workdays Monday to Friday, 09:00 to 17:00.
    private val state = State(
        settings = Settings(listOf(1, 2, 3, 4, 5), "09:00", "17:00"),
        settingsSince = t("2026-01-01T00:00:00Z"),
    )

    @Test
    fun fifteenAndFiveMinutesBeforeShutdownTime() {
        val now = t("2026-01-05T16:00:00Z")
        assertEquals(
            listOf(t("2026-01-05T16:45:00Z"), t("2026-01-05T16:55:00Z")),
            ShutdownWarnings.forNextRitual(state, now).map { it.at },
        )
        assertEquals(15L, ShutdownWarnings.next(state, now)!!.minutesBefore)
        assertNull(ShutdownWarnings.current(state, now))
    }

    @Test
    fun betweenTheWarningsTheFirstShowsAndTheSecondIsNext() {
        val now = t("2026-01-05T16:50:00Z")
        val current = ShutdownWarnings.current(state, now)!!
        assertEquals(15L, current.minutesBefore)
        assertEquals(10L, ShutdownWarnings.minutesLeft(current, now))
        assertEquals(t("2026-01-05T16:55:00Z"), ShutdownWarnings.next(state, now)!!.at)
        assertEquals(5L, ShutdownWarnings.current(state, t("2026-01-05T16:56:30Z"))!!.minutesBefore)
        assertEquals(4L, ShutdownWarnings.minutesLeft(ShutdownWarnings.current(state, t("2026-01-05T16:56:30Z"))!!, t("2026-01-05T16:56:30Z")))
    }

    @Test
    fun aShutdownDelayMovesTheWarnings() {
        val delayed = state.copy(shutdownDelays = listOf(ShutdownDelay(d("2026-01-05"), t("2026-01-05T16:00:00Z"))))
        assertEquals(
            listOf(t("2026-01-05T17:15:00Z"), t("2026-01-05T17:25:00Z")),
            ShutdownWarnings.forNextRitual(delayed, t("2026-01-05T16:10:00Z")).map { it.at },
        )
    }

    @Test
    fun onceDueTheNextWorkdaysRitualIsNext() {
        val next = ShutdownWarnings.next(state, t("2026-01-05T17:00:00Z"))!!
        assertEquals(d("2026-01-06"), next.date)
        assertEquals(t("2026-01-06T16:45:00Z"), next.at)
        assertNull(ShutdownWarnings.current(state, t("2026-01-05T17:01:00Z")))
    }

    @Test
    fun restDaysAreSkipped() {
        // Friday evening: the next ritual is Monday's.
        val next = ShutdownWarnings.next(state, t("2026-01-09T18:00:00Z"))!!
        assertEquals(d("2026-01-12"), next.date)
    }

    @Test
    fun noSettingsNoWarnings() {
        assertEquals(emptyList<ShutdownWarning>(), ShutdownWarnings.forNextRitual(State(), t("2026-01-05T16:00:00Z")))
    }

    @Test
    fun aCompletedRitualHasNoWarnings() {
        val done = state.copy(rituals = listOf(Ritual(d("2026-01-06"), t("2026-01-06T17:05:00Z"))))
        // Monday after its ritual: Tuesday's is already recorded (a stale cache), so Wednesday's is next.
        assertEquals(d("2026-01-07"), ShutdownWarnings.next(done, t("2026-01-05T18:00:00Z"))!!.date)
    }
}
