package dev.cloister.core

import java.time.Instant
import java.time.LocalDate
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** The time zone used for day boundaries when the account hasn't set one. */
const val DEFAULT_TIME_ZONE = "UTC"

/** Everything the rules engine knows. Its JSON form is shared with the Rust engine and the backend. */
data class State(
    val blocks: List<Block> = emptyList(),
    /** Saved Day Plans, sorted by date. */
    val dayPlans: List<DayPlan> = emptyList(),
    /** Schedules, sorted by id. */
    val schedules: List<Schedule> = emptyList(),
    /** IANA time zone for day boundaries ("tomorrow"). */
    val timeZone: String = DEFAULT_TIME_ZONE,
    val blocklistEntries: List<BlocklistEntry> = emptyList(),
    val blocklistRemovals: List<BlocklistRemoval> = emptyList(),
    val blocklistOptOuts: List<BlocklistOptOut> = emptyList(),
    /** Breaches recorded, sorted by time then id. */
    val breaches: List<Breach> = emptyList(),
    /** Workdays, Workday Start and Shutdown Time in force as of the last change; null until first set. */
    val settings: Settings? = null,
    /** Requested settings that loosen something, waiting for the next day boundary. */
    val pendingSettings: PendingSettings? = null,
    /** Open Loose Ends, sorted by capture then id. */
    val looseEnds: List<LooseEnd> = emptyList(),
    /** Completed Shutdown Rituals, sorted by date. */
    val rituals: List<Ritual> = emptyList(),
    /** Shutdown Delays used, sorted by date. */
    val shutdownDelays: List<ShutdownDelay> = emptyList(),
) {
    fun toJson(): JsonObject = obj(
        "blocks" to arr(blocks) { it.toJson() },
        "day_plans" to nonEmpty(dayPlans) { arr(it) { p -> p.toJson() } },
        "schedules" to nonEmpty(schedules) { arr(it) { s -> s.toJson() } },
        "time_zone" to timeZone.takeIf { it != DEFAULT_TIME_ZONE }?.let(::str),
        "blocklist_entries" to nonEmpty(blocklistEntries) { arr(it) { e -> e.toJson() } },
        "blocklist_removals" to nonEmpty(blocklistRemovals) { arr(it) { r -> r.toJson() } },
        "blocklist_opt_outs" to nonEmpty(blocklistOptOuts) { arr(it) { o -> o.toJson() } },
        "breaches" to nonEmpty(breaches) { arr(it) { b -> b.toJson() } },
        "settings" to settings?.toJson(),
        "pending_settings" to pendingSettings?.toJson(),
        "loose_ends" to nonEmpty(looseEnds) { arr(it) { l -> l.toJson() } },
        "rituals" to nonEmpty(rituals) { arr(it) { r -> r.toJson() } },
        "shutdown_delays" to nonEmpty(shutdownDelays) { arr(it) { d -> d.toJson() } },
    )

    companion object {
        fun fromJson(el: JsonElement): State = Fields(el, "state").run {
            State(
                blocks = objects("blocks", emptyList(), Block::fromJson),
                dayPlans = objects("day_plans", emptyList(), DayPlan::fromJson),
                schedules = objects("schedules", emptyList(), Schedule::fromJson),
                timeZone = stringOrNull("time_zone") ?: DEFAULT_TIME_ZONE,
                blocklistEntries = objects("blocklist_entries", emptyList(), BlocklistEntry::fromJson),
                blocklistRemovals = objects("blocklist_removals", emptyList(), BlocklistRemoval::fromJson),
                blocklistOptOuts = objects("blocklist_opt_outs", emptyList(), BlocklistOptOut::fromJson),
                breaches = objects("breaches", emptyList(), Breach::fromJson),
                settings = obj("settings", Settings::fromJson),
                pendingSettings = obj("pending_settings", PendingSettings::fromJson),
                looseEnds = objects("loose_ends", emptyList(), LooseEnd::fromJson),
                rituals = objects("rituals", emptyList(), Ritual::fromJson),
                shutdownDelays = objects("shutdown_delays", emptyList(), ShutdownDelay::fromJson),
            ).also { done() }
        }
    }
}

/** A period during which Blocklists are enforced. Its end can only move later. */
data class Block(
    val id: String,
    val startsAt: Instant,
    val endsAt: Instant,
    /** Ids of the Blocklists enforced, sorted. */
    val blocklists: List<String>,
    /** Whether this Block is a Deep Session. */
    val deep: Boolean,
    val intent: String? = null,
    val depthRating: Int? = null,
    val extraDomains: List<String> = emptyList(),
    val extraApps: List<String> = emptyList(),
    val quitSocial: Boolean = false,
) {
    fun isActive(now: Instant) = !startsAt.isAfter(now) && now.isBefore(endsAt)

    fun toJson(): JsonObject = obj(
        "id" to str(id),
        "starts_at" to time(startsAt),
        "ends_at" to time(endsAt),
        "blocklists" to strArray(blocklists),
        "deep" to JsonPrimitive(deep),
        "intent" to intent?.let(::str),
        "depth_rating" to depthRating?.let(::JsonPrimitive),
        "extra_domains" to nonEmpty(extraDomains, ::strArray),
        "extra_apps" to nonEmpty(extraApps, ::strArray),
        "quit_social" to if (quitSocial) JsonPrimitive(true) else null,
    )

    companion object {
        fun fromJson(el: JsonElement): Block = Fields(el, "block").run {
            Block(
                id = string("id"),
                startsAt = instant("starts_at"),
                endsAt = instant("ends_at"),
                blocklists = strings("blocklists"),
                deep = bool("deep"),
                intent = stringOrNull("intent"),
                depthRating = longOrNull("depth_rating")?.toInt(),
                extraDomains = strings("extra_domains", emptyList()),
                extraApps = strings("extra_apps", emptyList()),
                quitSocial = bool("quit_social", default = false),
            ).also { done() }
        }
    }
}

/** What a Blocklist entry is. Ordered as in the Rust engine: `app` before `domain`. */
enum class EntryKind(val code: String) {
    App("app"),
    Domain("domain");

    companion object {
        fun of(code: String) = entries.firstOrNull { it.code == code }
            ?: throw JsonFormException("unknown entry kind `$code`")
    }
}

/** A domain or app the account added to a Blocklist. */
data class BlocklistEntry(val blocklist: String, val kind: EntryKind, val entry: String) : Comparable<BlocklistEntry> {
    override fun compareTo(other: BlocklistEntry) =
        compareValuesBy(this, other, { it.blocklist }, { it.kind }, { it.entry })

    fun toJson() = obj("blocklist" to str(blocklist), "kind" to str(kind.code), "entry" to str(entry))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "blocklist entry").run {
            BlocklistEntry(string("blocklist"), EntryKind.of(string("kind")), string("entry")).also { done() }
        }
    }
}

/** A removal from a Blocklist, in force from [effectiveAt]. */
data class BlocklistRemoval(
    val blocklist: String,
    val kind: EntryKind,
    val entry: String,
    val effectiveAt: Instant,
) : Comparable<BlocklistRemoval> {
    fun isPending(now: Instant) = now.isBefore(effectiveAt)
    fun matches(blocklist: String, kind: EntryKind, entry: String) =
        this.blocklist == blocklist && this.kind == kind && this.entry == entry

    override fun compareTo(other: BlocklistRemoval) =
        compareValuesBy(this, other, { it.blocklist }, { it.kind }, { it.entry }, { it.effectiveAt })

    fun toJson() = obj(
        "blocklist" to str(blocklist), "kind" to str(kind.code), "entry" to str(entry),
        "effective_at" to time(effectiveAt),
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "blocklist removal").run {
            BlocklistRemoval(string("blocklist"), EntryKind.of(string("kind")), string("entry"), instant("effective_at"))
                .also { done() }
        }
    }
}

/** Opting out of a Distraction List category, in force from [effectiveAt]. */
data class BlocklistOptOut(val blocklist: String, val effectiveAt: Instant) : Comparable<BlocklistOptOut> {
    fun isPending(now: Instant) = now.isBefore(effectiveAt)

    override fun compareTo(other: BlocklistOptOut) =
        compareValuesBy(this, other, { it.blocklist }, { it.effectiveAt })

    fun toJson() = obj("blocklist" to str(blocklist), "effective_at" to time(effectiveAt))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "blocklist opt-out").run {
            BlocklistOptOut(string("blocklist"), instant("effective_at")).also { done() }
        }
    }
}

/** One day divided into Slots; [startsAt]/[endsAt] are the Workday's bounds. */
data class DayPlan(
    val date: LocalDate,
    val startsAt: Instant,
    val endsAt: Instant,
    /** Total length of the deep Slots, in minutes. Only goes up. */
    val plannedDeepMinutes: Long,
    /** Sorted by start. */
    val slots: List<Slot>,
) {
    fun toJson() = obj(
        "date" to str(date.toString()),
        "starts_at" to time(startsAt),
        "ends_at" to time(endsAt),
        "planned_deep_minutes" to JsonPrimitive(plannedDeepMinutes),
        "slots" to arr(slots) { it.toJson() },
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "day plan").run {
            DayPlan(
                date = date("date"),
                startsAt = instant("starts_at"),
                endsAt = instant("ends_at"),
                plannedDeepMinutes = long("planned_deep_minutes"),
                slots = objects("slots", read = Slot::fromJson),
            ).also { done() }
        }
    }
}

/** A labelled stretch of time on a Day Plan. A deep Slot is a scheduled Deep Session. */
data class Slot(
    val id: String,
    val startsAt: Instant,
    val endsAt: Instant,
    val label: String,
    val deep: Boolean,
    val depthRating: Int? = null,
) {
    val minutes: Long get() = java.time.Duration.between(startsAt, endsAt).toMinutes()

    fun toJson() = obj(
        "id" to str(id),
        "starts_at" to time(startsAt),
        "ends_at" to time(endsAt),
        "label" to str(label),
        "deep" to JsonPrimitive(deep),
        "depth_rating" to depthRating?.let(::JsonPrimitive),
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "slot").run {
            Slot(
                id = string("id"),
                startsAt = instant("starts_at"),
                endsAt = instant("ends_at"),
                label = string("label"),
                deep = bool("deep"),
                depthRating = longOrNull("depth_rating")?.toInt(),
            ).also { done() }
        }
    }
}

/** A recurring template for the Day Plans of ISO [weekdays]; times are local "HH:MM" in [timeZone]. */
data class Schedule(
    val id: String,
    val timeZone: String,
    val weekdays: List<Int>,
    val starts: String,
    val ends: String,
    val slots: List<ScheduleSlot>,
) {
    fun toJson() = obj(
        "id" to str(id),
        "time_zone" to str(timeZone),
        "weekdays" to arr(weekdays) { JsonPrimitive(it) },
        "starts" to str(starts),
        "ends" to str(ends),
        "slots" to arr(slots) { it.toJson() },
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "schedule").run {
            Schedule(
                id = string("id"),
                timeZone = string("time_zone"),
                weekdays = objects("weekdays", read = ::weekday),
                starts = string("starts"),
                ends = string("ends"),
                slots = objects("slots", read = ScheduleSlot::fromJson),
            ).also { done() }
        }
    }
}

internal fun weekday(el: JsonElement): Int =
    (el as? JsonPrimitive)?.takeUnless { it.isString }?.content?.toIntOrNull()?.takeIf { it in 0..255 }
        ?: throw JsonFormException("weekday must be 0 to 255")

data class ScheduleSlot(val starts: String, val ends: String, val label: String, val deep: Boolean = false) {
    fun toJson() = obj("starts" to str(starts), "ends" to str(ends), "label" to str(label), "deep" to JsonPrimitive(deep))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "schedule slot").run {
            ScheduleSlot(string("starts"), string("ends"), string("label"), bool("deep", false)).also { done() }
        }
    }
}

/** How a Device's enforcement was disabled. */
enum class BreachKind(val code: String) {
    VpnTurnedOff("vpn_turned_off"),
    AppBlockerTurnedOff("app_blocker_turned_off");

    companion object {
        fun of(code: String) = entries.firstOrNull { it.code == code }
            ?: throw JsonFormException("unknown Breach kind `$code`")
    }
}

/** A recorded event in which a Device's enforcement was disabled during a Block. No penalty follows. */
data class Breach(val id: String, val kind: BreachKind, val occurredAt: Instant) {
    fun toJson() = obj("id" to str(id), "kind" to str(kind.code), "occurred_at" to time(occurredAt))

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "breach").run {
            Breach(string("id"), BreachKind.of(string("kind")), instant("occurred_at")).also { done() }
        }
    }
}
