package dev.cloister.sync

import dev.cloister.core.Breach
import dev.cloister.core.BreachKind
import dev.cloister.core.Command
import dev.cloister.core.State
import java.io.File
import java.time.Instant
import java.util.UUID
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/**
 * This Device's cached copy of the account: the state it enforces offline, its Device id, the
 * server clock offset seen at the last sync, the session, and Breaches not yet sent. One JSON
 * file, rewritten atomically. Thread-safe.
 */
class Mirror(private val file: File) {
    private val lock = Any()
    private var data: Data = load()

    data class Data(
        val deviceId: String = UUID.randomUUID().toString(),
        val state: State = State(),
        val serverOffsetMillis: Long = 0,
        val session: String? = null,
        /** Breaches recorded while offline, sent (idempotently) on the next sync. */
        val pendingBreaches: List<Command.RecordBreach> = emptyList(),
    )

    val deviceId: String get() = synchronized(lock) { data.deviceId }
    val state: State get() = synchronized(lock) { data.state }
    val serverOffsetMillis: Long get() = synchronized(lock) { data.serverOffsetMillis }
    var session: String?
        get() = synchronized(lock) { data.session }
        set(value) = update { it.copy(session = value) }
    val pendingBreaches: List<Command.RecordBreach> get() = synchronized(lock) { data.pendingBreaches }

    /** The local clock corrected by the offset seen at the last sync. */
    fun serverTime(): Instant = Instant.now().plusMillis(serverOffsetMillis)

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
        update {
            val breach = Breach(cmd.breachId, kind, serverTime())
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
                serverOffsetMillis = o["server_offset_ms"]?.jsonPrimitive?.longOrNull ?: 0,
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
            put("server_offset_ms", d.serverOffsetMillis)
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
