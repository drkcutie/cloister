package dev.cloister.app.enforce

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.cloister.app.CloisterApp

/** Restarts enforcement after a reboot or an update (the Application starts the Enforcer). */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        CloisterApp.instance.enforcer.ensureVpn()
    }
}
