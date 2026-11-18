package dev.cloister.core

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** Trusted now runs on the boot clock from the last server time; the wall clock only adds enforcement. */
class TrustedClockTest {
    private val t = { s: String -> Instant.parse(s) }

    private class FakeClock(var elapsed: Long, var boot: Int?, var wall: Instant) : DeviceClock {
        override fun elapsedMillis() = elapsed
        override fun bootCount() = boot
        override fun wallNow() = wall
    }

    private val minute = 60_000L

    // Monday 2026-01-05, UTC. Workday 09:00 to 17:00.
    private val settings = Settings(listOf(1, 2, 3, 4, 5), "09:00", "17:00")

    @Test
    fun runsOnTheBootClockFromTheServerTime() {
        val clock = FakeClock(10 * minute, 1, t("2026-01-05T10:00:00Z"))
        val anchor = TrustedClock.anchor(t("2026-01-05T10:00:00Z"), clock)
        clock.elapsed += 5 * minute
        assertEquals(t("2026-01-05T10:05:00Z"), TrustedClock.now(anchor, clock))
    }

    @Test
    fun settingTheWallClockForwardOrBackMovesNothing() {
        val clock = FakeClock(10 * minute, 1, t("2026-01-05T10:00:00Z"))
        val anchor = TrustedClock.anchor(t("2026-01-05T10:00:00Z"), clock)
        clock.elapsed += minute
        clock.wall = t("2026-01-06T10:00:00Z")
        assertEquals(t("2026-01-05T10:01:00Z"), TrustedClock.now(anchor, clock))
        clock.wall = t("2025-12-01T10:00:00Z")
        assertEquals(t("2026-01-05T10:01:00Z"), TrustedClock.now(anchor, clock))
    }

    @Test
    fun aRebootCountsAtLeastTheUptimeSinceIt() {
        val anchor = ClockAnchor(t("2026-01-05T10:00:00Z"), 120 * minute, boot = 1)
        // Booted again 3 minutes ago: the boot clock reads less than at the anchor.
        assertEquals(t("2026-01-05T10:03:00Z"), TrustedClock.now(anchor, 3 * minute, bootNow = 2))
        assertEquals(t("2026-01-05T10:03:00Z"), TrustedClock.now(anchor, 3 * minute, bootNow = null))
        // Up longer than before the reboot: the boot counter tells.
        assertEquals(t("2026-01-05T13:00:00Z"), TrustedClock.now(anchor, 180 * minute, bootNow = 2))
    }

    @Test
    fun checkpointsAtMostEvery30SecondsAndAtOnceAfterAReboot() {
        val anchor = ClockAnchor(t("2026-01-05T10:00:00Z"), 10 * minute, boot = 1)
        assertSame(anchor, TrustedClock.checkpoint(anchor, 10 * minute + 29_000, 1))
        assertEquals(
            ClockAnchor(t("2026-01-05T10:00:30Z"), 10 * minute + 30_000, 1),
            TrustedClock.checkpoint(anchor, 10 * minute + 30_000, 1),
        )
        val afterReboot = TrustedClock.checkpoint(anchor, 2_000, 2)
        assertEquals(ClockAnchor(t("2026-01-05T10:00:02Z"), 2_000, 2), afterReboot)
        // Later readings run on from the new anchor, never back.
        assertEquals(t("2026-01-05T10:20:02Z"), TrustedClock.now(afterReboot, 2_000 + 20 * minute, 2))
    }

    @Test
    fun aWallClockSetForwardCantEndABlockEarly() {
        val block = Block("b", t("2026-01-05T10:00:00Z"), t("2026-01-05T11:00:00Z"), listOf("social"), deep = false)
        val state = State(blocks = listOf(block))
        val e = TrustedClock.enforcement(state, trusted = t("2026-01-05T10:30:00Z"), wall = t("2026-01-05T12:00:00Z"))
        assertTrue(e.blockedDomains.isNotEmpty())
        assertEquals(RulesEngine.enforcement(state, t("2026-01-05T10:30:00Z")).blockedDomains, e.blockedDomains)
    }

    @Test
    fun aWallClockSetBackCantLiftEveningLockdown() {
        val state = State(settings = settings)
        val trusted = t("2026-01-05T18:00:00Z")
        val e = TrustedClock.enforcement(state, trusted, wall = t("2026-01-05T12:00:00Z"))
        assertEquals(RulesEngine.enforcement(state, trusted), e)
        assertTrue(e.blockedDomains.isNotEmpty())
        assertEquals(Lockdown.EveningLockdown, TrustedClock.lockdown(state, trusted, t("2026-01-05T12:00:00Z")))
    }

    @Test
    fun aPhoneOffForHoursStillEntersEveningLockdownByTheWallClock() {
        val state = State(settings = settings)
        // Trusted now lags (the reboot lost hours); the wall clock says it's evening.
        val trusted = t("2026-01-05T12:00:00Z")
        val wall = t("2026-01-05T18:00:00Z")
        assertTrue(RulesEngine.enforcement(state, trusted).isEmpty)
        val e = TrustedClock.enforcement(state, trusted, wall)
        assertEquals(RulesEngine.enforcement(state, wall).blockedDomains, e.blockedDomains)
        assertEquals(Lockdown.EveningLockdown, TrustedClock.lockdown(state, trusted, wall))
    }

    @Test
    fun theUnionKeepsTheBlockAndAddsTheLockdown() {
        val block = Block("b", t("2026-01-05T16:00:00Z"), t("2026-01-05T16:40:00Z"), emptyList(), deep = false, extraDomains = listOf("example.org"))
        val state = State(blocks = listOf(block), settings = settings)
        val e = TrustedClock.enforcement(state, trusted = t("2026-01-05T16:30:00Z"), wall = t("2026-01-05T18:00:00Z"))
        assertTrue("example.org" in e.blockedDomains)
        assertTrue(RulesEngine.enforcement(state, t("2026-01-05T18:00:00Z")).blockedDomains.all { it in e.blockedDomains })
    }
}
