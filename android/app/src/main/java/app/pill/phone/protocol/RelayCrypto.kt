package app.pill.phone.protocol

import com.google.crypto.tink.subtle.Hkdf
import com.google.crypto.tink.subtle.X25519
import com.google.crypto.tink.subtle.XChaCha20Poly1305
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/** What a pairing with a v2 PC leaves on the phone (docs/phone-relay.md, "Keys"). Strings so [Paired] keeps value equality. */
data class RelayCfg(
    /** Base URL of the relay worker, `wss://...`. */
    val url: String,
    val room: String,
    /** The join secret, base64url. */
    val join: String,
    val kP2c: String,
    val kC2p: String,
    /** Our X25519 secret key and the PC's public key, base64url. Kept so a later re-derivation is possible. */
    val secret: String,
    val pcPublic: String,
)

/** Everything derived from the X25519 exchange; pure functions, nothing here touches Android. */
object V2 {
    private const val SALT_PREFIX = "pill-phone-v2"
    private val rng = SecureRandom()

    fun b64u(b: ByteArray): String = Base64.getUrlEncoder().withoutPadding().encodeToString(b)
    fun unb64u(s: String): ByteArray = Base64.getUrlDecoder().decode(s.trim())

    class KeyPair(val secret: ByteArray, val public: ByteArray)

    fun newKeyPair(): KeyPair {
        val sk = X25519.generatePrivateKey()
        return KeyPair(sk, X25519.publicFromPrivate(sk))
    }

    class Keys(val kP2c: ByteArray, val kC2p: ByteArray, val room: String, val join: String)

    private fun hmac(key: ByteArray, data: String): ByteArray =
        Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(key, "HmacSHA256")) }.doFinal(data.toByteArray())

    /** `okm = HKDF-SHA256(X25519(sk, pk), salt = SHA256("pill-phone-v2" || code || deviceId), "keys", 96)`, then room and join from `k_room`. */
    fun derive(secret: ByteArray, peerPublic: ByteArray, code: String, deviceId: String): Keys {
        val shared = X25519.computeSharedSecret(secret, peerPublic)
        val salt = MessageDigest.getInstance("SHA-256").digest((SALT_PREFIX + code + deviceId).toByteArray())
        val okm = Hkdf.computeHkdf("HmacSha256", shared, salt, "keys".toByteArray(), 96)
        val kRoom = okm.copyOfRange(64, 96)
        val room = hmac(kRoom, "room").copyOfRange(0, 16).joinToString("") { "%02x".format(it) }
        return Keys(okm.copyOfRange(0, 32), okm.copyOfRange(32, 64), room, b64u(hmac(kRoom, "join")))
    }

    sealed interface Pairing {
        /** Old PC (no `pk`/`relay` in the answer) or a relay-less setup: LAN only. */
        data object NotOffered : Pairing
        /** The answer and our own derivation disagree, or the PC key is garbage: pairing must fail. */
        data object Mismatch : Pairing
        data class Ok(val cfg: RelayCfg) : Pairing
    }

    /** Check the PC's half of the exchange. [relay] is the answer's relay block, [pcPk] its public key. */
    fun completePairing(secret: ByteArray, pcPk: String?, relay: RelayBlock?, code: String, deviceId: String): Pairing {
        if (pcPk.isNullOrBlank() || relay == null) return Pairing.NotOffered
        return try {
            val pub = unb64u(pcPk)
            if (pub.size != 32) return Pairing.Mismatch
            val k = derive(secret, pub, code, deviceId)
            val gotJoin = unb64u(relay.join)
            // constant-time on the secret part; the room is not secret
            if (k.room != relay.room.lowercase() || !MessageDigest.isEqual(unb64u(k.join), gotJoin)) return Pairing.Mismatch
            if (RelayUrl.build(relay.url, k.room) == null) return Pairing.NotOffered
            Pairing.Ok(RelayCfg(relay.url.trim(), k.room, k.join, b64u(k.kP2c), b64u(k.kC2p), b64u(secret), b64u(pub)))
        } catch (_: Exception) {
            Pairing.Mismatch
        }
    }

    fun randomSid(): String = ByteArray(8).also { rng.nextBytes(it) }.joinToString("") { "%02x".format(it) }
}

/** `wss://<worker>/r/<room>?role=phone`. The block's url is the worker's base; a url that already names the room is kept. */
object RelayUrl {
    fun build(base: String, room: String): String? {
        var b = base.trim().trimEnd('/')
        if (b.startsWith("https://")) b = "wss://" + b.removePrefix("https://")
        if (!b.startsWith("wss://") || b.length <= 6) return null
        val u = if ("/r/" in b) b else "$b/r/$room"
        return u + (if ('?' in u) "&" else "?") + "role=phone"
    }
}

/** 50 frames that fail authentication within a minute close the connection. */
class AuthFailGuard(private val limit: Int = 50, private val windowMs: Long = 60_000) {
    private val times = ArrayDeque<Long>()

    /** Record one failure at [now]; true when the limit is reached and the connection should be closed. */
    @Synchronized
    fun record(now: Long): Boolean {
        times.addLast(now)
        while (times.isNotEmpty() && now - times.first() >= windowMs) times.removeFirst()
        return times.size >= limit
    }
}

/**
 * One direction pair of an end-to-end encrypted relay connection: `frame = nonce(24) || XChaCha20-Poly1305(envelope)`,
 * `aad = "pill-v2" || room || dir`, `envelope = {"sid","seq","m"}` (docs/phone-relay.md, "Frames over the relay").
 * The phone sends `p2c` and receives `c2p`; tests build the PC's side by swapping the keys and directions.
 */
class FrameCrypto(
    private val room: String,
    kSend: ByteArray,
    kRecv: ByteArray,
    private val sendDir: String = "p2c",
    private val recvDir: String = "c2p",
    val sid: String = V2.randomSid(),
) {
    private val sealer = XChaCha20Poly1305(kSend)
    private val opener = XChaCha20Poly1305(kRecv)
    private var seq = 0L
    // ponytail: the last 16 senders are remembered; a frame of an older sid replayed after 16 reconnects of the PC would pass. Persisting is not worth it.
    private val seen = object : LinkedHashMap<String, Long>(16, 0.75f, false) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Long>?) = size > MAX_SIDS
    }

    sealed interface Opened {
        class Ok(val json: String) : Opened
        /** Authentic but not newer than what we already accepted from that sid. */
        data object Replay : Opened
        /** Wrong key, tampered, truncated or not an envelope: dropped silently and counted by the caller. */
        data object Invalid : Opened
    }

    private fun aad(dir: String) = ("pill-v2" + room + dir).toByteArray()

    /** Encrypt one v1 message (JSON text). Null when it exceeds the 900 KiB plaintext limit. Callers must send frames in the order sealed. */
    @Synchronized
    fun seal(msgJson: String): ByteArray? {
        val env = "{\"sid\":\"$sid\",\"seq\":${++seq},\"m\":$msgJson}".toByteArray()
        if (env.size > MAX_PLAIN) { seq--; return null }
        return sealer.encrypt(env, aad(sendDir))
    }

    @Synchronized
    fun open(frame: ByteArray): Opened {
        if (frame.size < 24 + 16) return Opened.Invalid
        val plain = try {
            opener.decrypt(frame, aad(recvDir))
        } catch (_: Exception) {
            return Opened.Invalid
        }
        val env = try {
            Codec.json.parseToJsonElement(String(plain)) as? JsonObject
        } catch (_: Exception) {
            null
        } ?: return Opened.Invalid
        val rsid = (env["sid"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return Opened.Invalid
        val rseq = (env["seq"] as? JsonPrimitive)?.takeIf { !it.isString }?.content?.toLongOrNull() ?: return Opened.Invalid
        val m = env["m"] ?: return Opened.Invalid
        if (rseq <= (seen[rsid] ?: 0L)) return Opened.Replay
        seen[rsid] = rseq
        return Opened.Ok(m.toString())
    }

    companion object {
        const val MAX_PLAIN = 900 * 1024
        private const val MAX_SIDS = 16
    }
}
