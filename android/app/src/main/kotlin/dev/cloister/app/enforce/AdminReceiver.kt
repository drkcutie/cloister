package dev.cloister.app.enforce

import android.app.admin.DeviceAdminReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import dev.cloister.app.CloisterApp
import dev.cloister.app.ui.Copy

/**
 * Device Admin: while it is active the app can't be uninstalled. During a Block the app blocker
 * steps out of the deactivation screen, and with Device Owner (Hardcore Mode) uninstalling is
 * blocked outright.
 */
class AdminReceiver : DeviceAdminReceiver() {
    override fun onDisableRequested(context: Context, intent: Intent): CharSequence {
        val block = CloisterApp.instance.enforcer.now.value.block
        return if (block != null) {
            "A Block is running. Ends at ${Copy.clock(block.endsAt)}. Can't be cancelled."
        } else {
            "Without Device Admin, Cloister can be uninstalled during a Block."
        }
    }

    companion object {
        fun component(context: Context) = ComponentName(context, AdminReceiver::class.java)
    }
}
