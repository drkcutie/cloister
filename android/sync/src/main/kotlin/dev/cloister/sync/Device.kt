package dev.cloister.sync

import dev.cloister.core.BreachKind
import dev.cloister.core.Command
import dev.cloister.core.Enforcement
import dev.cloister.core.MirrorMerge
import dev.cloister.core.Reason
import dev.cloister.core.RulesEngine
import dev.cloister.core.State
import dev.cloister.core.parseInstant
import io.github.jan.supabase.SupabaseClient
import io.github.jan.supabase.auth.Auth
import io.github.jan.supabase.auth.FlowType
import io.github.jan.supabase.auth.OtpType
import io.github.jan.supabase.auth.SessionManager
import io.github.jan.supabase.auth.auth
import io.github.jan.supabase.auth.providers.Google
import io.github.jan.supabase.auth.providers.builtin.Email
import io.github.jan.supabase.auth.providers.builtin.IDToken
import io.github.jan.supabase.auth.providers.builtin.OTP
import io.github.jan.supabase.auth.user.UserSession
import io.github.jan.supabase.createSupabaseClient
import io.github.jan.supabase.postgrest.Postgrest
import io.github.jan.supabase.postgrest.postgrest
import io.github.jan.supabase.postgrest.query.filter.FilterOperator
import io.github.jan.supabase.realtime.PostgresAction
import io.github.jan.supabase.realtime.Realtime
import io.github.jan.supabase.realtime.channel
import io.github.jan.supabase.realtime.postgresChangeFlow
import io.github.jan.supabase.realtime.realtime
import java.time.Instant
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** What the backend said about a command. [reason] is a stable code; map it to calm copy. */
sealed interface CommandOutcome {
    data object Accepted : CommandOutcome
    data class Rejected(val reason: String) : CommandOutcome {
        /** The engine's Reason, or null for a backend-only code (`invalid_command`...). */
        val engineReason get() = Reason.of(reason)
    }
}

enum class SyncStatus(val label: String) { Online("Online"), Offline("Offline, still enforcing") }

/**
 * This phone as a Device on the account. Commands go to the backend (server time is
 * authoritative); the cached state in the [Mirror] is what is enforced, so enforcement keeps
 * working offline. Realtime messages are only hints: every one triggers a re-read over REST.
 */
class Device(
    val supabase: SupabaseClient,
    val mirror: Mirror,
    private val name: String,
) {
    private val pullLock = Mutex()
    private val _version = MutableStateFlow(0L)
    private val _status = MutableStateFlow(SyncStatus.Offline)
    @Volatile private var enforcing = false

    /** Bumped whenever the cached state changes; collect it to refresh UI and enforcement. */
    val version: StateFlow<Long> = _version.asStateFlow()
    val status: StateFlow<SyncStatus> = _status.asStateFlow()

    val signedIn: Boolean get() = supabase.auth.currentSessionOrNull() != null
    val email: String? get() = supabase.auth.currentUserOrNull()?.email

    fun state(): State = mirror.state
    fun serverTime(): Instant = mirror.serverTime()

    /** The Enforcement for now, by the best estimate of server time (works offline). */
    fun enforcement(): Enforcement = RulesEngine.enforcement(mirror.state, mirror.serverTime())

    // --- sign-in ---------------------------------------------------------------------------

    /** Sends the sign-in email: a magic link (opens `cloister://auth`) that also carries a code. */
    suspend fun sendMagicLink(email: String) {
        supabase.auth.signInWith(OTP, redirectUrl = REDIRECT_URL) {
            this.email = email
            createUser = true
        }
    }

    /** Signs in with the 6-digit code from the sign-in email. */
    suspend fun verifyEmailCode(email: String, code: String) {
        supabase.auth.verifyEmailOtp(type = OtpType.Email.EMAIL, email = email, token = code.trim())
        afterSignIn()
    }

    /** Signs in with a Google ID token from Credential Manager. */
    suspend fun signInWithGoogle(idToken: String, rawNonce: String?) {
        supabase.auth.signInWith(IDToken) {
            this.idToken = idToken
            provider = Google
            nonce = rawNonce
        }
        afterSignIn()
    }

    /** Email + password: only enabled on the local stack, for tests and the emulator smoke test. */
    suspend fun signInWithPassword(email: String, password: String, signUp: Boolean = false) {
        if (signUp) {
            runCatching { supabase.auth.signUpWith(Email) { this.email = email; this.password = password } }
        }
        supabase.auth.signInWith(Email) { this.email = email; this.password = password }
        afterSignIn()
    }

    /** After any sign-in (also a magic link handled by the app): register and pull. */
    suspend fun afterSignIn() {
        register()
        pull()
    }

    suspend fun signOut() {
        runCatching { supabase.auth.signOut() }
        mirror.session = null
    }

    // --- commands --------------------------------------------------------------------------

    /** Sends a command to the backend, then re-reads the account (either way the backend knows best). */
    suspend fun send(command: Command): CommandOutcome {
        val response = online {
            supabase.postgrest.rpc("command", buildJsonObject { put("cmd", command.toJson()) })
                .decodeAs<JsonObject>()
        }
        val outcome = when (response["outcome"]?.jsonPrimitive?.content) {
            "accepted" -> CommandOutcome.Accepted
            else -> CommandOutcome.Rejected(response["reason"]?.jsonPrimitive?.content ?: "invalid_command")
        }
        pull()
        return outcome
    }

    /**
     * Records a Breach now (shown at once, also offline) and sends it when it can. Resending is
     * safe: the backend records each id once.
     */
    suspend fun recordBreach(kind: BreachKind) {
        mirror.recordBreach(kind)
        bump()
        runCatching { flushBreaches() }
    }

    /** Sends the Breaches recorded while offline. */
    suspend fun flushBreaches() {
        for (cmd in mirror.pendingBreaches) {
            val outcome = send(cmd)
            if (outcome is CommandOutcome.Accepted || outcome is CommandOutcome.Rejected) {
                mirror.update { d -> d.copy(pendingBreaches = d.pendingBreaches.filterNot { it.breachId == cmd.breachId }) }
            }
        }
    }

    // --- sync ------------------------------------------------------------------------------

    /** Re-reads the account into the mirror, tighten-only. Returns whether anything changed. */
    suspend fun pull(): Boolean = pullLock.withLock {
        val incoming = online {
            State.fromJson(supabase.postgrest.rpc("state").decodeAs<JsonElement>())
        }
        val serverNow = fetchServerNow()
        val before = mirror.state
        mirror.update {
            it.copy(
                serverOffsetMillis = serverNow.toEpochMilli() - System.currentTimeMillis(),
                state = MirrorMerge.merge(it.state, incoming, serverNow),
            )
        }
        val changed = mirror.state != before
        if (changed) bump()
        changed
    }

    /** Tells the account whether this Device is enforcing; repeated by the sync loop as a heartbeat. */
    suspend fun reportEnforcing(enforcing: Boolean) {
        this.enforcing = enforcing
        register()
    }

    /**
     * Keeps the mirror current until [scope] ends: Realtime on every table that changes what is
     * enforced, a poll every 60 s, a status report every 30 s, and backoff while offline.
     */
    fun startSync(scope: CoroutineScope): Job = scope.launch {
        launch {
            while (isActive) {
                runCatching { flushBreaches(); pull() }
                delay(60.seconds)
            }
        }
        launch {
            while (isActive) {
                runCatching { register() }
                delay(30.seconds)
            }
        }
        var backoff = 1
        while (isActive) {
            val uid = supabase.auth.currentUserOrNull()?.id
            if (uid == null) { delay(5.seconds); continue }
            val channel = supabase.channel("cloister-$uid-${System.nanoTime()}")
            try {
                val flows = TABLES.map { t ->
                    channel.postgresChangeFlow<PostgresAction>(schema = "public") {
                        table = t
                        filter("user_id", FilterOperator.EQ, uid)
                    }
                }
                val listener = merge(*flows.toTypedArray())
                    .onEach { runCatching { pull() } }
                    .launchIn(this)
                channel.subscribe(blockUntilSubscribed = true)
                pull() // changes made before the subscription took hold are not pushed
                backoff = 1
                while (isActive && channel.status.value == io.github.jan.supabase.realtime.RealtimeChannel.Status.SUBSCRIBED) {
                    delay(5.seconds)
                }
                listener.cancel()
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                _status.value = SyncStatus.Offline
            } finally {
                runCatching { supabase.realtime.removeChannel(channel) }
            }
            delay((backoff * 2).coerceAtMost(60).seconds)
            backoff = (backoff * 2).coerceAtMost(30)
        }
    }

    private suspend fun register() {
        online {
            supabase.postgrest.from("devices").upsert(
                buildJsonObject {
                    put("id", mirror.deviceId)
                    put("name", name)
                    put("platform", "android")
                    put("enforcing", enforcing)
                },
            ) { onConflict = "id" }
        }
    }

    private suspend fun fetchServerNow(): Instant = online {
        val v = supabase.postgrest.rpc("server_now").decodeAs<JsonElement>()
        parseInstant((v as JsonPrimitive).content)
    }

    private suspend fun <T> online(call: suspend () -> T): T = try {
        call().also { _status.value = SyncStatus.Online }
    } catch (e: Exception) {
        if (e is kotlinx.coroutines.CancellationException) throw e
        _status.value = SyncStatus.Offline
        throw e
    }

    private fun bump() {
        _version.value = _version.value + 1
    }

    companion object {
        /** Tables whose changes alter what this Device enforces or shows. */
        val TABLES = listOf(
            "blocks", "day_plans", "schedules", "blocklist_entries", "blocklist_removals", "blocklist_opt_outs",
            "breaches", "settings", "loose_ends", "rituals", "shutdown_delays", "weekly_targets",
        )

        /** Where the magic link sends the browser back to (registered in backend config). */
        const val REDIRECT_URL = "cloister://auth"

        /** A Supabase client whose session lives in the [mirror]. */
        fun client(url: String, anonKey: String, mirror: Mirror, extraAuth: (io.github.jan.supabase.auth.AuthConfig.() -> Unit) = {}): SupabaseClient =
            createSupabaseClient(url, anonKey) {
                install(Auth) {
                    sessionManager = MirrorSessionManager(mirror)
                    flowType = FlowType.PKCE
                    scheme = "cloister"
                    host = "auth"
                    extraAuth()
                }
                install(Postgrest)
                install(Realtime)
            }
    }
}

/** Keeps the session in the mirror file (app-private storage on Android). */
private class MirrorSessionManager(private val mirror: Mirror) : SessionManager {
    private val json = Json { ignoreUnknownKeys = true }

    override suspend fun saveSession(session: UserSession) {
        mirror.session = json.encodeToString(UserSession.serializer(), session)
    }

    /** supabase-kt treats an exception here as "no saved session". */
    override suspend fun loadSession(): UserSession =
        json.decodeFromString(UserSession.serializer(), mirror.session ?: throw NoSuchElementException("no session"))

    override suspend fun deleteSession() {
        mirror.session = null
    }
}
