package dev.cloister.app.enforce

import android.accessibilityservice.AccessibilityService
import android.content.Intent
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.telecom.TelecomManager
import android.util.Log
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import androidx.compose.ui.platform.ComposeView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import dev.cloister.app.CloisterApp
import dev.cloister.app.MainActivity
import dev.cloister.app.ui.BlockedAppOverlay
import dev.cloister.app.ui.LockOverlay

/**
 * The app blocker. Watches which app comes to the front and, during a Block:
 * - covers a blocked app with a calm overlay (Intent and time left);
 * - covers everything with the lock overlay while the VPN is off (emergency calls stay open);
 * - steps back out of the screens that would remove Cloister (Device Admin deactivation, its
 *   uninstall dialog, switching this service off).
 */
class AppWatcher : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())
    private var foreground: String? = null
    private var overlay: ComposeView? = null
    private var overlayKind: Kind? = null
    private var owner: OverlayOwner? = null
    private var allowOwnAppUntil = 0L

    private enum class Kind { BlockedApp, Lock }

    override fun onServiceConnected() {
        instance = this
        running = true
        CloisterApp.instance.enforcer.tick()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent) {
        if (event.eventType != AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) return
        val pkg = event.packageName?.toString() ?: return
        // Our own overlay window reports as this package; it isn't a new foreground app.
        if (pkg == packageName && overlay != null) return
        if (pkg == "com.android.systemui") return
        foreground = pkg
        guard(pkg, event.className?.toString().orEmpty())
        evaluate(last)
    }

    override fun onInterrupt() = Unit

    override fun onUnbind(intent: Intent?): Boolean {
        stop()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        stop()
        super.onDestroy()
    }

    private fun stop() {
        if (!running) return
        running = false
        instance = null
        hideOverlay()
        CloisterApp.instance.enforcer.appBlockerStopped()
    }

    /** Leaves the screens that would take Cloister off the phone while a Block runs. */
    private fun guard(pkg: String, cls: String) {
        if (last.session == null) return
        val removing = cls.contains("DeviceAdminAdd") ||
            (pkg.endsWith("packageinstaller") && windowMentions("Cloister")) ||
            (pkg == "com.android.settings" && windowMentions("Cloister app blocker"))
        if (removing) {
            Log.i(TAG, "leaving $pkg/$cls during a Block")
            performGlobalAction(GLOBAL_ACTION_BACK)
        }
    }

    private fun windowMentions(text: String) =
        runCatching { rootInActiveWindow?.findAccessibilityNodeInfosByText(text)?.isNotEmpty() == true }
            .getOrDefault(false)

    private fun evaluate(now: EnforcementNow) {
        val fg = foreground
        val ownAppAllowed = fg == packageName && SystemClock.elapsedRealtime() < allowOwnAppUntil
        when {
            now.vpnOffDuringBlock && fg !in lockAllowed() && !ownAppAllowed -> showOverlay(Kind.Lock, now)
            fg != null && fg != packageName && fg in now.enforcement.blockedApps -> showOverlay(Kind.BlockedApp, now)
            else -> hideOverlay()
        }
    }

    /** Apps the lock overlay leaves open: the VPN consent dialog and the phone. */
    private fun lockAllowed(): Set<String> {
        val dialer = runCatching { getSystemService(TelecomManager::class.java).defaultDialerPackage }.getOrNull()
        return setOfNotNull("com.android.vpndialogs", "com.android.phone", "com.android.emergency", dialer)
    }

    private fun showOverlay(kind: Kind, now: EnforcementNow) {
        if (overlayKind == kind) return
        hideOverlay()
        val o = OverlayOwner().also { owner = it }
        val view = ComposeView(this).apply {
            setViewTreeLifecycleOwner(o)
            setViewTreeSavedStateRegistryOwner(o)
            setViewTreeViewModelStoreOwner(o)
            contentDescription = if (kind == Kind.Lock) "Cloister lock overlay" else "Cloister blocked app overlay"
            setContent {
                val state = CloisterApp.instance.enforcer.now
                val current = androidx.compose.runtime.produceState(now) { state.collect { value = it } }.value
                val offset = current.serverTime.toEpochMilli() - System.currentTimeMillis()
                when (kind) {
                    Kind.BlockedApp -> BlockedAppOverlay(current.session, offset, current.lockdown, current.nextWorkdayStart) {
                        performGlobalAction(GLOBAL_ACTION_HOME)
                        hideOverlay()
                    }
                    Kind.Lock -> LockOverlay(
                        current.session, offset,
                        onTurnVpnOn = {
                            allowOwnAppUntil = SystemClock.elapsedRealtime() + 60_000
                            hideOverlay()
                            startActivity(
                                Intent(this@AppWatcher, MainActivity::class.java)
                                    .setAction(MainActivity.ACTION_TURN_ON_VPN)
                                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                            )
                        },
                        onEmergencyCall = {
                            hideOverlay()
                            startActivity(Intent(Intent.ACTION_DIAL).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                        },
                    )
                }
            }
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,
            PixelFormat.OPAQUE,
        )
        getSystemService(WindowManager::class.java).addView(view, params)
        o.resume()
        overlay = view
        overlayKind = kind
        Log.i(TAG, "overlay shown: $kind over $foreground")
    }

    private fun hideOverlay() {
        val view = overlay ?: return
        runCatching { getSystemService(WindowManager::class.java).removeView(view) }
        owner?.destroy()
        overlay = null
        overlayKind = null
        owner = null
    }

    /** Lifecycle and saved state for a ComposeView that lives outside any Activity. */
    private class OverlayOwner : LifecycleOwner, SavedStateRegistryOwner, ViewModelStoreOwner {
        private val registry = LifecycleRegistry(this)
        private val saved = SavedStateRegistryController.create(this)
        override val lifecycle: Lifecycle get() = registry
        override val savedStateRegistry: SavedStateRegistry get() = saved.savedStateRegistry
        override val viewModelStore = ViewModelStore()

        init {
            saved.performRestore(null)
            registry.currentState = Lifecycle.State.CREATED
        }

        fun resume() {
            registry.currentState = Lifecycle.State.RESUMED
        }

        fun destroy() {
            registry.currentState = Lifecycle.State.DESTROYED
            viewModelStore.clear()
        }
    }

    companion object {
        private const val TAG = "CloisterAppWatcher"
        @Volatile var running = false
            private set
        @Volatile private var instance: AppWatcher? = null
        @Volatile private var last = EnforcementNow()

        /** New Enforcement from the Enforcer (any thread). */
        fun update(now: EnforcementNow) {
            last = now
            val watcher = instance ?: return
            watcher.main.post { watcher.evaluate(now) }
        }
    }
}
