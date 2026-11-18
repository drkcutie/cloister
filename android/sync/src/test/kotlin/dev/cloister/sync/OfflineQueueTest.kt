package dev.cloister.sync

import dev.cloister.core.BreachKind
import dev.cloister.core.Command
import dev.cloister.core.DeviceClock
import dev.cloister.core.Settings
import dev.cloister.core.State
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.time.Instant
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Commands decided offline: queued, replayed on the cached state, sent in order later. */
class OfflineQueueTest {
    private val t = { s: String -> Instant.parse(s) }

    private class FixedClock(var elapsed: Long = 0) : DeviceClock {
        override fun elapsedMillis() = elapsed
        override fun bootCount(): Int? = 1
        override fun wallNow(): Instant = Instant.parse("2026-01-05T10:00:00Z")
    }

    private val file = File(Files.createTempDirectory("cloister-queue").toFile(), "mirror.json")
    private fun mirror(clock: FixedClock = FixedClock()) =
        Mirror(file, clock).apply { anchorServerTime(t("2026-01-05T10:00:00Z")) }

    private fun start(id: String) = Command.StartBlock(blockId = id, minutes = 60, deep = false)

    @Test
    fun onlyTighteningOrRecordingCommandsQueue() {
        assertTrue(OfflineQueue.queueable(start("b")))
        assertTrue(OfflineQueue.queueable(Command.CompleteRitual("Shutdown complete")))
        assertFalse(OfflineQueue.queueable(Command.CancelBlock("b")))
        assertFalse(OfflineQueue.queueable(Command.OptOutOfBlocklist("social")))
        assertFalse(OfflineQueue.queueable(Command.UseShutdownDelay))
        assertFalse(OfflineQueue.queueable(Command.ChangeSettings(listOf(1), "09:00", "17:00")))
    }

    @Test
    fun anAcceptedCommandIsQueuedAndShowsAtOnceAndSurvivesARestart() {
        val m = mirror()
        assertEquals(CommandOutcome.Accepted, m.decideOffline(start("b1")))
        assertEquals(1, m.queued.size)
        assertEquals(listOf("b1"), m.replayedState().blocks.map { it.id })
        assertTrue(m.state.blocks.isEmpty()) // the cached backend state itself is untouched
        val again = Mirror(file, FixedClock())
        assertEquals(m.queued, again.queued)
        assertEquals(listOf("b1"), again.replayedState().blocks.map { it.id })
    }

    @Test
    fun aRejectedCommandIsNotQueued() {
        val m = mirror()
        m.decideOffline(start("b1"))
        val outcome = m.decideOffline(start("b1"))
        assertEquals(CommandOutcome.Rejected("duplicate_block"), outcome)
        assertEquals(1, m.queued.size)
    }

    @Test
    fun aDeepStartInEveningLockdownIsRefusedOffline() {
        val m = mirror()
        m.update { it.copy(state = State(settings = Settings(listOf(1, 2, 3, 4, 5), "06:00", "08:00"))) }
        val outcome = m.decideOffline(Command.StartBlock(blockId = "d", minutes = 60, deep = true, intent = "Draft"))
        assertEquals(CommandOutcome.Rejected("evening_lockdown"), outcome)
        assertTrue(m.queued.isEmpty())
    }

    @Test
    fun breachesGoThroughTheQueue() {
        val m = mirror()
        val cmd = m.recordBreach(BreachKind.VpnTurnedOff)
        assertEquals(listOf(cmd.breachId), m.replayedState().breaches.map { it.id })
    }

    @Test
    fun flushSendsInOrderDropsAnsweredAndStopsAtTheFirstNetworkFailure() = runBlocking {
        val m = mirror()
        m.decideOffline(start("b1"))
        m.decideOffline(start("b2"))
        m.decideOffline(start("b3"))
        val sent = mutableListOf<String>()
        val result = runCatching {
            OfflineQueue.flush(m.queued, { c ->
                val id = (c as Command.StartBlock).blockId
                when (id) {
                    "b1" -> CommandOutcome.Accepted.also { sent += id }
                    "b2" -> CommandOutcome.Rejected("duplicate_block").also { sent += id } // the backend's answer stands
                    else -> throw IOException("unreachable")
                }
            }) { m.dropQueued(it) }
        }
        assertTrue(OfflineQueue.isOffline(result.exceptionOrNull()!!))
        assertEquals(listOf("b1", "b2"), sent)
        assertEquals(listOf("b3"), m.queued.map { (it.command as Command.StartBlock).blockId })
    }
}
