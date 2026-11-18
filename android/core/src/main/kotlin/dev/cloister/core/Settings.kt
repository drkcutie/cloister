package dev.cloister.core

import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneOffset
import java.time.ZonedDateTime
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

/** Workdays (ISO, sorted), Workday Start and Shutdown Time ("HH:MM", local to the state's time zone). */
data class Settings(val workdays: List<Int>, val workdayStart: String, val shutdownTime: String) {
    /**
     * The stricter of two settings, field by field (strings compare like times: "HH:MM"): Workdays
     * in both, the later Workday Start and the earlier Shutdown Time. If those times would leave no
     * working time, this one's times are kept, so a Workday always has a ritual that lapses.
     */
    fun tightest(other: Settings): Settings {
        val start = maxOf(workdayStart, other.workdayStart)
        val shutdown = minOf(shutdownTime, other.shutdownTime)
        val (s, e) = if (start < shutdown) start to shutdown else workdayStart to shutdownTime
        return Settings(workdays = workdays.filter { it in other.workdays }, workdayStart = s, shutdownTime = e)
    }

    fun isWorkday(date: LocalDate) = date.dayOfWeek.value in workdays

    /** A broken value locks the whole day rather than freeing it. */
    internal fun times(): Pair<LocalTime, LocalTime> =
        (DayPlans.parseHhmm(workdayStart) ?: LocalTime.MIDNIGHT) to (DayPlans.parseHhmm(shutdownTime) ?: LocalTime.MIDNIGHT)

    fun toJson() = obj(
        "workdays" to arr(workdays) { JsonPrimitive(it) },
        "workday_start" to str(workdayStart),
        "shutdown_time" to str(shutdownTime),
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "settings").run {
            Settings(objects("workdays", read = ::weekday), string("workday_start"), string("shutdown_time")).also { done() }
        }
    }
}

/** Requested settings that loosen something, in force from [effectiveAt]. */
data class PendingSettings(val settings: Settings, val effectiveAt: Instant) {
    fun toJson() = obj(
        "workdays" to arr(settings.workdays) { JsonPrimitive(it) },
        "workday_start" to str(settings.workdayStart),
        "shutdown_time" to str(settings.shutdownTime),
        "effective_at" to time(effectiveAt),
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "pending settings").run {
            PendingSettings(
                Settings(objects("workdays", read = ::weekday), string("workday_start"), string("shutdown_time")),
                instant("effective_at"),
            ).also { done() }
        }
    }
}

/** Settings that were in force until [until] (the history [WorkdaySettings.workday] reads for days that have begun). */
data class PastSettings(val settings: Settings, val until: Instant) {
    fun toJson() = obj(
        "workdays" to arr(settings.workdays) { JsonPrimitive(it) },
        "workday_start" to str(settings.workdayStart),
        "shutdown_time" to str(settings.shutdownTime),
        "until" to time(until),
    )

    companion object {
        fun fromJson(el: JsonElement) = Fields(el, "past settings").run {
            PastSettings(
                Settings(objects("workdays", read = ::weekday), string("workday_start"), string("shutdown_time")),
                instant("until"),
            ).also { done() }
        }
    }
}

/** Why the clock says no work now. */
enum class Lockdown(val reason: Reason) { EveningLockdown(Reason.EveningLockdown), RestDay(Reason.RestDay) }

/** What the settings say about a date. */
sealed interface Workday {
    data object NotConfigured : Workday
    data object RestDay : Workday
    data class Bounds(val start: Instant, val end: Instant) : Workday
}

/**
 * Workdays, Evening Lockdown and Rest Days (the Kotlin twin of `cloister-core/src/settings.rs`).
 * Evening Lockdown runs on the clock alone; settings follow tighten now, loosen tomorrow.
 */
object WorkdaySettings {
    private fun zone(state: State) = Blocklists.zone(state.timeZone) ?: ZoneOffset.UTC

    /** The settings in force at [at]: from the history before the last change, else the pending ones once their day has come. */
    fun settingsAt(state: State, at: Instant): Settings? {
        state.pastSettings.firstOrNull { at.isBefore(it.until) }?.let { return it.settings }
        val p = state.pendingSettings
        return if (p != null && !p.effectiveAt.isAfter(at)) p.settings else state.settings
    }

    /** Whether the clock locks work at [at], and why. Null before any settings exist. */
    fun lockdown(state: State, at: Instant): Lockdown? {
        val settings = settingsAt(state, at) ?: return null
        val local = at.atZone(zone(state))
        if (!settings.isWorkday(local.toLocalDate())) return Lockdown.RestDay
        val (start, shutdown) = settings.times()
        val t = local.toLocalTime()
        val afterShutdown = !t.isBefore(shutdown) && !Rituals.delayHoldsLockdown(state, local.toLocalDateTime(), shutdown)
        return if (t.isBefore(start) || afterShutdown) Lockdown.EveningLockdown else null
    }

    private fun local(state: State, date: LocalDate, time: LocalTime): Instant =
        ZonedDateTime.ofLocal(date.atTime(time), zone(state), null).withEarlierOffsetAtOverlap().toInstant()

    /**
     * The Workday bounds of [date]. A day still to come uses the settings in force when it begins.
     * A day that has begun keeps the Workday it was when it began; today takes the settings in
     * force now if they keep it a Workday.
     */
    fun workday(state: State, date: LocalDate, now: Instant): Workday {
        val begins = local(state, date, LocalTime.MIDNIGHT)
        val current = settingsAt(state, maxOf(now, begins)) ?: return Workday.NotConfigured
        val today = now.atZone(zone(state)).toLocalDate() == date
        val settings = if (begins.isAfter(now) || (today && current.isWorkday(date))) current
        else settingsAt(state, begins) ?: current
        if (!settings.isWorkday(date)) return Workday.RestDay
        val (start, shutdown) = settings.times()
        return Workday.Bounds(local(state, date, start), local(state, date, shutdown))
    }

    /** When work is next allowed after [now]; null without settings or with no Workday in two weeks. */
    fun nextWorkdayStart(state: State, now: Instant): Instant? {
        val today = now.atZone(zone(state)).toLocalDate()
        return (0L until 15L).asSequence().map { today.plusDays(it) }.firstNotNullOfOrNull { date ->
            (workday(state, date, now) as? Workday.Bounds)?.takeIf { it.start < it.end && it.start > now }?.start
        }
    }

    private fun checked(workdays: List<Int>, start: String, shutdown: String): Settings {
        val days = workdays.toSortedSet()
        val a = DayPlans.parseHhmm(start)
        val b = DayPlans.parseHhmm(shutdown)
        if (a == null || b == null || days.size != workdays.size || days.any { it !in 1..7 } || !b.isAfter(a)) {
            throw Rejection(Reason.InvalidSettings)
        }
        return Settings(days.toList(), start, shutdown)
    }

    internal fun changeSettings(state: State, cmd: Command.ChangeSettings, now: Instant): State {
        cmd.timeZone?.let { Blocklists.zone(it) ?: throw Rejection(Reason.UnknownTimeZone) }
        val requested = checked(cmd.workdays, cmd.workdayStart, cmd.shutdownTime)
        val configured = state.settings != null || state.pendingSettings != null
        var next = state
        if (cmd.timeZone != null && cmd.timeZone != state.timeZone) {
            if (configured) throw Rejection(Reason.TimeZoneCannotChange)
            next = next.copy(timeZone = cmd.timeZone)
        }
        val base = settingsAt(next, now)
            ?: return next.copy(settings = requested, pendingSettings = null, settingsSince = next.settingsSince ?: now)
        val inForce = base.tightest(requested)
        next = recordSettingsChange(next, inForce, now)
        val pending = if (inForce != requested) PendingSettings(requested, Blocklists.nextDayBoundary(next, now)) else null
        return next.copy(settings = inForce, pendingSettings = pending)
    }

    /** `State::record_settings_change`: keep the settings history before [inForce] replaces the settings at [now]. */
    private fun recordSettingsChange(state: State, inForce: Settings, now: Instant): State {
        val past = state.pastSettings.toMutableList()
        fun push(s: Settings, until: Instant) {
            if (past.lastOrNull()?.let { !it.until.isBefore(until) } == true) return
            past += PastSettings(s, until)
        }
        val old = state.settings
        val p = state.pendingSettings
        if (old != null && p != null && !p.effectiveAt.isAfter(now) && old != p.settings) push(old, p.effectiveAt)
        val withPast = state.copy(pastSettings = past.toList())
        settingsAt(withPast, now)?.let { base -> if (base != inForce) push(base, now) }
        return state.copy(pastSettings = past.toList())
    }
}
