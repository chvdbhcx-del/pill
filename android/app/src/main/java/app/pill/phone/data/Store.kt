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

/**
 * Persistent state. The pairings (with their bearer tokens and relay keys) live in EncryptedSharedPreferences backed by the
 * Android Keystore; plain settings live in ordinary preferences.
 */
object Store {
    private val LEGACY_RELAY_KEYS = listOf("url", "room", "join", "kp2c", "kc2p", "sk", "pcpk")
    private lateinit var plain: SharedPreferences
    private lateinit var secure: SharedPreferences
    private val lock = Any()

    private val _pcs = MutableStateFlow<List<Paired>>(emptyList())
    /** Every paired PC, in the order they were added. */
    val pcs: StateFlow<List<Paired>> = _pcs.asStateFlow()

    private val _selectedPc = MutableStateFlow<String?>(null)
    /** The PC the Home hero describes (only matters with two or more). */
    val selectedPc: StateFlow<String?> = _selectedPc.asStateFlow()

    private val _onboarded = MutableStateFlow(false)
    val onboarded: StateFlow<Boolean> = _onboarded.asStateFlow()

    private val _autoReceive = MutableStateFlow(false)
    /** The global "Auto-receive files"; a PC can override it for itself. */
    val autoReceive: StateFlow<Boolean> = _autoReceive.asStateFlow()

    private val _phoneExpanded = MutableStateFlow(false)
    /** The "This phone" row on Home is open (details shown). Remembered across launches. */
    val phoneExpanded: StateFlow<Boolean> = _phoneExpanded.asStateFlow()

    private val _includeOngoing = MutableStateFlow(false)
    val includeOngoing: StateFlow<Boolean> = _includeOngoing.asStateFlow()

    private val _relayEnabled = MutableStateFlow(true)
    /** "Use internet relay when away from home". */
    val relayEnabled: StateFlow<Boolean> = _relayEnabled.asStateFlow()

    private val _overrides = MutableStateFlow<Map<String, Boolean>>(emptyMap())
    /** The global app filter; a PC can have its own instead. */
    val overrides: StateFlow<Map<String, Boolean>> = _overrides.asStateFlow()

    fun init(ctx: Context) {
        val app = ctx.applicationContext
        plain = app.getSharedPreferences("pill", Context.MODE_PRIVATE)
        secure = openSecure(app)
        _onboarded.value = plain.getBoolean("onboarded", false)
        _autoReceive.value = plain.getBoolean("autoReceive", false)
        _includeOngoing.value = plain.getBoolean("includeOngoing", false)
        _phoneExpanded.value = plain.getBoolean("phoneExpanded", false)
        _relayEnabled.value = plain.getBoolean("relayEnabled", true)
        _overrides.value = plain.getStringSet("filterOn", emptySet()).orEmpty().associateWith { true } +
            plain.getStringSet("filterOff", emptySet()).orEmpty().associateWith { false }
        _selectedPc.value = plain.getString("selectedPc", null)
        _pcs.value = readPairings()
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

    /** One stable id per install, kept across re-pairing so a PC replaces the token instead of adding a device. The same for every PC. */
    val deviceId: String
        get() = secure.getString("deviceId", null) ?: UUID.randomUUID().toString().also { secure.edit().putString("deviceId", it).apply() }

    // ---- pairings ----

    /** The list from the encrypted store; the single pairing of the first app version is moved into it once, without pairing again. */
    private fun readPairings(): List<Paired> {
        val stored = secure.getString("pcs", null)
        if (stored != null) return PairingBook.decode(stored)
        val list = PairingBook.migrate(
            secure.getString("host", null), secure.getInt("port", 0), secure.getString("token", null),
            secure.getString("pcName", null), deviceId, readLegacyRelay(),
        )
        val e = secure.edit()
        e.putString("pcs", PairingBook.encode(list)).remove("host").remove("port").remove("token").remove("pcName")
        for (k in LEGACY_RELAY_KEYS) e.remove("relay.$k")
        e.apply()
        return list
    }

    private fun readLegacyRelay(): RelayCfg? {
        fun s(k: String) = secure.getString("relay.$k", null)
        return RelayCfg(s("url") ?: return null, s("room") ?: return null, s("join") ?: return null, s("kp2c") ?: return null,
            s("kc2p") ?: return null, s("sk") ?: return null, s("pcpk") ?: return null)
    }

    private fun write(next: List<Paired>) {
        secure.edit().putString("pcs", PairingBook.encode(next)).apply()
        _pcs.value = next
    }

    private fun mutate(f: (List<Paired>) -> List<Paired>) = synchronized(lock) {
        val next = f(_pcs.value)
        if (next == _pcs.value) return@synchronized
        write(next)
        if (_selectedPc.value != null && next.none { it.pcId == _selectedPc.value }) setSelectedPc(null)
    }

    fun pc(pcId: String): Paired? = _pcs.value.firstOrNull { it.pcId == pcId }

    /** A fresh pairing: replaces the entry of the same PC (same address or name), otherwise adds one. Returns what was written. */
    fun addOrReplace(host: String, port: Int, token: String, pcName: String, relay: RelayCfg?): Paired = synchronized(lock) {
        val (next, written) = PairingBook.upsert(_pcs.value, Paired(PairingBook.newPcId(), host, port, token, pcName, deviceId, relay))
        write(next)
        written
    }

    /** Forget one PC (unpair, or that PC revoked us). The device id and the other PCs stay. */
    fun remove(pcId: String) = mutate { PairingBook.remove(it, pcId) }

    /** The PC reports a new name of its own. */
    fun setPcName(pcId: String, name: String) = mutate { l -> PairingBook.update(l, pcId) { it.copy(pcName = name) } }

    /** The user's own name for a PC; blank clears it. */
    fun rename(pcId: String, alias: String) = mutate { l -> PairingBook.update(l, pcId) { it.copy(alias = alias.trim().take(40).ifBlank { null }) } }

    fun setMirror(pcId: String, on: Boolean) = mutate { l -> PairingBook.update(l, pcId) { it.copy(mirror = on) } }
    fun setAutoReceiveFor(pcId: String, on: Boolean?) = mutate { l -> PairingBook.update(l, pcId) { it.copy(autoReceive = on) } }

    /** Give the PC its own app filter (starting from a copy of the global one) or hand it back to the global one. */
    fun setOwnFilter(pcId: String, own: Boolean) = mutate { l ->
        PairingBook.update(l, pcId) { it.copy(filter = if (own) (it.filter ?: _overrides.value) else null) }
    }

    fun setAppEnabledFor(pcId: String, pkg: String, on: Boolean) = mutate { l ->
        PairingBook.update(l, pcId) { it.copy(filter = (it.filter ?: _overrides.value) + (pkg to on)) }
    }

    /** A session with this PC is up (or just ended). */
    fun touchSeen(pcId: String, at: Long = System.currentTimeMillis()) = mutate { l -> PairingBook.update(l, pcId) { it.copy(lastSeen = at) } }

    fun setSelectedPc(pcId: String?) {
        plain.edit().apply { if (pcId == null) remove("selectedPc") else putString("selectedPc", pcId) }.apply()
        _selectedPc.value = pcId
    }

    // ---- plain settings ----

    fun setOnboarded() { plain.edit().putBoolean("onboarded", true).apply(); _onboarded.value = true }
    fun setAutoReceive(on: Boolean) { plain.edit().putBoolean("autoReceive", on).apply(); _autoReceive.value = on }
    fun setPhoneExpanded(on: Boolean) { plain.edit().putBoolean("phoneExpanded", on).apply(); _phoneExpanded.value = on }
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
