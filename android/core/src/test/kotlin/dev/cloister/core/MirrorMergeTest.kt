package dev.cloister.core

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The phone's cached state only tightens when the backend's state is folded in, so a stale or
 * tampered response can't loosen what is enforced offline (same rule as the desktop mirror).
 */
class MirrorMergeTest {
    private val t = { s: String -> Instant.parse(s) }
    private val lists = listOf("forums", "news", "shopping", "social", "video")
    private fun block(id: String, ends: String, deep: Boolean = true) =
        Block(id, t("2026-01-05T09:00:00Z"), t(ends), lists, deep)

    private val now = t("2026-01-05T09:30:00Z")

    @Test
    fun newBlocksAreAddedAndLaterEndsTaken() {
        val local = State(blocks = listOf(block("a", "2026-01-05T10:00:00Z")))
        val incoming = State(blocks = listOf(block("a", "2026-01-05T11:00:00Z"), block("b", "2026-01-05T12:00:00Z")))
        val merged = MirrorMerge.merge(local, incoming, now)
        assertEquals(listOf("a" to "2026-01-05T11:00:00Z", "b" to "2026-01-05T12:00:00Z"),
            merged.blocks.map { it.id to it.endsAt.toString() })
    }

    @Test
    fun aBlockIsNeverShortenedOrDropped() {
        val local = State(blocks = listOf(block("a", "2026-01-05T11:00:00Z")))
        val merged = MirrorMerge.merge(local, State(blocks = listOf(block("a", "2026-01-05T10:00:00Z"))), now)
        assertEquals(t("2026-01-05T11:00:00Z"), merged.blocks.single().endsAt)
        assertEquals(1, MirrorMerge.merge(local, State(), now).blocks.size)
    }

    @Test
    fun aNewRemovalTakesEffectNoSoonerThanTheNextDayBoundary() {
        val incoming = State(
            blocklistRemovals = listOf(BlocklistRemoval("forums", EntryKind.Domain, "reddit.com", t("2026-01-05T09:00:00Z"))),
        )
        val merged = MirrorMerge.merge(State(), incoming, now)
        assertEquals(t("2026-01-06T00:00:00Z"), merged.blocklistRemovals.single().effectiveAt)
        assertEquals(listOf("reddit.com"), Blocklists.effectiveEntries(merged, "forums", EntryKind.Domain, now).filter { it == "reddit.com" })
    }

    @Test
    fun additionsAreKeptAndAKnownDepthRatingStays() {
        val entry = BlocklistEntry("news", EntryKind.Domain, "example.com")
        val local = State(
            blocks = listOf(block("a", "2026-01-05T09:20:00Z").copy(depthRating = 4)),
            blocklistEntries = listOf(entry),
        )
        val merged = MirrorMerge.merge(local, State(blocks = listOf(block("a", "2026-01-05T09:20:00Z"))), now)
        assertEquals(listOf(entry), merged.blocklistEntries)
        assertEquals(4, merged.blocks.single().depthRating)
    }

    @Test
    fun laterSettingsFromTheBackendWaitForTomorrow() {
        val nineToFive = Settings(listOf(1, 2, 3, 4, 5), "09:00", "17:00")
        val local = State(settings = nineToFive)
        // A stale or hostile response that moves Shutdown Time later must not apply today.
        val incoming = State(settings = Settings(listOf(1, 2, 3, 4, 5), "09:00", "20:00"))
        val merged = MirrorMerge.merge(local, incoming, now)
        assertEquals(nineToFive, merged.settings)
        assertEquals(t("2026-01-06T00:00:00Z"), merged.pendingSettings!!.effectiveAt)
        // An earlier Shutdown Time applies at once.
        val stricter = MirrorMerge.merge(local, State(settings = Settings(listOf(1, 2, 3, 4, 5), "09:00", "16:00")), now)
        assertEquals("16:00", stricter.settings!!.shutdownTime)
        assertEquals(null, stricter.pendingSettings)
    }
}
