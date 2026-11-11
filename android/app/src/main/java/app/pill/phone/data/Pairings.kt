package app.pill.phone.data

import app.pill.phone.protocol.RelayCfg
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.UUID

/**
 * One paired PC: how to reach it, the bearer token and its relay keys, plus this PC's own settings. A phone holds a list of
 * these (see [PairingBook]); [pcId] is the stable key, made at pairing time, while [deviceId] is the per-install id every PC
 * sees (the same for all of them). The whole list lives in the encrypted store, so tokens and keys never leave it.
 */
@Serializable
data class Paired(
    val pcId: String,
    val host: String,
    val port: Int,
    val token: String,
    /** The name the PC reports about itself (it follows the PC when that is renamed). */
    val pcName: String,
    val deviceId: String,
    /** Keys and address of the internet relay (protocol v2); null for a PC that cannot do it: the phone then works on the LAN only. */
    val relay: RelayCfg? = null,
    /** The user's own name for this PC (rename), shown instead of [pcName]. */
    val alias: String? = null,
    /** "Mirror notifications" for this PC. */
    val mirror: Boolean = true,
    /** "Auto-receive files" for this PC; null follows the global switch. */
    val autoReceive: Boolean? = null,
    /** This PC's own app filter (package -> on/off overrides); null follows the global filter. */
    val filter: Map<String, Boolean>? = null,
    /** Last time a session with this PC was up, ms. 0 = never. */
    val lastSeen: Long = 0,
) {
    val address: String get() = "$host:$port"

    /** What the user sees: their own name, else the PC's. */
    val label: String get() = alias?.takeIf { it.isNotBlank() } ?: pcName.ifBlank { host }

    /** The app overrides that apply to this PC given the global ones. */
    fun overrides(global: Map<String, Boolean>): Map<String, Boolean> = filter ?: global

    fun autoReceives(global: Boolean): Boolean = autoReceive ?: global

    /** Everything that decides how the connection is made; a change means the running session must restart. */
    val credentials: List<Any?> get() = listOf(host, port, token, relay)
}

/** The list of pairings as pure functions, so the rules (replace or add, migrate, unpair one) are testable without a phone. */
object PairingBook {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    fun newPcId(): String = UUID.randomUUID().toString()

    fun encode(list: List<Paired>): String = json.encodeToString(kotlinx.serialization.builtins.ListSerializer(Paired.serializer()), list)

    fun decode(text: String?): List<Paired> {
        if (text.isNullOrBlank()) return emptyList()
        return try { json.decodeFromString(kotlinx.serialization.builtins.ListSerializer(Paired.serializer()), text) } catch (_: Exception) { emptyList() }
    }

    /** The single pairing of the old app version becomes the first entry; its settings stay at their defaults (they follow the global ones). */
    fun migrate(
        host: String?, port: Int, token: String?, pcName: String?, deviceId: String, relay: RelayCfg?, pcId: String = newPcId(),
    ): List<Paired> {
        if (host.isNullOrBlank() || token.isNullOrBlank()) return emptyList()
        return listOf(Paired(pcId, host, port, token, pcName.orEmpty(), deviceId, relay))
    }

    /** The entry a fresh pairing replaces: the same address, or the same (non-blank) name. */
    fun findSame(list: List<Paired>, host: String, port: Int, pcName: String): Paired? =
        list.firstOrNull { it.host == host && it.port == port }
            ?: list.firstOrNull { pcName.isNotBlank() && it.pcName.equals(pcName, ignoreCase = true) }

    /**
     * A fresh pairing: replaces the entry of the same PC (keeping its id, its name and its settings, taking the new token and keys),
     * otherwise it is added at the end. Returns the new list and the entry that was written.
     */
    fun upsert(list: List<Paired>, fresh: Paired): Pair<List<Paired>, Paired> {
        val old = findSame(list, fresh.host, fresh.port, fresh.pcName)
        if (old == null) return (list + fresh) to fresh
        val merged = fresh.copy(pcId = old.pcId, alias = old.alias, mirror = old.mirror, autoReceive = old.autoReceive, filter = old.filter, lastSeen = old.lastSeen)
        return list.map { if (it.pcId == old.pcId) merged else it } to merged
    }

    fun remove(list: List<Paired>, pcId: String): List<Paired> = list.filter { it.pcId != pcId }

    fun update(list: List<Paired>, pcId: String, f: (Paired) -> Paired): List<Paired> = list.map { if (it.pcId == pcId) f(it) else it }
}
