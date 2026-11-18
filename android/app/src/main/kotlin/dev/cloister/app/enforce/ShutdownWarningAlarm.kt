package dev.cloister.app.enforce

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import dev.cloister.app.CloisterApp
import dev.cloister.app.MainActivity
import dev.cloister.app.R
import dev.cloister.app.ui.Copy
import dev.cloister.core.ShutdownWarnings
import dev.cloister.core.State
import java.time.Duration
import java.time.Instant

/**
 * Shutdown warnings 15 and 5 minutes before the ritual is due, as notifications. The next one is
 * an alarm on the boot clock (so the phone's wall clock can't move it), rescheduled from the rule
 * state on every Enforcer tick that changes it and after a reboot; the app UI needn't be open.
 */
object ShutdownWarningAlarm {
    private const val CHANNEL = "shutdown-warnings"
    private const val NOTIFICATION_ID = 15
    @Volatile private var scheduledFor: Instant? = null

    /** Sets the alarm for the next warning (or clears it), unless it is already set for that time. */
    fun schedule(context: Context, state: State, trustedNow: Instant, force: Boolean = false) {
        val next = ShutdownWarnings.next(state, trustedNow)?.at
        if (!force && next == scheduledFor) return
        val alarms = context.getSystemService(AlarmManager::class.java) ?: return
        val intent = pendingIntent(context)
        if (next == null) {
            alarms.cancel(intent)
        } else {
            val trigger = SystemClock.elapsedRealtime() + Duration.between(trustedNow, next).toMillis().coerceAtLeast(0)
            if (Build.VERSION.SDK_INT < 31 || alarms.canScheduleExactAlarms()) {
                alarms.setExactAndAllowWhileIdle(AlarmManager.ELAPSED_REALTIME_WAKEUP, trigger, intent)
            } else {
                alarms.setWindow(AlarmManager.ELAPSED_REALTIME_WAKEUP, trigger, 60_000, intent)
            }
        }
        scheduledFor = next
    }

    /** Shows the warning whose time has come, if its ritual is still ahead. */
    fun show(context: Context, state: State, trustedNow: Instant) {
        val warning = ShutdownWarnings.current(state, trustedNow) ?: return
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Shutdown warnings", NotificationManager.IMPORTANCE_DEFAULT))
        val minutes = ShutdownWarnings.minutesLeft(warning, trustedNow)
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(Copy.shutdownWarning(minutes))
            .setContentText("The Shutdown Ritual is due at ${Copy.clock(warning.dueAt)}.")
            .setAutoCancel(true)
            .setContentIntent(
                PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
            )
            .build()
        runCatching { nm.notify(NOTIFICATION_ID, notification) } // no permission: nothing to show
    }

    private fun pendingIntent(context: Context) = PendingIntent.getBroadcast(
        context, 0, Intent(context, ShutdownWarningReceiver::class.java),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
}

/** The warning alarm went off: show it, then set the next one. */
class ShutdownWarningReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val device = CloisterApp.instance.device
        val now = device.serverTime()
        val state = device.state()
        ShutdownWarningAlarm.show(context, state, now)
        ShutdownWarningAlarm.schedule(context, state, now, force = true)
    }
}
