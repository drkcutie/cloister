package dev.cloister.app.devtools

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import dev.cloister.app.CloisterApp
import kotlinx.coroutines.launch

/**
 * Debug builds only, for the emulator smoke test (android/scripts/smoke-test.sh):
 *   adb shell am broadcast -n dev.cloister/.devtools.DebugReceiver -a dev.cloister.DEBUG_SIGN_IN --es email E --es password P
 *   adb shell am broadcast -n dev.cloister/.devtools.DebugReceiver -a dev.cloister.DEBUG_SYNC
 * Sign-in uses the local stack's email + password provider (no email round trip).
 */
class DebugReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = CloisterApp.instance
        val pending = goAsync()
        app.scope.launch {
            val result = runCatching {
                when (intent.action) {
                    "dev.cloister.DEBUG_SIGN_IN" -> app.device.signInWithPassword(
                        intent.getStringExtra("email")!!, intent.getStringExtra("password")!!, signUp = true,
                    )
                    "dev.cloister.DEBUG_SYNC" -> app.device.pull()
                }
                app.enforcer.tick()
            }
            Log.i("CloisterDebug", "${intent.action}: ${result.exceptionOrNull() ?: "ok"} blocked=${app.enforcer.now.value.enforcement.blockedDomains.size}")
            pending.finish()
        }
    }
}
