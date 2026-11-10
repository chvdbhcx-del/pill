package app.pill.phone.protocol

import java.net.URI
import java.net.URLDecoder

object Proto {
    const val DEFAULT_PORT = 41717
    const val SERVER_VERSION = 1
    const val PING_MS = 20_000L

    /** No frame (not even a pong) for this long means the socket is dead. */
    const val DEAD_AFTER_MS = 50_000L
}

/** Where and how to pair: parsed from a `pillphone://pair?...` link, the PC's `http://ip:port/p?c=` QR, or typed in. */
data class PairTarget(val host: String, val port: Int, val code: String, val pcName: String? = null) {
    val address: String get() = "$host:$port"
}

object PairLink {
    private val CODE = Regex("^[A-Z2-9]{8}$")
    private val HOSTNAME = Regex("^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?(\\.[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?)*$")
    private val IPV4 = Regex("^(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})$")

    fun normalizeCode(raw: String): String = raw.filter { !it.isWhitespace() && it != '-' }.uppercase()

    fun isValidCode(raw: String): Boolean = CODE.matches(normalizeCode(raw))

    /** The protocol is LAN-only: reject public IPv4 addresses and dotted public host names. */
    fun isLanHost(host: String): Boolean {
        val m = IPV4.matchEntire(host)
        if (m != null) {
            val o = m.groupValues.drop(1).map { it.toInt() }
            if (o.any { it > 255 }) return false
            return o[0] == 10 || o[0] == 127 ||
                (o[0] == 172 && o[1] in 16..31) ||
                (o[0] == 192 && o[1] == 168) ||
                (o[0] == 169 && o[1] == 254)
        }
        if (!HOSTNAME.matches(host)) return false
        // "mypc", "mypc.local", "mypc.lan"; a dotted public name such as example.com is not the LAN.
        return !host.contains('.') || host.endsWith(".local", true) || host.endsWith(".lan", true) ||
            host.endsWith(".home", true) || host.endsWith(".internal", true)
    }

    /** Accepts pillphone://pair?h=&p=&c=&n= , the intent:// form, and http://host:port/p?c=CODE . */
    fun parse(raw: String?): PairTarget? {
        val s = raw?.trim().orEmpty()
        if (s.isEmpty()) return null
        return try {
            when {
                s.startsWith("pillphone:", true) -> fromQuery(query(s))
                s.startsWith("intent:", true) -> fromQuery(query(s.substringBefore("#Intent")))
                s.startsWith("http://", true) || s.startsWith("https://", true) -> fromUrl(s)
                else -> null
            }
        } catch (_: Exception) {
            null
        }
    }

    /** "host:port" (or just "host": default port) plus the typed code; a pasted PC link works too. */
    fun parseManual(address: String, code: String): PairTarget? {
        if (address.contains("c=")) {
            val fromLink = parse(address) ?: return null
            return if (isValidCode(code)) fromLink.copy(code = normalizeCode(code)) else fromLink
        }
        val a = address.trim().removePrefix("http://").removePrefix("https://").substringBefore('/')
        if (a.isEmpty() || a.contains(' ')) return null
        val hasPort = a.contains(':') && !a.startsWith("[")
        val host = if (hasPort) a.substringBeforeLast(':') else a
        val port = if (hasPort) a.substringAfterLast(':').toIntOrNull() ?: return null else Proto.DEFAULT_PORT
        return build(host, port, code, null)
    }

    private fun fromQuery(q: Map<String, String>): PairTarget? {
        val h = q["h"] ?: return null
        val p = q["p"]?.toIntOrNull() ?: return null
        val c = q["c"] ?: return null
        return build(h, p, c, q["n"])
    }

    private fun fromUrl(s: String): PairTarget? {
        val u = URI(s)
        val host = u.host ?: return null
        if (u.port <= 0 || u.path != "/p") return null
        return build(host, u.port, query(s)["c"] ?: return null, null)
    }

    private fun build(host: String, port: Int, code: String, name: String?): PairTarget? {
        val h = host.trim()
        if (port !in 1..65535 || !isLanHost(h) || !isValidCode(code)) return null
        return PairTarget(h, port, normalizeCode(code), name?.takeIf { it.isNotBlank() }?.take(64))
    }

    private fun query(s: String): Map<String, String> {
        val q = s.substringAfter('?', "").substringBefore('#')
        if (q.isEmpty()) return emptyMap()
        return q.split('&').mapNotNull {
            val i = it.indexOf('=')
            if (i <= 0) null else URLDecoder.decode(it.substring(0, i), "UTF-8") to URLDecoder.decode(it.substring(i + 1), "UTF-8")
        }.toMap()
    }
}
