package dev.cloister.sync

import dev.cloister.core.DeviceClock
import java.io.File
import java.nio.file.Files
import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Test

/** The mirror's trusted clock: anchored at server time, run on by the boot clock, kept across restarts. */
class MirrorClockTest {
    private class FakeClock(var elapsed: Long, var boot: Int?, var wall: Instant) : DeviceClock {
        override fun elapsedMillis() = elapsed
        override fun bootCount() = boot
        override fun wallNow() = wall
    }

    private val file = File(Files.createTempDirectory("cloister-mirror").toFile(), "mirror.json")
    private val t = { s: String -> Instant.parse(s) }

    @Test
    fun theWallClockNeverMovesTrustedNow() {
        val clock = FakeClock(1_000, 1, t("2026-01-05T10:00:00Z"))
        val mirror = Mirror(file, clock)
        mirror.anchorServerTime(t("2026-01-05T12:00:00Z"))
        clock.elapsed += 60_000
        clock.wall = t("2030-01-01T00:00:00Z")
        assertEquals(t("2026-01-05T12:01:00Z"), mirror.serverTime())
        clock.wall = t("2020-01-01T00:00:00Z")
        assertEquals(t("2026-01-05T12:01:00Z"), mirror.serverTime())
    }

    @Test
    fun theAnchorSurvivesARestartAndARebootLosesAtMost30Seconds() {
        val clock = FakeClock(1_000, 1, t("2026-01-05T10:00:00Z"))
        Mirror(file, clock).apply {
            anchorServerTime(t("2026-01-05T12:00:00Z"))
            clock.elapsed += 45_000
            serverTime() // a tick: checkpoints at 12:00:45
            clock.elapsed += 20_000 // 12:01:05, not yet checkpointed
        }
        // Rebooted 10 s ago.
        clock.elapsed = 10_000
        clock.boot = 2
        val restarted = Mirror(file, clock)
        assertEquals(t("2026-01-05T12:00:55Z"), restarted.serverTime())
        clock.elapsed += 60_000
        assertEquals(t("2026-01-05T12:01:55Z"), restarted.serverTime())
    }
}
