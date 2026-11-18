package dev.cloister.core

import java.time.Instant

/**
 * Folds the backend's state into the Device's cached state so that nothing loosens sooner than
 * tomorrow from here (the Kotlin twin of the desktop mirror's merge):
 * - Blocks: new ones are added, a later end is taken; nothing is shortened or dropped, and a
 *   known Intent or Depth Rating stays.
 * - Blocklist edits: additions are kept; a removal or opt-out the backend no longer has is
 *   dropped; a known one keeps its date; a new one takes effect no earlier than the next day
 *   boundary after [now].
 * - Day Plans and Schedules are replaced (the backend already applied the ratchet).
 * - Breaches: the union.
 */
object MirrorMerge {
    fun merge(local: State, incoming: State, now: Instant): State {
        val blocks = local.blocks.associateBy { it.id }.toMutableMap()
        for (b in incoming.blocks) {
            val known = blocks[b.id]
            blocks[b.id] = if (known == null) b else known.copy(
                endsAt = maxOf(known.endsAt, b.endsAt),
                intent = known.intent ?: b.intent,
                depthRating = known.depthRating ?: b.depthRating,
            )
        }
        val withZone = local.copy(timeZone = incoming.timeZone)
        val boundary = Blocklists.nextDayBoundary(withZone, now)
        val removals = incoming.blocklistRemovals.map { r ->
            val known = local.blocklistRemovals.firstOrNull { it.matches(r.blocklist, r.kind, r.entry) }
            r.copy(effectiveAt = known?.effectiveAt ?: maxOf(r.effectiveAt, boundary))
        }.sorted().distinctBy { Triple(it.blocklist, it.kind, it.entry) }
        val optOuts = incoming.blocklistOptOuts
            .filter { Catalog.builtin.get(it.blocklist)?.kind == BlocklistKind.Distraction }
            .map { o ->
                val known = local.blocklistOptOuts.firstOrNull { it.blocklist == o.blocklist }
                o.copy(effectiveAt = known?.effectiveAt ?: maxOf(o.effectiveAt, boundary))
            }.sorted().distinctBy { it.blocklist }
        val breaches = (local.breaches + incoming.breaches).distinctBy { it.id }
            .sortedWith(compareBy({ it.occurredAt }, { it.id }))
        return State(
            blocks = blocks.values.sortedWith(compareBy({ it.startsAt }, { it.id })),
            dayPlans = incoming.dayPlans,
            schedules = incoming.schedules,
            timeZone = incoming.timeZone,
            blocklistEntries = (local.blocklistEntries + incoming.blocklistEntries).toSortedSet().toList(),
            blocklistRemovals = removals,
            blocklistOptOuts = optOuts,
            breaches = breaches,
        )
    }
}
