package dev.cloister.app.enforce

import android.app.admin.DevicePolicyManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.UserManager
import androidx.core.content.ContextCompat
import dev.cloister.core.Block
import dev.cloister.core.BreachKind
import dev.cloister.core.Enforcement
import dev.cloister.core.Lockdown
import dev.cloister.core.WorkdaySettings
import dev.cloister.core.RulesEngine
import dev.cloister.sync.Device
import java.time.Instant
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** What this phone enforces right now, and the shape of its enforcement layer. */
data class EnforcementNow(
    val enforcement: Enforcement = Enforcement(),
    /** The Block or planned Deep Session to show (Intent, time left), if any. */
    val session: Block? = null,
    /** Evening Lockdown or a Rest Day, on the clock (null without settings or during work). */
    val lockdown: Lockdown? = null,
    /** When work is next allowed, while locked down. */
    val nextWorkdayStart: Instant? = null,
    val serverTime: Instant = Instant.EPOCH,
    val vpnRunning: Boolean = false,
    val appBlockerOn: Boolean = false,
    val adminActive: Boolean = false,
    val deviceOwner: Boolean = false,
    /** The VPN was set up but is off while a Block runs: the lock overlay shows. */
    val vpnOffDuringBlock: Boolean = false,
)

/**
 * Applies the rules engine's Enforcement on this phone every couple of seconds and whenever
 * the cached state changes: the VPN's blocked domains, the app blocker's blocked apps and
 * overlays, and (Hardcore Mode) Device Owner policies. Turning the VPN or the app blocker off
 * during a Block records a Breach; nothing else follows.
 */
class Enforcer(private val context: Context, val device: Device, private val scope: CoroutineScope) {
    private val _now = MutableStateFlow(EnforcementNow())
    val now: StateFlow<EnforcementNow> = _now.asStateFlow()
    private var syncJob: Job? = null
    private var lastReportedEnforcing: Boolean? = null
    private val prefs = context.getSharedPreferences("enforcer", Context.MODE_PRIVATE)

    /** The user has turned the VPN on at least once (so it being off later is a choice). */
    var vpnWanted: Boolean
        get() = prefs.getBoolean("vpn_wanted", false)
        set(v) = prefs.edit().putBoolean("vpn_wanted", v).apply()

    fun start() {
        scope.launch { device.version.collect { tick() } }
        scope.launch {
            while (true) {
                tick()
                delay(2_000)
            }
        }
        ensureVpn()
    }

    /** Starts the VPN if the user has granted it (or the system manages it as always-on). */
    fun ensureVpn() {
        if (FilterVpn.running || VpnService.prepare(context) != null) return
        ContextCompat.startForegroundService(context, Intent(context, FilterVpn::class.java))
    }

    @Synchronized
    fun tick() {
        val serverTime = device.serverTime()
        val state = device.state()
        val enforcement = RulesEngine.enforcement(state, serverTime)
        val session = RulesEngine.activeSessions(state, serverTime)
            .sortedWith(compareBy({ it.quitSocial }, { -it.endsAt.epochSecond }))
            .firstOrNull()
        val lockdown = WorkdaySettings.lockdown(state, serverTime)
        FilterVpn.update(enforcement.blockedDomains)
        val dpm = context.getSystemService(DevicePolicyManager::class.java)
        val admin = AdminReceiver.component(context)
        val owner = dpm.isDeviceOwnerApp(context.packageName)
        val next = EnforcementNow(
            enforcement = enforcement,
            session = session,
            lockdown = lockdown,
            nextWorkdayStart = lockdown?.let { WorkdaySettings.nextWorkdayStart(state, serverTime) },
            serverTime = serverTime,
            vpnRunning = FilterVpn.running,
            appBlockerOn = AppWatcher.running,
            adminActive = dpm.isAdminActive(admin),
            deviceOwner = owner,
            vpnOffDuringBlock = session != null && vpnWanted && !FilterVpn.running,
        )
        _now.value = next
        AppWatcher.update(next)
        if (owner) applyHardcore(dpm, admin, blockRunning = session != null)
        if (device.signedIn && syncJob == null) syncJob = device.startSync(scope)
        val enforcing = next.vpnRunning && next.appBlockerOn
        if (device.signedIn && enforcing != lastReportedEnforcing) {
            lastReportedEnforcing = enforcing
            scope.launch { runCatching { device.reportEnforcing(enforcing) } }
        }
        if (!next.vpnRunning && vpnWanted) ensureVpn()
    }

    /** The system took the VPN away (the user turned it off, or another VPN started). */
    fun vpnRevoked() {
        val running = _now.value.session != null
        if (running) scope.launch { device.recordBreach(BreachKind.VpnTurnedOff) }
        tick()
    }

    /** The user turned the Accessibility service off. */
    fun appBlockerStopped() {
        if (_now.value.session != null) scope.launch { device.recordBreach(BreachKind.AppBlockerTurnedOff) }
        tick()
    }

    /**
     * Hardcore Mode (Device Owner, set up with adb): the VPN is always on with lockdown and
     * can't be reconfigured, Private DNS can't bypass it, and during a Block the app can't be
     * uninstalled.
     */
    private fun applyHardcore(dpm: DevicePolicyManager, admin: ComponentName, blockRunning: Boolean) {
        runCatching {
            if (dpm.getAlwaysOnVpnPackage(admin) != context.packageName) {
                dpm.setAlwaysOnVpnPackage(admin, context.packageName, true)
            }
            dpm.addUserRestriction(admin, UserManager.DISALLOW_CONFIG_VPN)
            if (Build.VERSION.SDK_INT >= 29) {
                dpm.addUserRestriction(admin, UserManager.DISALLOW_CONFIG_PRIVATE_DNS)
                dpm.setGlobalPrivateDnsModeOpportunistic(admin)
            }
            dpm.setPermittedAccessibilityServices(admin, null)
            if (dpm.isUninstallBlocked(admin, context.packageName) != blockRunning) {
                dpm.setUninstallBlocked(admin, context.packageName, blockRunning)
            }
        }
    }
}
