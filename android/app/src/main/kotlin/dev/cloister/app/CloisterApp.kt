package dev.cloister.app

import android.app.Application
import android.os.Build
import dev.cloister.app.enforce.Enforcer
import dev.cloister.sync.Device
import dev.cloister.sync.Mirror
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** Holds the one [Device] and starts enforcement for the life of the process. */
class CloisterApp : Application() {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    lateinit var device: Device
        private set
    lateinit var enforcer: Enforcer
        private set

    /** False when the build has no backend key (the app still enforces its cached state). */
    val configured get() = BuildConfig.ANON_KEY.isNotEmpty()

    override fun onCreate() {
        super.onCreate()
        instance = this
        val mirror = Mirror(File(filesDir, "mirror.json"))
        val url = BuildConfig.API_URL
        val key = BuildConfig.ANON_KEY.ifEmpty { "not-configured" }
        device = Device(Device.client(url, key, mirror), mirror, Build.MODEL ?: "Android phone")
        enforcer = Enforcer(this, device, scope)
        enforcer.start()
    }

    companion object {
        lateinit var instance: CloisterApp
            private set
    }
}
