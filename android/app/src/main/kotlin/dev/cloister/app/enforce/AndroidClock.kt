package dev.cloister.app.enforce

import android.content.ContentResolver
import android.os.SystemClock
import android.provider.Settings
import dev.cloister.core.DeviceClock
import java.time.Instant

/** The phone's boot clock and boot counter (neither can be set by the user) and its wall clock. */
class AndroidClock(private val resolver: ContentResolver) : DeviceClock {
    override fun elapsedMillis(): Long = SystemClock.elapsedRealtime()

    override fun bootCount(): Int? =
        runCatching { Settings.Global.getInt(resolver, Settings.Global.BOOT_COUNT) }.getOrNull()

    override fun wallNow(): Instant = Instant.now()
}
