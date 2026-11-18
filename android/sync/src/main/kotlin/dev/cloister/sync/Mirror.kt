package dev.cloister.sync

import dev.cloister.core.Breach
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
 * Breaches not yet sent. One JSON file, rewritten atomically. Thread-safe.
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
        /** Breaches recorded while offline, sent (idempotently) on the next sync. */
        val pendingBreaches: List<Command.RecordBreach> = emptyList(),
    )

    val deviceId: String get() = synchronized(lock) { data.deviceId }
    val state: State get() = synchronized(lock) { data.state }
    val anchor: ClockAnchor? get() = synchronized(lock) { data.anchor }
    var session: String?
        get() = synchronized(lock) { data.session }
        set(value) = update { it.copy(session = value) }
    val pendingBreaches: List<Command.RecordBreach> get() = synchronized(lock) { data.pendingBreaches }

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
     * Records a Breach in the cached state at once (so it shows offline) and queues it for the
     * backend, which stamps the server time.
     */
    fun recordBreach(kind: BreachKind): Command.RecordBreach {
        val cmd = Command.RecordBreach("${deviceId.take(8)}-${UUID.randomUUID()}", kind, Instant.now())
        val breach = Breach(cmd.breachId, kind, serverTime())
        update {
            it.copy(
                state = it.state.copy(breaches = it.state.breaches + breach),
                pendingBreaches = it.pendingBreaches + cmd,
            )
        }
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
                pendingBreaches = (o["pending_breaches"] as? JsonArray).orEmpty()
                    .map { Command.fromJson(it) as Command.RecordBreach },
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
            put("pending_breaches", JsonArray(d.pendingBreaches.map { it.toJson() }))
        }
        val tmp = File(file.path + ".tmp")
        tmp.writeText(json.toString())
        if (!tmp.renameTo(file)) {
            file.delete()
            tmp.renameTo(file)
        }
    }
}
