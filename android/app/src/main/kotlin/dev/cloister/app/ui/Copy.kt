package dev.cloister.app.ui

import java.time.Duration
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** Voice: say less, name the time, never "error", "sorry" or "not allowed". */
object Copy {
    private val hhmm = DateTimeFormatter.ofPattern("HH:mm")

    fun clock(t: Instant): String = hhmm.format(t.atZone(ZoneId.systemDefault()))

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

    /** Calm copy for a rejection code (fixtures/README.md); [endsAt] names the time where it helps. */
    fun reason(code: String, endsAt: Instant? = null): String = when (code) {
        "invalid_length" -> "A Block needs a length of at least one minute."
        "block_too_long" -> "Blocks are at most 24 hours."
        "confirmation_required" -> "Blocks over 4 hours need a second confirmation."
        "block_cannot_be_shortened" -> "A Block can be extended, not shortened."
        "block_cannot_be_cancelled", "loosening_not_accepted" ->
            endsAt?.let { "Ends at ${clock(it)}. Can't be cancelled." } ?: "A Block can't be cancelled."
        "block_has_ended" -> "That Block has already ended."
        "duplicate_block" -> "That Block has already started."
        "not_a_deep_session" -> "Only a Deep Session has a Depth Rating."
        "deep_session_not_ended" -> "The Deep Session is still running. Rate it when it ends."
        "invalid_depth_rating" -> "A Depth Rating is 1 to 5."
        "depth_rating_already_recorded" -> "This Depth Rating is already recorded."
        "not_signed_in" -> "Sign in to start a Block."
        else -> "The backend kept things as they are ($code)."
    }
}
