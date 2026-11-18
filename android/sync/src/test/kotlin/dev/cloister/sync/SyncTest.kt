package dev.cloister.sync

import dev.cloister.core.BreachKind
import dev.cloister.core.Command
import java.io.File
import java.nio.file.Files
import java.util.UUID
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Two phones on one account against a local Supabase (run through `backend/scripts/test.sh`
 * or with CLOISTER_API_URL / CLOISTER_ANON_KEY set; skipped otherwise). Uses the local stack's
 * email + password sign-in so no email round trip is needed.
 */
class SyncTest {
    private val url = System.getenv("CLOISTER_API_URL")
    private val key = System.getenv("CLOISTER_ANON_KEY")
    private val dir: File = Files.createTempDirectory("cloister-sync").toFile()

    private fun device(name: String, url: String = this.url, file: File = File(dir, "$name.json")): Device {
        val mirror = Mirror(file)
        return Device(Device.client(url, key, mirror), mirror, name)
    }

    @Test
    fun aBlockStartedOnOneDeviceIsEnforcedOnTheOtherAndOffline() = runBlocking {
        assumeTrue("needs a local Supabase", url != null && key != null)
        val email = "phone-${UUID.randomUUID()}@cloister.test"
        val a = device("a")
        val b = device("b")
        a.signInWithPassword(email, "correct horse battery", signUp = true)
        b.signInWithPassword(email, "correct horse battery")
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        b.startSync(scope)
        delay(3.seconds) // let B's Realtime subscription take hold

        val outcome = a.send(Command.StartBlock("blk-${UUID.randomUUID()}", 30, intent = "Port the engine"))
        assertEquals(CommandOutcome.Accepted, outcome)
        withTimeout(15.seconds) {
            while ("youtube.com" !in b.enforcement().blockedDomains) delay(200)
        }
        // Rules are the backend's: a cancel is refused with a stable reason.
        val id = a.state().blocks.single().id
        assertEquals(CommandOutcome.Rejected("block_cannot_be_cancelled"), a.send(Command.CancelBlock(id)))

        // A Breach is synced to the account.
        a.recordBreach(BreachKind.VpnTurnedOff)
        assertTrue(a.mirror.queued.isEmpty())
        withTimeout(15.seconds) {
            while (b.state().breaches.isEmpty()) delay(200)
        }
        scope.cancel()

        // Offline: a Device that can't reach the backend still enforces from its cache.
        val offline = device("b-offline", url = "http://127.0.0.1:9", file = File(dir, "b.json"))
        assertTrue("youtube.com" in offline.enforcement().blockedDomains)
        assertTrue(runCatching { offline.pull() }.isFailure)
        assertEquals(SyncStatus.Offline, offline.status.value)
        assertTrue("youtube.com" in offline.enforcement().blockedDomains)
    }

    @Test
    fun theWeeklyTargetAndSettingsStartReachTheOtherPhone() = runBlocking {
        assumeTrue("needs a local Supabase", url != null && key != null)
        val email = "phone-${UUID.randomUUID()}@cloister.test"
        val a = device("wa")
        val b = device("wb")
        a.signInWithPassword(email, "correct horse battery", signUp = true)
        b.signInWithPassword(email, "correct horse battery")
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        b.startSync(scope)
        delay(3.seconds)

        assertEquals(CommandOutcome.Accepted, a.send(Command.SetWeeklyTarget(20)))
        assertEquals(CommandOutcome.Accepted, a.send(Command.SetWeeklyTarget(15)))
        assertEquals(
            CommandOutcome.Accepted,
            a.send(Command.ChangeSettings(listOf(1, 2, 3, 4, 5), "09:00", "17:00", "UTC")),
        )
        withTimeout(15.seconds) {
            while (b.state().pendingWeeklyTarget == null || b.state().settingsSince == null) delay(200)
        }
        assertEquals(20, b.state().weeklyTarget?.hours)
        assertEquals(15, b.state().pendingWeeklyTarget?.hours)
        scope.cancel()
    }
}
