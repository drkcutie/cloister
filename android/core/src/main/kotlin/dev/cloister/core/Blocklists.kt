package dev.cloister.core

import java.time.Instant
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.ZonedDateTime

/**
 * The account's own Blocklist edits on top of the built-in catalog, under **tighten now, loosen
 * tomorrow** (ADR 0001): additions apply at once, removals and opt-outs at the next day boundary.
 */
object Blocklists {
    /** An IANA zone, or null. Offsets like `+08:00` are not IANA names and are refused. */
    fun zone(name: String): ZoneId? =
        if (name in ZoneId.getAvailableZoneIds() || name == "UTC") ZoneId.of(name) else null

    /**
     * The next local midnight after [now] in the state's time zone (UTC if unknown): when a
     * loosening change made at [now] takes effect. A midnight skipped by a clock change maps to
     * the first instant after it; a repeated one to its later instant.
     */
    fun nextDayBoundary(state: State, now: Instant): Instant {
        val zone = zone(state.timeZone) ?: ZoneOffset.UTC
        val tomorrow = now.atZone(zone).toLocalDate().plusDays(1)
        return ZonedDateTime.ofLocal(tomorrow.atStartOfDay(), zone, null)
            .withLaterOffsetAtOverlap().toInstant()
    }

    /** Trim, lower-case, drop a trailing dot; null unless it is a plausible domain. */
    fun normalizeDomain(input: String): String? {
        val lower = input.trim().lowercase(java.util.Locale.ROOT)
        val d = lower.removeSuffix(".")
        val labels = d.split('.')
        fun ok(l: String) = l.length in 1..63 &&
            l.all { it in 'a'..'z' || it in '0'..'9' || it == '-' } &&
            !l.startsWith('-') && !l.endsWith('-')
        // Rust lower-cases ASCII only; a non-ASCII letter must still be refused.
        if (input.trim().any { it.code > 127 }) return null
        return d.takeIf { it.length <= 253 && labels.size >= 2 && labels.all(::ok) }
    }

    /** Trim; null when blank or longer than 200 characters. */
    fun normalizeApp(input: String): String? =
        input.trim().takeIf { it.isNotEmpty() && it.codePointCount(0, it.length) <= 200 }

    internal fun normalize(kind: EntryKind, input: String): String =
        (if (kind == EntryKind.Domain) normalizeDomain(input) else normalizeApp(input))
            ?: throw Rejection(Reason.InvalidEntry)

    internal fun normalizeAll(kind: EntryKind, inputs: List<String>): List<String> =
        inputs.map { normalize(kind, it) }.toSortedSet().toList()

    private fun knownBlocklist(id: String) {
        if (Catalog.builtin.get(id) == null) throw Rejection(Reason.UnknownBlocklist)
    }

    /** Built-in entries plus the account's additions, before removals. */
    private fun listed(state: State, blocklist: String, kind: EntryKind): java.util.SortedSet<String> {
        val builtin = Catalog.builtin.get(blocklist)?.let { if (kind == EntryKind.Domain) it.domains else it.apps }
        val set = builtin.orEmpty().toSortedSet()
        state.blocklistEntries.filter { it.blocklist == blocklist && it.kind == kind }.mapTo(set) { it.entry }
        return set
    }

    /** The entries of a Blocklist in force at [now], sorted. */
    fun effectiveEntries(state: State, blocklist: String, kind: EntryKind, now: Instant): List<String> {
        val set = listed(state, blocklist, kind)
        state.blocklistRemovals
            .filter { it.blocklist == blocklist && it.kind == kind && !it.isPending(now) }
            .forEach { set.remove(it.entry) }
        return set.toList()
    }

    /** Ids of the Distraction List categories in force at [now] (not opted out), sorted. */
    fun distractionIdsInForce(state: State, now: Instant): List<String> =
        Catalog.builtin.distractionIds().filter { id ->
            state.blocklistOptOuts.none { it.blocklist == id && !it.isPending(now) }
        }.sorted()

    internal fun addEntry(state: State, blocklist: String, kind: EntryKind, input: String): State {
        knownBlocklist(blocklist)
        val entry = normalize(kind, input)
        var next = state.copy(blocklistRemovals = state.blocklistRemovals.filterNot { it.matches(blocklist, kind, entry) })
        if (entry !in listed(next, blocklist, kind)) {
            next = next.copy(blocklistEntries = (next.blocklistEntries + BlocklistEntry(blocklist, kind, entry)).sorted())
        }
        return next
    }

    internal fun removeEntry(state: State, blocklist: String, kind: EntryKind, input: String, now: Instant): State {
        knownBlocklist(blocklist)
        val entry = normalize(kind, input)
        if (entry !in effectiveEntries(state, blocklist, kind, now)) throw Rejection(Reason.UnknownEntry)
        if (state.blocklistRemovals.any { it.matches(blocklist, kind, entry) }) return state
        val removal = BlocklistRemoval(blocklist, kind, entry, nextDayBoundary(state, now))
        return state.copy(blocklistRemovals = (state.blocklistRemovals + removal).sorted())
    }

    internal fun optOut(state: State, blocklist: String, now: Instant): State {
        knownBlocklist(blocklist)
        if (Catalog.builtin.get(blocklist)?.kind != BlocklistKind.Distraction) throw Rejection(Reason.CannotOptOut)
        if (state.blocklistOptOuts.any { it.blocklist == blocklist }) return state
        val optOut = BlocklistOptOut(blocklist, nextDayBoundary(state, now))
        return state.copy(blocklistOptOuts = (state.blocklistOptOuts + optOut).sorted())
    }

    internal fun optIn(state: State, blocklist: String): State {
        knownBlocklist(blocklist)
        return state.copy(blocklistOptOuts = state.blocklistOptOuts.filterNot { it.blocklist == blocklist })
    }
}
