package dev.cloister.app

import android.Manifest
import android.app.admin.DevicePolicyManager
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.lifecycle.lifecycleScope
import com.google.android.libraries.identity.googleid.GetGoogleIdOption
import com.google.android.libraries.identity.googleid.GoogleIdTokenCredential
import dev.cloister.app.enforce.AdminReceiver
import dev.cloister.app.ui.App
import dev.cloister.app.ui.Actions
import io.github.jan.supabase.auth.handleDeeplinks
import java.security.MessageDigest
import java.util.UUID
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val app get() = CloisterApp.instance

    private val vpnConsent = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        if (result.resultCode == RESULT_OK) {
            app.enforcer.vpnWanted = true
            app.enforcer.ensureVpn()
        }
    }
    private val notifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    private val actions = object : Actions {
        override fun turnOnVpn() {
            val consent = VpnService.prepare(this@MainActivity)
            if (consent != null) vpnConsent.launch(consent) else {
                app.enforcer.vpnWanted = true
                app.enforcer.ensureVpn()
            }
        }

        override fun openAccessibilitySettings() {
            startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
        }

        override fun turnOnDeviceAdmin() {
            startActivity(
                Intent(DevicePolicyManager.ACTION_ADD_DEVICE_ADMIN)
                    .putExtra(DevicePolicyManager.EXTRA_DEVICE_ADMIN, AdminReceiver.component(this@MainActivity))
                    .putExtra(DevicePolicyManager.EXTRA_ADD_EXPLANATION, getString(R.string.admin_description)),
            )
        }

        override fun allowNotifications() {
            if (Build.VERSION.SDK_INT >= 33) notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }

        override val googleConfigured get() = BuildConfig.GOOGLE_SERVER_CLIENT_ID.isNotEmpty()

        override suspend fun signInWithGoogle() {
            val rawNonce = UUID.randomUUID().toString()
            val hashed = MessageDigest.getInstance("SHA-256").digest(rawNonce.toByteArray())
                .joinToString("") { "%02x".format(it) }
            val option = GetGoogleIdOption.Builder()
                .setServerClientId(BuildConfig.GOOGLE_SERVER_CLIENT_ID)
                .setFilterByAuthorizedAccounts(false)
                .setNonce(hashed)
                .build()
            val result = CredentialManager.create(this@MainActivity)
                .getCredential(this@MainActivity, GetCredentialRequest.Builder().addCredentialOption(option).build())
            val credential = GoogleIdTokenCredential.createFrom(result.credential.data)
            app.device.signInWithGoogle(credential.idToken, rawNonce)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handle(intent)
        setContent { App(app.device, app.enforcer, app.configured, actions) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onResume() {
        super.onResume()
        app.enforcer.tick()
    }

    private fun handle(intent: Intent?) {
        intent ?: return
        if (intent.action == ACTION_TURN_ON_VPN) actions.turnOnVpn()
        if (intent.data?.scheme == "cloister") {
            // Magic link: cloister://auth?code=... (PKCE).
            app.device.supabase.handleDeeplinks(intent) {
                lifecycleScope.launch { runCatching { app.device.afterSignIn() } }
            }
        }
    }

    companion object {
        const val ACTION_TURN_ON_VPN = "dev.cloister.TURN_ON_VPN"
    }
}
