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

    /** Record a Breach at server time. Recording the same id again changes nothing. */
    data class RecordBreach(val breachId: String, val kind: BreachKind, val clientTime: Instant? = null) : Command {
        override fun toJson() = obj(
            "type" to str("record_breach"), "breach_id" to str(breachId), "kind" to str(kind.code),
            "client_time" to clientTime?.let(::time),
        )
    }

    /** Set Workdays, Workday Start and Shutdown Time: what tightens applies now, what loosens tomorrow. */
    data class ChangeSettings(
        val workdays: List<Int>,
        val workdayStart: String,
        val shutdownTime: String,
        val timeZone: String? = null,
    ) : Command {
        override fun toJson() = obj(
            "type" to str("change_settings"), "workdays" to arr(workdays) { JsonPrimitive(it) },
            "workday_start" to str(workdayStart), "shutdown_time" to str(shutdownTime),
            "time_zone" to timeZone?.let(::str),
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

    /** Add a Loose End (text trimmed, 1 to 500 characters). */
    data class CaptureLooseEnd(val looseEndId: String, val text: String) : Command {
        override fun toJson() = obj("type" to str("capture_loose_end"), "loose_end_id" to str(looseEndId), "text" to str(text))
    }

    /** Close a Loose End: it leaves the list. */
    data class CloseLooseEnd(val looseEndId: String) : Command {
        override fun toJson() = obj("type" to str("close_loose_end"), "loose_end_id" to str(looseEndId))
    }

    /**
     * Postpone today's ritual and Evening Lockdown to 30 minutes after Shutdown Time, once per ISO
     * week, from Workday Start until then, while today's ritual is open.
     */
    data object UseShutdownDelay : Command {
        override fun toJson() = obj("type" to str("use_shutdown_delay"))
    }

    /**
     * Complete the due Shutdown Ritual; [typed] must be exactly "Shutdown complete". On the last
     * Workday of the week the Weekly Review is part of it: [weeklyTarget] (hours) sets next week's
     * target and is required then, refused otherwise.
     */
    data class CompleteRitual(val typed: String, val weeklyTarget: Int? = null) : Command {
        override fun toJson() = obj(
            "type" to str("complete_ritual"), "typed" to str(typed),
            "weekly_target" to weeklyTarget?.let(::JsonPrimitive),
        )
    }

    /** Set the weekly Deep Hours target (1 to 100 hours): raising applies now, lowering next week. */
    data class SetWeeklyTarget(val hours: Int) : Command {
        override fun toJson() = obj("type" to str("set_weekly_target"), "hours" to JsonPrimitive(hours))
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
                    "record_breach" ->
                        RecordBreach(string("breach_id"), BreachKind.of(string("kind")), instantOrNull("client_time"))
                    "change_settings" -> ChangeSettings(
                        objects("workdays", read = ::weekday), string("workday_start"), string("shutdown_time"),
                        stringOrNull("time_zone"),
                    )
                    "write_day_plan" -> WriteDayPlan(
                        date("date"), instant("starts_at"), instant("ends_at"),
                        objects("slots", read = SlotInput::fromJson),
                    )
                    "put_schedule" -> PutSchedule(
                        string("schedule_id"), string("time_zone"), objects("weekdays", read = ::weekday),
                        string("starts"), string("ends"), objects("slots", read = ScheduleSlot::fromJson),
                    )
                    "capture_loose_end" -> CaptureLooseEnd(string("loose_end_id"), string("text"))
                    "close_loose_end" -> CloseLooseEnd(string("loose_end_id"))
                    "use_shutdown_delay" -> UseShutdownDelay
                    "complete_ritual" -> CompleteRitual(string("typed"), longOrNull("weekly_target")?.let(::hours))
                    "set_weekly_target" -> SetWeeklyTarget(hours(long("hours")))
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

/** Hours as the Rust engine reads them (u16): anything else isn't a command. */
private fun hours(n: Long): Int =
    n.takeIf { it in 0..65535 }?.toInt() ?: throw JsonFormException("hours must be 0 to 65535")

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
    SchedulesOverlap("schedules_overlap"),
    EveningLockdown("evening_lockdown"),
    RestDay("rest_day"),
    InvalidSettings("invalid_settings"),
    TimeZoneCannotChange("time_zone_cannot_change"),
    InvalidLooseEnd("invalid_loose_end"),
    DuplicateLooseEnd("duplicate_loose_end"),
    UnknownLooseEnd("unknown_loose_end"),
    NoShutdownTime("no_shutdown_time"),
    ShutdownDelayUsed("shutdown_delay_used"),
    RitualNotDue("ritual_not_due"),
    RitualAlreadyCompleted("ritual_already_completed"),
    LooseEndRequired("loose_end_required"),
    DayPlanRequired("day_plan_required"),
    DeepSlotRequired("deep_slot_required"),
    WorkdayNotCovered("workday_not_covered"),
    ConfirmationMismatch("confirmation_mismatch"),
    InvalidWeeklyTarget("invalid_weekly_target"),
    WeeklyReviewRequired("weekly_review_required"),
    WeeklyReviewNotDue("weekly_review_not_due"),

    // Codes from the backend or an enforcement service, never from this engine.
    /** An enforcement service: a loosening goes through the account and applies tomorrow. */
    TakesEffectTomorrow("takes_effect_tomorrow"),
    /** An enforcement service: ritual completion, a Shutdown Delay or closing a Loose End go through the account. */
    AccountRequired("account_required"),
    /** The backend: a Block row can't be changed directly. */
    BlockCannotBeChanged("block_cannot_be_changed"),
    NotSignedIn("not_signed_in"),
    /** The backend couldn't read the command. */
    InvalidCommand("invalid_command");

    companion object {
        /** The Reason for a code, or null for a code this client doesn't know. */
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
