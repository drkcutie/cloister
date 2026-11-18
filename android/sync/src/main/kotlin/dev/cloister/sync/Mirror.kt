package dev.cloister.sync

import dev.cloister.core.BreachKind
import dev.cloister.core.ClockAnchor
import dev.cloister.core.Command
import dev.cloister.core.DeviceClock
import dev.cloister.core.JvmDeviceClock
import dev.cloister.core.State
import dev.cloister.core.TrustedClock
import dev.cloister.core.parseInstant
import java.io.File
import java.time.Instant
import java.util.UUID
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put

/**
 * This Device's cached copy of the account: the state it enforces offline, its Device id, the
 * trusted clock's anchor (the last server time and the boot clock then), the session, and
 * commands decided offline, not yet sent. One JSON file, rewritten atomically. Thread-safe.
 */
class Mirror(private val file: File, private val clock: DeviceClock = JvmDeviceClock) {
    private val lock = Any()
    private var data: Data = load()

    data class Data(
        val deviceId: String = UUID.randomUUID().toString(),
        val state: State = State(),
        /** Null until the first server time; then trusted now runs on from it (see [TrustedClock]). */
        val anchor: ClockAnchor? = null,
        val session: String? = null,
        /** Commands decided offline (Breaches too), sent in order on the next sync. */
        val queued: List<Queued> = emptyList(),
    )

    val deviceId: String get() = synchronized(lock) { data.deviceId }
    val state: State get() = synchronized(lock) { data.state }
    val anchor: ClockAnchor? get() = synchronized(lock) { data.anchor }
    var session: String?
        get() = synchronized(lock) { data.session }
        set(value) = update { it.copy(session = value) }
    val queued: List<Queued> get() = synchronized(lock) { data.queued }

    /** The cached state with the queued commands applied: what this Device shows and enforces. */
    fun replayedState(): State = synchronized(lock) { OfflineQueue.replay(data.state, data.queued) }

    /** Decides [command] here at trusted now and queues it if accepted. */
    fun decideOffline(command: Command): CommandOutcome = synchronized(lock) {
        val (outcome, queue) = OfflineQueue.decide(replayedState(), data.queued, command, serverTime())
        update { it.copy(queued = queue) }
        outcome
    }

    /** Drops a queued command the backend has answered. */
    fun dropQueued(q: Queued) = update { d -> d.copy(queued = d.queued.filterNot { it == q }) }

    /**
     * Trusted now: the last server time plus the boot-clock time since, never the wall clock (which
     * the user can set). Re-anchors at most every 30 s, and at once after a reboot, so a reboot
     * loses little. Before the first sync there is nothing to enforce; the first reading of the
     * wall clock is the anchor until the server's time replaces it.
     */
    fun serverTime(): Instant = synchronized(lock) {
        val elapsed = clock.elapsedMillis()
        val boot = clock.bootCount()
        val anchor = data.anchor ?: ClockAnchor(clock.wallNow(), elapsed, boot)
        val kept = TrustedClock.checkpoint(anchor, elapsed, boot)
        if (kept != data.anchor) update { it.copy(anchor = kept) }
        TrustedClock.now(kept, elapsed, boot)
    }

    /** The wall clock; only ever used to enforce more (see [TrustedClock.enforcement]). */
    fun wallTime(): Instant = clock.wallNow()

    /** Anchors trusted now at a server time just read. */
    fun anchorServerTime(serverNow: Instant) = update { it.copy(anchor = TrustedClock.anchor(serverNow, clock)) }

    fun update(change: (Data) -> Data) = synchronized(lock) {
        val next = change(data)
        if (next != data) {
            data = next
            save(next)
        }
    }

    /**
     * Records a Breach at once (it shows offline through the queue) and queues it for the
     * backend, which stamps the server time and records each id once.
     */
    fun recordBreach(kind: BreachKind): Command.RecordBreach {
        val cmd = Command.RecordBreach("${deviceId.take(8)}-${UUID.randomUUID()}", kind, Instant.now())
        val at = serverTime()
        update { it.copy(queued = it.queued + Queued(cmd, at)) }
        return cmd
    }

    private fun load(): Data {
        if (!file.exists()) return Data().also(::save)
        return try {
            val o = Json.parseToJsonElement(file.readText()).jsonObject
            Data(
                deviceId = o.getValue("device_id").jsonPrimitive.content,
                state = State.fromJson(o.getValue("state")),
                anchor = (o["clock_anchor"] as? kotlinx.serialization.json.JsonObject)?.let { a ->
                    ClockAnchor(
                        trustedAt = parseInstant(a.getValue("trusted_at").jsonPrimitive.content),
                        elapsedAt = a.getValue("elapsed_at").jsonPrimitive.long,
                        boot = a["boot"]?.jsonPrimitive?.intOrNull,
                    )
                },
                session = (o["session"] as? JsonPrimitive)?.content,
                queued = (o["queued_commands"] as? JsonArray).orEmpty().map { q ->
                    val qo = q.jsonObject
                    Queued(Command.fromJson(qo.getValue("command")), parseInstant(qo.getValue("queued_at").jsonPrimitive.content))
                } + (o["pending_breaches"] as? JsonArray).orEmpty() // from before the queue
                    .map { Queued(Command.fromJson(it), Instant.now()) },
            )
        } catch (e: Exception) {
            // A corrupt cache must not stop enforcement of a fresh one; the next sync refills it.
            Data()
        }
    }

    private fun save(d: Data) {
        file.parentFile?.mkdirs()
        val json = buildJsonObject {
            put("device_id", d.deviceId)
            put("state", d.state.toJson())
            d.anchor?.let { a ->
                put(
                    "clock_anchor",
                    buildJsonObject {
                        put("trusted_at", a.trustedAt.toString())
                        put("elapsed_at", a.elapsedAt)
                        a.boot?.let { put("boot", it) }
                    },
                )
            }
            d.session?.let { put("session", it) }
            put(
                "queued_commands",
                JsonArray(d.queued.map { q -> buildJsonObject { put("command", q.command.toJson()); put("queued_at", q.queuedAt.toString()) } }),
            )
        }
        val tmp = File(file.path + ".tmp")
        tmp.writeText(json.toString())
        if (!tmp.renameTo(file)) {
            file.delete()
            tmp.renameTo(file)
        }
    }
}
