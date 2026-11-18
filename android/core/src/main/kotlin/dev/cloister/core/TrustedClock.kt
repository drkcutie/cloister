package dev.cloister.core

import java.time.Duration
import java.time.Instant

/** The phone's clocks: time since boot (never set by the user), a boot counter, and the wall clock. */
interface DeviceClock {
    /** Milliseconds since boot, including deep sleep (Android: `SystemClock.elapsedRealtime`). */
    fun elapsedMillis(): Long

    /** Which boot this is, when the platform says (Android: `Settings.Global.BOOT_COUNT`). */
    fun bootCount(): Int?

    /** The wall clock. The user can set it, so it is never trusted for "now". */
    fun wallNow(): Instant
}

/** A [DeviceClock] for plain JVMs and tests: time since this process started, no boot counter. */
object JvmDeviceClock : DeviceClock {
    private val origin = System.nanoTime()
    override fun elapsedMillis() = (System.nanoTime() - origin) / 1_000_000
    override fun bootCount(): Int? = null
    override fun wallNow(): Instant = Instant.now()
}

/**
 * A known server time [trustedAt], seen when the boot clock read [elapsedAt] during boot [boot].
 * Trusted now runs on from it with the boot clock, which the user can't set.
 */
data class ClockAnchor(val trustedAt: Instant, val elapsedAt: Long, val boot: Int? = null)

/**
 * Server-anchored, monotonic time. The wall clock never moves trusted now: setting the phone's
 * clock forward can't end a Block early, setting it back can't lift Evening Lockdown.
 */
object TrustedClock {
    /** How often a running Device re-anchors (so a reboot loses at most this much). */
    val CHECKPOINT_EVERY: Duration = Duration.ofSeconds(30)

    /** Whether the phone has rebooted since [anchor] was taken. */
    fun rebooted(anchor: ClockAnchor, elapsedNow: Long, bootNow: Int?): Boolean =
        elapsedNow < anchor.elapsedAt || (anchor.boot != null && bootNow != null && anchor.boot != bootNow)

    /**
     * Trusted now: the anchor plus the boot-clock time since it. After a reboot the time since the
     * anchor is unknown, but at least the uptime since this boot has passed.
     */
    fun now(anchor: ClockAnchor, elapsedNow: Long, bootNow: Int? = null): Instant =
        if (rebooted(anchor, elapsedNow, bootNow)) anchor.trustedAt.plusMillis(elapsedNow.coerceAtLeast(0))
        else anchor.trustedAt.plusMillis(elapsedNow - anchor.elapsedAt)

    /** Trusted now by [clock]. */
    fun now(anchor: ClockAnchor, clock: DeviceClock): Instant = now(anchor, clock.elapsedMillis(), clock.bootCount())

    /** An anchor at a server time just read. */
    fun anchor(serverNow: Instant, clock: DeviceClock) = ClockAnchor(serverNow, clock.elapsedMillis(), clock.bootCount())

    /**
     * The anchor to keep: [anchor] moved up to trusted now when it is older than [CHECKPOINT_EVERY]
     * or from an earlier boot (after a reboot it must be re-anchored at once, before the boot clock
     * passes the old reading). Returns [anchor] itself when nothing needs saving.
     */
    fun checkpoint(anchor: ClockAnchor, elapsedNow: Long, bootNow: Int? = null): ClockAnchor {
        val stale = elapsedNow - anchor.elapsedAt >= CHECKPOINT_EVERY.toMillis()
        if (!rebooted(anchor, elapsedNow, bootNow) && !stale) return anchor
        return ClockAnchor(now(anchor, elapsedNow, bootNow), elapsedNow, bootNow ?: anchor.boot)
    }

    /**
     * What to enforce: the Enforcement at [trusted], and also at [wall] when the wall clock is
     * later (a phone that was off for hours still enters Evening Lockdown). The union of both, so
     * moving the wall clock can only add to what is enforced.
     */
    fun enforcement(state: State, trusted: Instant, wall: Instant): Enforcement {
        val atTrusted = RulesEngine.enforcement(state, trusted)
        if (!wall.isAfter(trusted)) return atTrusted
        val atWall = RulesEngine.enforcement(state, wall)
        val ritual = if (atWall.ritual == RitualScreen.Due) RitualScreen.Due else atTrusted.ritual
        return Enforcement(
            blockedDomains = (atTrusted.blockedDomains + atWall.blockedDomains).toSortedSet().toList(),
            blockedApps = (atTrusted.blockedApps + atWall.blockedApps).toSortedSet().toList(),
            ritual = ritual,
        )
    }

    /** Evening Lockdown or a Rest Day at [trusted], or at [wall] when that is later. */
    fun lockdown(state: State, trusted: Instant, wall: Instant): Lockdown? =
        WorkdaySettings.lockdown(state, trusted)
            ?: if (wall.isAfter(trusted)) WorkdaySettings.lockdown(state, wall) else null
}
