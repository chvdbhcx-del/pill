package app.pill.phone.data

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import app.pill.phone.protocol.RelayCfg
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.UUID

/** Everything needed to talk to the paired PC. The token never leaves the encrypted store. */
data class Paired(
    val host: String, val port: Int, val token: String, val pcName: String, val deviceId: String,
    /** Keys and address of the internet relay (protocol v2); null for a PC that cannot do it: the phone then works on the LAN only. */
    val relay: RelayCfg? = null,
) {
    val address: String get() = "$host:$port"
}

/**
 * Persistent state. The pairing (with the bearer token) lives in EncryptedSharedPreferences backed by the
 * Android Keystore; plain settings live in ordinary preferences.
 */
object Store {
    private val RELAY_KEYS = listOf("url", "room", "join", "kp2c", "kc2p", "sk", "pcpk")
    private lateinit var plain: SharedPreferences
    private lateinit var secure: SharedPreferences

    private val _paired = MutableStateFlow<Paired?>(null)
    val paired: StateFlow<Paired?> = _paired.asStateFlow()

    private val _onboarded = MutableStateFlow(false)
    val onboarded: StateFlow<Boolean> = _onboarded.asStateFlow()

    private val _autoReceive = MutableStateFlow(false)
    val autoReceive: StateFlow<Boolean> = _autoReceive.asStateFlow()

    private val _includeOngoing = MutableStateFlow(false)
    val includeOngoing: StateFlow<Boolean> = _includeOngoing.asStateFlow()

    private val _relayEnabled = MutableStateFlow(true)
    /** "Use internet relay when away from home". */
    val relayEnabled: StateFlow<Boolean> = _relayEnabled.asStateFlow()

    private val _overrides = MutableStateFlow<Map<String, Boolean>>(emptyMap())
    val overrides: StateFlow<Map<String, Boolean>> = _overrides.asStateFlow()

    fun init(ctx: Context) {
        val app = ctx.applicationContext
        plain = app.getSharedPreferences("pill", Context.MODE_PRIVATE)
        secure = openSecure(app)
        _onboarded.value = plain.getBoolean("onboarded", false)
        _autoReceive.value = plain.getBoolean("autoReceive", false)
        _includeOngoing.value = plain.getBoolean("includeOngoing", false)
        _relayEnabled.value = plain.getBoolean("relayEnabled", true)
        _overrides.value = plain.getStringSet("filterOn", emptySet()).orEmpty().associateWith { true } +
            plain.getStringSet("filterOff", emptySet()).orEmpty().associateWith { false }
        _paired.value = readPaired()
    }

    private fun openSecure(app: Context): SharedPreferences = try {
        val key = MasterKey.Builder(app).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build()
        EncryptedSharedPreferences.create(
            app, "pill_secure", key,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    } catch (_: Exception) {
        // A broken Keystore (rare, some restored-backup devices): start from a clean store and pair again.
        app.deleteSharedPreferences("pill_secure")
        EncryptedSharedPreferences.create(
            app, "pill_secure", MasterKey.Builder(app).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    /** One stable id per install, kept across re-pairing so the PC replaces the token instead of adding a device. */
    val deviceId: String
        get() = secure.getString("deviceId", null) ?: UUID.randomUUID().toString().also { secure.edit().putString("deviceId", it).apply() }

    private fun readPaired(): Paired? {
        val host = secure.getString("host", null) ?: return null
        val token = secure.getString("token", null) ?: return null
        return Paired(host, secure.getInt("port", 0), token, secure.getString("pcName", "") ?: "", deviceId, readRelay())
    }

    private fun readRelay(): RelayCfg? {
        fun s(k: String) = secure.getString("relay.$k", null)
        return RelayCfg(s("url") ?: return null, s("room") ?: return null, s("join") ?: return null, s("kp2c") ?: return null,
            s("kc2p") ?: return null, s("sk") ?: return null, s("pcpk") ?: return null)
    }

    private fun SharedPreferences.Editor.putRelay(r: RelayCfg?) = apply {
        for (k in RELAY_KEYS) remove("relay.$k")
        if (r != null) {
            putString("relay.url", r.url).putString("relay.room", r.room).putString("relay.join", r.join)
            putString("relay.kp2c", r.kP2c).putString("relay.kc2p", r.kC2p).putString("relay.sk", r.secret).putString("relay.pcpk", r.pcPublic)
        }
    }

    /** Update the PC's address or name; the relay keys stay as they are. */
    fun savePairing(host: String, port: Int, token: String, pcName: String) {
        secure.edit().putString("host", host).putInt("port", port).putString("token", token).putString("pcName", pcName).apply()
        _paired.value = readPaired()
    }

    /** A fresh pairing: the PC and (in the same write) the relay keys derived with it, or none for a PC without relay support. */
    fun savePaired(host: String, port: Int, token: String, pcName: String, relay: RelayCfg?) {
        secure.edit().putString("host", host).putInt("port", port).putString("token", token).putString("pcName", pcName).putRelay(relay).apply()
        _paired.value = readPaired()
    }

    /** Forget the PC (unpair, or the PC revoked us). The device id stays. */
    fun clearPairing() {
        secure.edit().remove("host").remove("port").remove("token").remove("pcName").putRelay(null).apply()
        _paired.value = null
    }

    fun setOnboarded() { plain.edit().putBoolean("onboarded", true).apply(); _onboarded.value = true }
    fun setAutoReceive(on: Boolean) { plain.edit().putBoolean("autoReceive", on).apply(); _autoReceive.value = on }
    fun setRelayEnabled(on: Boolean) { plain.edit().putBoolean("relayEnabled", on).apply(); _relayEnabled.value = on }
    fun setIncludeOngoing(on: Boolean) { plain.edit().putBoolean("includeOngoing", on).apply(); _includeOngoing.value = on }

    fun setAppEnabled(pkg: String, on: Boolean) {
        val next = _overrides.value + (pkg to on)
        _overrides.value = next
        plain.edit()
            .putStringSet("filterOn", next.filterValues { it }.keys)
            .putStringSet("filterOff", next.filterValues { !it }.keys)
            .apply()
    }
}
