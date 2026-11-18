package dev.cloister.app.enforce

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.cloister.app.CloisterApp

/** Restarts enforcement and the Shutdown warnings after a reboot or an update (the Application starts the Enforcer). */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = CloisterApp.instance
        app.enforcer.ensureVpn()
        // Alarms don't survive a reboot: set the next Shutdown warning again.
        ShutdownWarningAlarm.schedule(context, app.device.state(), app.device.serverTime(), force = true)
    }
}
