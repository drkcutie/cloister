package dev.cloister.core

import java.time.Instant
import java.time.LocalDate
import java.time.OffsetDateTime
import java.time.format.DateTimeFormatter
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.longOrNull

/** A malformed State or Command (the JSON forms are documented in `fixtures/README.md`). */
class JsonFormException(message: String) : IllegalArgumentException(message)

/**
 * Strict reader over a JSON object: unknown keys are errors, like serde's `deny_unknown_fields`.
 * Call [done] after reading every field.
 */
internal class Fields(private val obj: JsonObject, private val what: String) {
    private val seen = mutableSetOf<String>()

    constructor(el: JsonElement, what: String) : this(
        el as? JsonObject ?: throw JsonFormException("$what: expected an object"), what,
    )

    private fun raw(key: String): JsonElement? {
        seen += key
        return obj[key]?.takeUnless { it is JsonNull }
    }

    private fun need(key: String) = raw(key) ?: throw JsonFormException("$what: missing `$key`")

    fun string(key: String) = need(key).asString(key)
    fun stringOrNull(key: String) = raw(key)?.asString(key)
    fun long(key: String) = need(key).asLong(key)
    fun longOrNull(key: String) = raw(key)?.asLong(key)
    fun bool(key: String, default: Boolean) = raw(key)?.asBool(key) ?: default
    fun bool(key: String) = need(key).asBool(key)
    fun instant(key: String) = parseInstant(string(key))
    fun instantOrNull(key: String) = stringOrNull(key)?.let(::parseInstant)
    fun date(key: String): LocalDate = try {
        LocalDate.parse(string(key))
    } catch (e: Exception) {
        throw JsonFormException("$what: `$key` must be a date")
    }
    fun strings(key: String, default: List<String>? = null): List<String> {
        val el = raw(key) ?: return default ?: throw JsonFormException("$what: missing `$key`")
        return (el as? JsonArray ?: throw JsonFormException("$what: `$key` must be an array"))
            .map { it.asString(key) }
    }
    fun <T> objects(key: String, default: List<T>? = null, read: (JsonElement) -> T): List<T> {
        val el = raw(key) ?: return default ?: throw JsonFormException("$what: missing `$key`")
        return (el as? JsonArray ?: throw JsonFormException("$what: `$key` must be an array")).map(read)
    }

    fun done() {
        val unknown = obj.keys - seen
        if (unknown.isNotEmpty()) throw JsonFormException("$what: unknown field(s) $unknown")
    }

    private fun JsonElement.asString(key: String) =
        (this as? JsonPrimitive)?.takeIf { it.isString }?.content
            ?: throw JsonFormException("$what: `$key` must be a string")

    private fun JsonElement.asBool(key: String) =
        (this as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull
            ?: throw JsonFormException("$what: `$key` must be a boolean")

    private fun JsonElement.asLong(key: String) =
        (this as? JsonPrimitive)?.takeUnless { it.isString }?.longOrNull
            ?: throw JsonFormException("$what: `$key` must be an integer")
}

/** RFC 3339 to an instant; any offset is accepted. */
fun parseInstant(s: String): Instant = try {
    OffsetDateTime.parse(s).toInstant()
} catch (e: Exception) {
    throw JsonFormException("not an RFC 3339 time: $s")
}

/** RFC 3339 in UTC with `Z`, whole seconds unless there is a fraction (chrono's form). */
fun formatInstant(t: Instant): String = DateTimeFormatter.ISO_INSTANT.format(t)

/** Builds a JSON object, leaving out entries whose value is null. */
internal fun obj(vararg entries: Pair<String, JsonElement?>): JsonObject =
    JsonObject(entries.mapNotNull { (k, v) -> v?.let { k to it } }.toMap())

internal fun str(s: String) = JsonPrimitive(s)
internal fun time(t: Instant) = JsonPrimitive(formatInstant(t))
internal fun strArray(xs: List<String>) = JsonArray(xs.map(::JsonPrimitive))
internal fun <T> arr(xs: List<T>, f: (T) -> JsonElement) = JsonArray(xs.map(f))
internal fun <T> nonEmpty(xs: List<T>, f: (List<T>) -> JsonElement) = if (xs.isEmpty()) null else f(xs)
