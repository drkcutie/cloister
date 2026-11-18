package dev.cloister.app.ui

import android.util.Log
import dev.cloister.app.CloisterApp
import dev.cloister.core.Reason
import dev.cloister.core.WorkdaySettings
import java.time.Duration
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** Voice: say less, name the time, never "error", "sorry" or "not allowed". */
object Copy {
    private val hhmm = DateTimeFormatter.ofPattern("HH:mm")

    fun clock(t: Instant): String = hhmm.format(t.atZone(ZoneId.systemDefault()))

    /** "today", "tomorrow" or a weekday and date, relative to [now] in local time. */
    fun day(t: Instant, now: Instant): String {
        val zone = ZoneId.systemDefault()
        val d = t.atZone(zone).toLocalDate()
        val today = now.atZone(zone).toLocalDate()
        return when (d) {
            today -> "today"
            today.plusDays(1) -> "tomorrow"
            else -> DateTimeFormatter.ofPattern("EEEE d MMMM").format(d)
        }
    }

    /** "47:12" under an hour, "1:47:12" above. Fixed-width digits keep it still. */
    fun countdown(left: Duration): String {
        val s = left.seconds.coerceAtLeast(0)
        val h = s / 3600
        val m = (s % 3600) / 60
        val sec = s % 60
        return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%02d:%02d".format(m, sec)
    }

    fun minutes(m: Long): String = when {
        m < 60 -> "$m min"
        m % 60 == 0L -> "${m / 60} h"
        else -> "${m / 60} h ${m % 60} min"
    }

    /** "Shutdown in 15 minutes." */
    fun shutdownWarning(minutes: Long): String = if (minutes == 1L) "Shutdown in 1 minute." else "Shutdown in $minutes minutes."

    /** What a refusal says when the code is one this client doesn't know (the code goes to the log). */
    const val GENERIC_REFUSAL = "Not accepted. Nothing has changed."

    /**
     * Calm copy for a rejection code (fixtures/README.md). [endsAt] is the end of the Block the
     * refusal is about, when there is one. Evening Lockdown and Rest Day name the next Workday
     * Start. An unknown code gets [GENERIC_REFUSAL] and is logged, never shown.
     */
    fun reason(code: String, endsAt: Instant? = null): String {
        val reason = Reason.of(code) ?: run {
            Log.w("Cloister", "refusal with an unknown code: $code")
            return GENERIC_REFUSAL
        }
        val workdayStart = if (reason == Reason.EveningLockdown || reason == Reason.RestDay) {
            runCatching {
                val device = CloisterApp.instance.device
                val now = device.serverTime()
                WorkdaySettings.nextWorkdayStart(device.state(), now)?.let { it to now }
            }.getOrNull()
        } else null
        return reason(reason, endsAt, workdayStart?.first, workdayStart?.second ?: Instant.now())
    }

    /** The one Reason-to-copy map. Every Reason has copy: a new one won't compile until it does. */
    fun reason(reason: Reason, endsAt: Instant?, nextWorkdayStart: Instant?, now: Instant): String = when (reason) {
        Reason.InvalidLength -> "A Block needs a length of at least one minute."
        Reason.BlockTooLong -> "Blocks are at most 24 hours."
        Reason.ConfirmationRequired -> "Blocks over 4 hours need a second confirmation."
        Reason.DuplicateBlock -> "That Block has already started."
        Reason.UnknownBlock -> "That Block isn't on this account."
        Reason.UnknownBlocklist -> "That Blocklist isn't on this account."
        Reason.BlockCannotBeShortened -> "A Block can be extended, not shortened."
        Reason.TakesEffectTomorrow -> "This goes through your account and takes effect tomorrow."
        Reason.AccountRequired -> "This goes through your account. Sign in to do it."
        Reason.BlockCannotBeCancelled, Reason.BlockCannotBeChanged ->
            endsAt?.let { "Ends at ${clock(it)}. Can't be cancelled." } ?: "A Block can't be cancelled."
        Reason.BlockHasEnded -> "That Block has already ended."
        Reason.NotADeepSession -> "Only a Deep Session has a Depth Rating."
        Reason.DeepSessionNotEnded -> "The Deep Session is still running. Rate it when it ends."
        Reason.InvalidDepthRating -> "A Depth Rating is 1 to 5."
        Reason.DepthRatingAlreadyRecorded -> "This Depth Rating is already recorded."
        Reason.InvalidEntry -> "Enter a domain, like example.com, or an app name."
        Reason.UnknownEntry -> "That Blocklist doesn't hold this entry."
        Reason.CannotOptOut -> "The Work List can be edited, not switched off."
        Reason.InvalidDayPlan -> "The Day Plan runs from Workday Start to Shutdown Time."
        Reason.InvalidSlot -> "Each Slot needs a label and fits inside the Workday."
        Reason.SlotsOverlap -> "Two Slots overlap. Move one."
        Reason.DuplicateSlot -> "That Slot is already on a Day Plan."
        Reason.DeepSessionHasStarted -> "A Deep Session that has started stays as it is."
        Reason.SlotInThePast -> "A new deep Slot starts from now on."
        Reason.SlotCannotBeRemoved -> "This Slot can be moved, not removed."
        Reason.InvalidSchedule -> "A Schedule needs weekdays and a start before its end."
        Reason.UnknownTimeZone -> "That time zone isn't known."
        Reason.SchedulesOverlap -> "Another Schedule already covers one of these weekdays."
        Reason.EveningLockdown -> nextWorkdayStart
            ?.let { "Evening Lockdown. Deep Sessions start again ${day(it, now)} at ${clock(it)}." }
            ?: "Evening Lockdown. Deep Sessions start again at Workday Start."
        Reason.RestDay -> nextWorkdayStart
            ?.let { "A Rest Day. Deep Sessions start again ${day(it, now)} at ${clock(it)}." }
            ?: "A Rest Day. Deep Sessions start again on the next Workday."
        Reason.InvalidSettings -> "Workday Start comes before Shutdown Time."
        Reason.TimeZoneCannotChange -> "The time zone is set once, with the first settings."
        Reason.InvalidLooseEnd -> "A Loose End needs a few words, 500 characters at most."
        Reason.DuplicateLooseEnd -> "That Loose End is already captured."
        Reason.UnknownLooseEnd -> "That Loose End is already closed."
        Reason.NoShutdownTime -> "Set a Shutdown Time first."
        Reason.ShutdownDelayUsed -> "No Shutdown Delay left this week."
        Reason.RitualNotDue -> "No Shutdown Ritual is due."
        Reason.RitualAlreadyCompleted -> "Shutdown is already complete."
        Reason.LooseEndRequired -> "Capture at least one Loose End."
        Reason.DayPlanRequired -> "Save tomorrow's Day Plan."
        Reason.DeepSlotRequired -> "Tomorrow needs at least one deep Slot."
        Reason.WorkdayNotCovered -> "Cover the whole Workday with Slots."
        Reason.ConfirmationMismatch -> "Type \"Shutdown complete\" exactly."
        Reason.InvalidWeeklyTarget -> "A weekly target is 1 to 100 Deep Hours."
        Reason.WeeklyReviewRequired -> "Set next week's target in the Weekly Review."
        Reason.WeeklyReviewNotDue -> "The Weekly Review is on the last Workday of the week."
        Reason.NotSignedIn -> "Sign in to start a Block."
        Reason.InvalidCommand -> GENERIC_REFUSAL
    }
}
