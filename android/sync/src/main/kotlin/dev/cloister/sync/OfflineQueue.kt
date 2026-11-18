package dev.cloister.sync

import dev.cloister.core.Command
import dev.cloister.core.Outcome
import dev.cloister.core.RulesEngine
import dev.cloister.core.State
import java.io.IOException
import java.time.Instant

/** A command decided on this Device while offline, sent to the backend when it can be reached. */
data class Queued(val command: Command, val queuedAt: Instant)

/**
 * The offline command queue (the twin of the desktop Device's): commands that only tighten or
 * record, and the Shutdown Ritual's completion, may be decided here while the backend can't be
 * reached, shown at once and sent later. The backend decides each again; its answer stands.
 */
object OfflineQueue {
    /** Commands that may be decided offline. Loosening ones (cancel, removals, opt-outs, settings, plans, delay) need the network. */
    fun queueable(command: Command): Boolean = when (command) {
        is Command.StartBlock, is Command.ExtendBlock, is Command.StartQuitSocialBlock,
        is Command.AddBlocklistEntry, is Command.OptInToBlocklist, is Command.RecordDepthRating,
        is Command.RecordBreach, is Command.CaptureLooseEnd, is Command.CompleteRitual -> true
        else -> false
    }

    /** [state] with the queued commands applied on top, each as it was decided at its queue time. */
    fun replay(state: State, queue: List<Queued>): State = queue.fold(state) { s, q ->
        val decision = RulesEngine.decide(s, q.command, q.queuedAt)
        if (decision.outcome == Outcome.Accepted) decision.state else s
    }

    /**
     * Decides [command] offline at [now] against [state] (the replayed state). Accepted: the queue
     * with it appended; rejected: the engine's reason and the queue unchanged.
     */
    fun decide(state: State, queue: List<Queued>, command: Command, now: Instant): Pair<CommandOutcome, List<Queued>> =
        when (val outcome = RulesEngine.decide(state, command, now).outcome) {
            Outcome.Accepted -> CommandOutcome.Accepted to queue + Queued(command, now)
            is Outcome.Rejected -> CommandOutcome.Rejected(outcome.reason.code) to queue
        }

    /**
     * Sends the queue in order. Each command is dropped (through [drop]) once the backend has
     * answered, accepted or rejected. Stops at the first that can't reach it, rethrowing.
     */
    suspend fun flush(queue: List<Queued>, send: suspend (Command) -> CommandOutcome, drop: (Queued) -> Unit) {
        for (q in queue) {
            send(q.command)
            drop(q)
        }
    }

    /** Whether [e] means the backend couldn't be reached (rather than that it answered). */
    fun isOffline(e: Throwable): Boolean =
        e is io.github.jan.supabase.exceptions.HttpRequestException || e is IOException ||
            e.cause?.let { it !== e && isOffline(it) } == true
}
