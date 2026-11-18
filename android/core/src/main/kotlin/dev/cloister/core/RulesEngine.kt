package dev.cloister.core

import java.time.Duration
import java.time.Instant

/**
 * Cloister rules engine, the Kotlin twin of `cloister-core`: [decide] takes the current [State], a
 * [Command] and the **server time** and returns the new state plus the [Enforcement] to apply
 * now; [enforcement] answers without a command. It never reads a clock. Behaviour is pinned by
 * the shared scenarios in `fixtures/rules`.
 */
object RulesEngine {
    val MAX_BLOCK: Duration = Duration.ofHours(24)
    val CONFIRM_LONG_AFTER: Duration = Duration.ofHours(4)
    val QUIT_SOCIAL_BLOCK: Duration = Duration.ofDays(30)
    const val QUIT_SOCIAL_BLOCKLIST = "social"

    fun decide(state: State, command: Command, serverTime: Instant): Decision = try {
        val next = apply(state, command, serverTime)
        Decision(Outcome.Accepted, next, enforcement(next, serverTime))
    } catch (r: Rejection) {
        Decision(Outcome.Rejected(r.reason), state, enforcement(state, serverTime))
    }

    private fun checkLength(length: Duration, confirmLong: Boolean) {
        when {
            length <= Duration.ZERO -> throw Rejection(Reason.InvalidLength)
            length > MAX_BLOCK -> throw Rejection(Reason.BlockTooLong)
            length > CONFIRM_LONG_AFTER && !confirmLong -> throw Rejection(Reason.ConfirmationRequired)
        }
    }

    private fun isSlotId(state: State, id: String) = state.dayPlans.any { p -> p.slots.any { it.id == id } }

    private fun idTaken(state: State, id: String) = state.blocks.any { it.id == id } || isSlotId(state, id)

    private fun find(state: State, id: String) =
        state.blocks.firstOrNull { it.id == id } ?: throw Rejection(Reason.UnknownBlock)

    private fun State.replace(block: Block) = copy(blocks = blocks.map { if (it.id == block.id) block else it })

    private fun apply(state: State, command: Command, now: Instant): State = when (command) {
        is Command.StartBlock -> {
            // Guard against overflow before converting: anything past a day is too long anyway.
            val length = if (command.minutes > MAX_BLOCK.toMinutes() * 2) MAX_BLOCK.plusMinutes(1)
            else Duration.ofMinutes(command.minutes)
            checkLength(length, command.confirmLong)
            if (command.blocklists.any { Catalog.builtin.get(it) == null }) throw Rejection(Reason.UnknownBlocklist)
            val extraDomains = Blocklists.normalizeAll(EntryKind.Domain, command.extraDomains)
            val extraApps = Blocklists.normalizeAll(EntryKind.App, command.extraApps)
            if (idTaken(state, command.blockId)) throw Rejection(Reason.DuplicateBlock)
            val ids = (Blocklists.distractionIdsInForce(state, now) + command.blocklists).toSortedSet().toList()
            state.copy(
                blocks = state.blocks + Block(
                    id = command.blockId,
                    startsAt = now,
                    endsAt = now.plus(length),
                    blocklists = ids,
                    deep = command.deep,
                    intent = command.intent?.trim()?.takeIf { it.isNotEmpty() },
                    extraDomains = extraDomains,
                    extraApps = extraApps,
                ),
            )
        }
        is Command.StartQuitSocialBlock -> {
            if (!command.confirmLong) throw Rejection(Reason.ConfirmationRequired)
            if (idTaken(state, command.blockId)) throw Rejection(Reason.DuplicateBlock)
            state.copy(
                blocks = state.blocks + Block(
                    id = command.blockId,
                    startsAt = now,
                    endsAt = now.plus(QUIT_SOCIAL_BLOCK),
                    blocklists = listOf(QUIT_SOCIAL_BLOCKLIST),
                    deep = false,
                    quitSocial = true,
                ),
            )
        }
        is Command.AddBlocklistEntry -> Blocklists.addEntry(state, command.blocklist, command.kind, command.entry)
        is Command.RemoveBlocklistEntry ->
            Blocklists.removeEntry(state, command.blocklist, command.kind, command.entry, now)
        is Command.OptOutOfBlocklist -> Blocklists.optOut(state, command.blocklist, now)
        is Command.OptInToBlocklist -> Blocklists.optIn(state, command.blocklist)
        is Command.ExtendBlock -> {
            val block = find(state, command.blockId)
            when {
                !now.isBefore(block.endsAt) -> throw Rejection(Reason.BlockHasEnded)
                command.endsAt.isBefore(block.endsAt) -> throw Rejection(Reason.BlockCannotBeShortened)
                command.endsAt == block.endsAt -> state
                else -> {
                    checkLength(Duration.between(block.startsAt, command.endsAt), command.confirmLong)
                    state.replace(block.copy(endsAt = command.endsAt))
                }
            }
        }
        is Command.CancelBlock -> {
            find(state, command.blockId)
            throw Rejection(Reason.BlockCannotBeCancelled)
        }
        is Command.RecordDepthRating -> recordDepthRating(state, command, now)
        is Command.WriteDayPlan ->
            DayPlans.writeDayPlan(state, command.date, command.startsAt, command.endsAt, command.slots, now)
        is Command.PutSchedule -> DayPlans.putSchedule(state, command, now)
    }

    /** A Deep Session is a deep Block or a deep Slot of a saved Day Plan. */
    private fun recordDepthRating(state: State, command: Command.RecordDepthRating, now: Instant): State {
        val block = state.blocks.firstOrNull { it.id == command.blockId }
        val slot = if (block == null) {
            state.dayPlans.flatMap { it.slots }.firstOrNull { it.id == command.blockId }
                ?: throw Rejection(Reason.UnknownBlock)
        } else null
        val deep = block?.deep ?: slot!!.deep
        val endsAt = block?.endsAt ?: slot!!.endsAt
        val recorded = block?.depthRating ?: slot?.depthRating
        when {
            !deep -> throw Rejection(Reason.NotADeepSession)
            now.isBefore(endsAt) -> throw Rejection(Reason.DeepSessionNotEnded)
            command.rating !in 1..5 -> throw Rejection(Reason.InvalidDepthRating)
            recorded != null -> throw Rejection(Reason.DepthRatingAlreadyRecorded)
        }
        return if (block != null) {
            state.replace(block.copy(depthRating = command.rating))
        } else {
            state.copy(
                dayPlans = state.dayPlans.map { p ->
                    p.copy(slots = p.slots.map { if (it.id == slot!!.id) it.copy(depthRating = command.rating) else it })
                },
            )
        }
    }

    /** What must be enforced at [serverTime], given [state]. */
    fun enforcement(state: State, serverTime: Instant): Enforcement {
        val lists = sortedSetOf<String>()
        val domains = sortedSetOf<String>()
        val apps = sortedSetOf<String>()
        val planned = DayPlans.plannedDeepSessions(state, serverTime)
        for (block in (state.blocks + planned).filter { it.isActive(serverTime) }) {
            lists += block.blocklists
            // Opting back in to a category applies at once, also to a running Block.
            if (!block.quitSocial) lists += Blocklists.distractionIdsInForce(state, serverTime)
            domains += block.extraDomains
            apps += block.extraApps
        }
        for (list in lists) {
            domains += Blocklists.effectiveEntries(state, list, EntryKind.Domain, serverTime)
            apps += Blocklists.effectiveEntries(state, list, EntryKind.App, serverTime)
        }
        domains += domains.flatMap { Catalog.builtin.related(it) }
        return Enforcement(domains.toList(), apps.toList(), RitualScreen.None)
    }

    /** The active Block or planned Deep Session to show (the one ending last), if any. */
    fun activeSessions(state: State, now: Instant): List<Block> =
        (state.blocks + DayPlans.plannedDeepSessions(state, now)).filter { it.isActive(now) }
}
