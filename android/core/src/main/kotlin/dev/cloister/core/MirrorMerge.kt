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
 * - Settings: what tightens now is taken now; what loosens waits for the next day boundary.
 * - Loose Ends, completed rituals, Shutdown Delays, the settings start and the weekly target are
 *   taken as they are (the backend applied the rules), as the desktop mirror does.
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
        val (settings, pending) = mergeSettings(local, incoming, now)
        return State(
            blocks = blocks.values.sortedWith(compareBy({ it.startsAt }, { it.id })),
            dayPlans = incoming.dayPlans,
            schedules = incoming.schedules,
            timeZone = incoming.timeZone,
            blocklistEntries = (local.blocklistEntries + incoming.blocklistEntries).toSortedSet().toList(),
            blocklistRemovals = removals,
            blocklistOptOuts = optOuts,
            breaches = breaches,
            settings = settings,
            pendingSettings = pending,
            settingsSince = incoming.settingsSince ?: local.settingsSince,
            weeklyTarget = incoming.weeklyTarget,
            pendingWeeklyTarget = incoming.pendingWeeklyTarget,
            looseEnds = incoming.looseEnds,
            rituals = incoming.rituals,
            shutdownDelays = incoming.shutdownDelays,
        )
    }

    /** `State::merge_settings` in the Rust engine. */
    private fun mergeSettings(local: State, incoming: State, now: Instant): Pair<Settings?, PendingSettings?> {
        val incomingNow = WorkdaySettings.settingsAt(incoming, now)
            ?: return local.settings to local.pendingSettings // the backend never drops settings
        val pending = incoming.pendingSettings?.takeIf { it.effectiveAt.isAfter(now) }
        val target = pending?.settings ?: incomingNow
        val base = WorkdaySettings.settingsAt(local, now) ?: return incomingNow to pending
        val inForce = base.tightest(incomingNow)
        if (inForce == target) return inForce to null
        val boundary = Blocklists.nextDayBoundary(local.copy(timeZone = incoming.timeZone), now)
        val known = local.pendingSettings?.takeIf { it.settings == target && it.effectiveAt.isAfter(now) }
        val effectiveAt = known?.effectiveAt ?: pending?.effectiveAt?.let { maxOf(it, boundary) } ?: boundary
        return inForce to PendingSettings(target, effectiveAt)
    }
}
