package dev.cloister.core

import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime

/**
 * Day Plans, Slots and Schedules. A deep Slot becomes a Deep Session at its start: it is never
 * stored as a Block but derived whenever Enforcement is asked for. A day without a saved Day
 * Plan falls back to its Schedule. Planned Deep Minutes never go down (the ratchet).
 */
object DayPlans {
    private val MAX_DAY: Duration = Duration.ofHours(24)

    fun plannedDeepMinutes(slots: List<Slot>) = slots.filter { it.deep }.sumOf { it.minutes }

    /** The Day Plan for [date]: the saved one, or one filled in from its Schedule. */
    fun dayPlan(state: State, date: LocalDate): DayPlan? = saved(state, date) ?: schedulePlan(state, date)

    private fun saved(state: State, date: LocalDate) = state.dayPlans.firstOrNull { it.date == date }

    /** The plan [date] gets from the Schedule covering its weekday. Slot ids: `<schedule>-<date>-<n>`. */
    fun schedulePlan(state: State, date: LocalDate): DayPlan? {
        val weekday = date.dayOfWeek.value
        val schedule = state.schedules.firstOrNull { weekday in it.weekdays } ?: return null
        return schedulePlanOf(schedule, date)
    }

    private fun schedulePlanOf(schedule: Schedule, date: LocalDate): DayPlan? {
        val zone = Blocklists.zone(schedule.timeZone) ?: return null
        fun at(hhmm: String) = parseHhmm(hhmm)?.let { local(zone, date, it) }
        val slots = schedule.slots.mapIndexed { i, s ->
            Slot(
                id = "${schedule.id}-$date-${i + 1}",
                startsAt = at(s.starts) ?: return null,
                endsAt = at(s.ends) ?: return null,
                label = s.label,
                deep = s.deep,
            )
        }
        return DayPlan(
            date = date,
            startsAt = at(schedule.starts) ?: return null,
            endsAt = at(schedule.ends) ?: return null,
            plannedDeepMinutes = plannedDeepMinutes(slots),
            slots = slots,
        )
    }

    /** Local wall time to an instant: an ambiguous time takes the earlier one, a skipped time moves later. */
    private fun local(zone: ZoneId, date: LocalDate, time: LocalTime): Instant =
        ZonedDateTime.ofLocal(date.atTime(time), zone, null).withEarlierOffsetAtOverlap().toInstant()

    /** Strict "HH:MM", 00:00 to 23:59. */
    internal fun parseHhmm(s: String): LocalTime? {
        if (s.length != 5 || s[2] != ':' || !s.withIndex().all { (i, c) -> i == 2 || c in '0'..'9' }) return null
        val h = s.substring(0, 2).toInt()
        val m = s.substring(3).toInt()
        return if (h <= 23 && m <= 59) LocalTime.of(h, m) else null
    }

    private fun planInForce(state: State, date: LocalDate, now: Instant): DayPlan? =
        saved(state, date) ?: schedulePlan(state, date)?.takeIf { !it.startsAt.isAfter(now) }

    /** The Deep Session a deep Slot becomes. */
    fun deepSession(slot: Slot, blocklists: List<String>) = Block(
        id = slot.id,
        startsAt = slot.startsAt,
        endsAt = slot.endsAt,
        blocklists = blocklists,
        deep = true,
        intent = slot.label,
        depthRating = slot.depthRating,
    )

    /**
     * Every Deep Session that deep Slots become around [now]: the deep Slots of saved Day Plans,
     * plus those of the Schedule-filled plan of the current day in each Schedule's time zone.
     */
    fun plannedDeepSessions(state: State, now: Instant): List<Block> {
        val slots = state.dayPlans.flatMap { it.slots }.toMutableList()
        val dates = state.schedules.mapNotNull { Blocklists.zone(it.timeZone) }
            .map { now.atZone(it).toLocalDate() }.toSortedSet()
        for (date in dates) {
            if (saved(state, date) == null) schedulePlan(state, date)?.let { slots += it.slots }
        }
        return slots.filter { it.deep }
            .map { deepSession(it, Blocklists.distractionIdsInForce(state, it.startsAt)) }
            .sortedWith(compareBy({ it.startsAt }, { it.id }))
    }

    private fun Slot.sameAs(s: SlotInput) =
        id == s.id && startsAt == s.startsAt && endsAt == s.endsAt && label == s.label.trim() && deep == s.deep

    internal fun writeDayPlan(
        state: State,
        date: LocalDate,
        startsAt: Instant,
        endsAt: Instant,
        input: List<SlotInput>,
        now: Instant,
    ): State {
        if (!endsAt.isAfter(startsAt) || Duration.between(startsAt, endsAt) > MAX_DAY) {
            throw Rejection(Reason.InvalidDayPlan)
        }
        for (s in input) {
            if (s.label.isBlank() || !s.endsAt.isAfter(s.startsAt) || s.startsAt.isBefore(startsAt) ||
                s.endsAt.isAfter(endsAt)
            ) throw Rejection(Reason.InvalidSlot)
        }
        val sorted = input.sortedWith(compareBy({ it.startsAt }, { it.id }))
        if (sorted.zipWithNext().any { (a, b) -> b.startsAt.isBefore(a.endsAt) }) throw Rejection(Reason.SlotsOverlap)
        val ids = mutableSetOf<String>()
        for (s in sorted) {
            val elsewhere = state.dayPlans.filter { it.date != date }.any { p -> p.slots.any { it.id == s.id } } ||
                state.blocks.any { it.id == s.id }
            if (!ids.add(s.id) || elsewhere) throw Rejection(Reason.DuplicateSlot)
        }
        val old = planInForce(state, date, now)
        val oldSlots = old?.slots.orEmpty()
        fun unchanged(s: SlotInput) = oldSlots.firstOrNull { it.sameAs(s) }
        for (o in oldSlots.filter { it.deep && !it.startsAt.isAfter(now) }) {
            if (sorted.none { o.sameAs(it) }) throw Rejection(Reason.DeepSessionHasStarted)
        }
        if (sorted.any { s -> s.deep && s.startsAt.isBefore(now) && unchanged(s)?.deep != true }) {
            throw Rejection(Reason.SlotInThePast)
        }
        val slots = sorted.map { s ->
            Slot(s.id, s.startsAt, s.endsAt, s.label.trim(), s.deep, unchanged(s)?.depthRating)
        }
        val minutes = plannedDeepMinutes(slots)
        if (minutes < (old?.plannedDeepMinutes ?: 0)) throw Rejection(Reason.SlotCannotBeRemoved)
        val plan = DayPlan(date, startsAt, endsAt, minutes, slots)
        val plans = state.dayPlans.filter { it.date != date } + plan
        return state.copy(dayPlans = plans.sortedBy { it.date })
    }

    internal fun putSchedule(state: State, cmd: Command.PutSchedule, now: Instant): State {
        Blocklists.zone(cmd.timeZone) ?: throw Rejection(Reason.UnknownTimeZone)
        val days = cmd.weekdays.toSortedSet()
        val dayStarts = parseHhmm(cmd.starts)
        val dayEnds = parseHhmm(cmd.ends)
        if (dayStarts == null || dayEnds == null || days.size != cmd.weekdays.size ||
            days.any { it !in 1..7 } || !dayEnds.isAfter(dayStarts)
        ) throw Rejection(Reason.InvalidSchedule)
        val slots = cmd.slots.map { s ->
            val a = parseHhmm(s.starts)
            val b = parseHhmm(s.ends)
            if (a == null || b == null || s.label.isBlank() || !b.isAfter(a) || a.isBefore(dayStarts) || b.isAfter(dayEnds)) {
                throw Rejection(Reason.InvalidSlot)
            }
            a to s
        }.sortedBy { it.first } // stable, like Rust's sort_by_key
        if (slots.zipWithNext().any { (x, y) -> y.first.isBefore(parseHhmm(x.second.ends)) }) {
            throw Rejection(Reason.SlotsOverlap)
        }
        if (state.schedules.any { it.id != cmd.scheduleId && it.weekdays.any(days::contains) }) {
            throw Rejection(Reason.SchedulesOverlap)
        }
        var dayPlans = state.dayPlans
        // A day already running on the old Schedule keeps its plan, saved as that day's Day Plan.
        state.schedules.firstOrNull { it.id == cmd.scheduleId }?.let { old ->
            val zone = Blocklists.zone(old.timeZone) ?: return@let
            val today = now.atZone(zone).toLocalDate()
            if (today.dayOfWeek.value in old.weekdays && saved(state, today) == null) {
                schedulePlanOf(old, today)?.takeIf { !it.startsAt.isAfter(now) && now.isBefore(it.endsAt) }?.let {
                    dayPlans = (dayPlans + it).sortedBy { p -> p.date }
                }
            }
        }
        val schedule = Schedule(
            id = cmd.scheduleId,
            timeZone = cmd.timeZone,
            weekdays = days.toList(),
            starts = cmd.starts,
            ends = cmd.ends,
            slots = slots.map { (_, s) -> ScheduleSlot(s.starts, s.ends, s.label.trim(), s.deep) },
        )
        val schedules = (state.schedules.filter { it.id != cmd.scheduleId } + schedule).sortedBy { it.id }
        return state.copy(dayPlans = dayPlans, schedules = schedules)
    }
}
