package dev.cloister.core

import java.time.Instant
import java.time.LocalDate
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** A request to change the state. JSON form: `{"type": "<snake_case>", ...fields}`, unknown fields are errors. */
sealed interface Command {
    fun toJson(): JsonObject

    /** Start a Block now. The Distraction List is always included; [blocklists] adds extra ones. */
    data class StartBlock(
        val blockId: String,
        val minutes: Long,
        val deep: Boolean = true,
        val blocklists: List<String> = emptyList(),
        val confirmLong: Boolean = false,
        /** The Device's own clock: recorded for diagnosis only, never trusted. */
        val clientTime: Instant? = null,
        val intent: String? = null,
        val extraDomains: List<String> = emptyList(),
        val extraApps: List<String> = emptyList(),
    ) : Command {
        override fun toJson() = obj(
            "type" to str("start_block"),
            "block_id" to str(blockId),
            "minutes" to JsonPrimitive(minutes),
            "deep" to JsonPrimitive(deep),
            "blocklists" to strArray(blocklists),
            "confirm_long" to JsonPrimitive(confirmLong),
            "client_time" to clientTime?.let(::time),
            "intent" to intent?.let(::str),
            "extra_domains" to nonEmpty(extraDomains, ::strArray),
            "extra_apps" to nonEmpty(extraApps, ::strArray),
        )
    }

    /** Move the end of an active Block later. */
    data class ExtendBlock(val blockId: String, val endsAt: Instant, val confirmLong: Boolean = false) : Command {
        override fun toJson() = obj(
            "type" to str("extend_block"), "block_id" to str(blockId), "ends_at" to time(endsAt),
            "confirm_long" to JsonPrimitive(confirmLong),
        )
    }

    /** Always rejected: a Block can't be cancelled. */
    data class CancelBlock(val blockId: String) : Command {
        override fun toJson() = obj("type" to str("cancel_block"), "block_id" to str(blockId))
    }

    /** Record a Deep Session's Depth Rating (1 to 5), once, at or after its end. */
    data class RecordDepthRating(val blockId: String, val rating: Int) : Command {
        override fun toJson() = obj(
            "type" to str("record_depth_rating"), "block_id" to str(blockId), "rating" to JsonPrimitive(rating),
        )
    }

    data class AddBlocklistEntry(val blocklist: String, val kind: EntryKind, val entry: String) : Command {
        override fun toJson() = obj(
            "type" to str("add_blocklist_entry"), "blocklist" to str(blocklist), "kind" to str(kind.code),
            "entry" to str(entry),
        )
    }

    data class RemoveBlocklistEntry(val blocklist: String, val kind: EntryKind, val entry: String) : Command {
        override fun toJson() = obj(
            "type" to str("remove_blocklist_entry"), "blocklist" to str(blocklist), "kind" to str(kind.code),
            "entry" to str(entry),
        )
    }

    data class OptOutOfBlocklist(val blocklist: String) : Command {
        override fun toJson() = obj("type" to str("opt_out_of_blocklist"), "blocklist" to str(blocklist))
    }

    data class OptInToBlocklist(val blocklist: String) : Command {
        override fun toJson() = obj("type" to str("opt_in_to_blocklist"), "blocklist" to str(blocklist))
    }

    data class StartQuitSocialBlock(val blockId: String, val confirmLong: Boolean = false) : Command {
        override fun toJson() = obj(
            "type" to str("start_quit_social_block"), "block_id" to str(blockId),
            "confirm_long" to JsonPrimitive(confirmLong),
        )
    }

    data class WriteDayPlan(
        val date: LocalDate,
        val startsAt: Instant,
        val endsAt: Instant,
        val slots: List<SlotInput>,
    ) : Command {
        override fun toJson() = obj(
            "type" to str("write_day_plan"), "date" to str(date.toString()), "starts_at" to time(startsAt),
            "ends_at" to time(endsAt), "slots" to arr(slots) { it.toJson() },
        )
    }

    data class PutSchedule(
        val scheduleId: String,
        val timeZone: String,
        val weekdays: List<Int>,
        val starts: String,
        val ends: String,
        val slots: List<ScheduleSlot>,
    ) : Command {
        override fun toJson() = obj(
            "type" to str("put_schedule"), "schedule_id" to str(scheduleId), "time_zone" to str(timeZone),
            "weekdays" to arr(weekdays) { JsonPrimitive(it) }, "starts" to str(starts), "ends" to str(ends),
            "slots" to arr(slots) { it.toJson() },
        )
    }

    companion object {
        fun fromJson(el: JsonElement): Command {
            val type = ((el as? JsonObject)?.get("type") as? JsonPrimitive)?.content
                ?: throw JsonFormException("command: missing `type`")
            return Fields(el, type).run {
                string("type")
                when (type) {
                    "start_block" -> StartBlock(
                        blockId = string("block_id"),
                        minutes = long("minutes"),
                        deep = bool("deep", true),
                        blocklists = strings("blocklists", emptyList()),
                        confirmLong = bool("confirm_long", false),
                        clientTime = instantOrNull("client_time"),
                        intent = stringOrNull("intent"),
                        extraDomains = strings("extra_domains", emptyList()),
                        extraApps = strings("extra_apps", emptyList()),
                    )
                    "extend_block" -> ExtendBlock(string("block_id"), instant("ends_at"), bool("confirm_long", false))
                    "cancel_block" -> CancelBlock(string("block_id"))
                    "record_depth_rating" -> RecordDepthRating(
                        string("block_id"),
                        long("rating").also { if (it !in 0..255) throw JsonFormException("rating must be 0 to 255") }
                            .toInt(),
                    )
                    "add_blocklist_entry" ->
                        AddBlocklistEntry(string("blocklist"), EntryKind.of(string("kind")), string("entry"))
                    "remove_blocklist_entry" ->
                        RemoveBlocklistEntry(string("blocklist"), EntryKind.of(string("kind")), string("entry"))
                    "opt_out_of_blocklist" -> OptOutOfBlocklist(string("blocklist"))
                    "opt_in_to_blocklist" -> OptInToBlocklist(string("blocklist"))
                    "start_quit_social_block" -> StartQuitSocialBlock(string("block_id"), bool("confirm_long", false))
                    "write_day_plan" -> WriteDayPlan(
                        date("date"), instant("starts_at"), instant("ends_at"),
                        objects("slots", read = SlotInput::fromJson),
                    )
                    "put_schedule" -> PutSchedule(
                        string("schedule_id"), string("time_zone"), objects("weekdays", read = ::weekday),
                        string("starts"), string("ends"), objects("slots", read = ScheduleSlot::fromJson),
                    )
                    else -> throw JsonFormException("unknown command type `$type`")
                }.also { done() }
            }
        }
    }
}

/** A Slot as written in a `write_day_plan` command. */
data class SlotInput(
    val id: String,
    val startsAt: Instant,
    val endsAt: Instant,
    val label: String,
    val deep: Boolean = false,
) {
    fun toJson() = obj(
        "id" to str(id), "starts_at" to time(startsAt), "ends_at" to time(endsAt), "label" to str(label),
        "deep" to JsonPrimitive(deep),
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "slot").run {
            SlotInput(string("id"), instant("starts_at"), instant("ends_at"), string("label"), bool("deep", false))
                .also { done() }
        }
    }
}

/** Why a command was rejected: a stable snake_case [code] shared by every engine. */
enum class Reason(val code: String) {
    InvalidLength("invalid_length"),
    BlockTooLong("block_too_long"),
    ConfirmationRequired("confirmation_required"),
    DuplicateBlock("duplicate_block"),
    UnknownBlock("unknown_block"),
    UnknownBlocklist("unknown_blocklist"),
    BlockCannotBeShortened("block_cannot_be_shortened"),
    BlockCannotBeCancelled("block_cannot_be_cancelled"),
    BlockHasEnded("block_has_ended"),
    NotADeepSession("not_a_deep_session"),
    DeepSessionNotEnded("deep_session_not_ended"),
    InvalidDepthRating("invalid_depth_rating"),
    DepthRatingAlreadyRecorded("depth_rating_already_recorded"),
    InvalidEntry("invalid_entry"),
    UnknownEntry("unknown_entry"),
    CannotOptOut("cannot_opt_out"),
    InvalidDayPlan("invalid_day_plan"),
    InvalidSlot("invalid_slot"),
    SlotsOverlap("slots_overlap"),
    DuplicateSlot("duplicate_slot"),
    DeepSessionHasStarted("deep_session_has_started"),
    SlotInThePast("slot_in_the_past"),
    SlotCannotBeRemoved("slot_cannot_be_removed"),
    InvalidSchedule("invalid_schedule"),
    UnknownTimeZone("unknown_time_zone"),
    SchedulesOverlap("schedules_overlap");

    companion object {
        /** The Reason for a code, or null for a code only the backend uses. */
        fun of(code: String) = entries.firstOrNull { it.code == code }
    }
}

/** A rule refused the command. */
class Rejection(val reason: Reason) : Exception(reason.code)

sealed interface Outcome {
    data object Accepted : Outcome
    data class Rejected(val reason: Reason) : Outcome
}

/** The Shutdown Ritual screen state. */
enum class RitualScreen(val code: String) { None("none"), Due("due"), Delayed("delayed"), Completed("completed"), Missed("missed") }

/** What a Device must enforce right now. */
data class Enforcement(
    /** Sorted, deduplicated. Each entry also blocks all of its subdomains. */
    val blockedDomains: List<String> = emptyList(),
    /** Sorted, deduplicated app identifiers. */
    val blockedApps: List<String> = emptyList(),
    val ritual: RitualScreen = RitualScreen.None,
) {
    val isEmpty get() = blockedDomains.isEmpty() && blockedApps.isEmpty()
}

data class Decision(val outcome: Outcome, val state: State, val enforcement: Enforcement)
