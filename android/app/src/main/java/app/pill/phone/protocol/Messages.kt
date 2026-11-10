package app.pill.phone.protocol

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Every WebSocket frame of protocol v1 (docs/phone-protocol.md in the Pill repo) as a typed message.
 * The discriminator is the `t` field. One sealed hierarchy for both directions so the same
 * [Codec] encodes what the phone sends and decodes what the PC sends (the tests round-trip all of it).
 */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("t")
sealed interface Msg {

    // ---- both directions ----
    @Serializable @SerialName("ping") data object Ping : Msg
    @Serializable @SerialName("pong") data object Pong : Msg

    /** Phone -> PC: the phone wants to send a file. PC -> phone: the PC offers a file. */
    @Serializable @SerialName("file.offer")
    data class FileOffer(val id: String, val name: String, val size: Long, val mime: String = "") : Msg

    // ---- PC -> phone ----
    @Serializable @SerialName("hello")
    data class Hello(val pcName: String = "", val serverVersion: Int = 1, val time: Long = 0) : Msg

    @Serializable @SerialName("notif.dismiss") data class NotifDismiss(val key: String) : Msg
    @Serializable @SerialName("notif.read") data class NotifRead(val key: String) : Msg
    @Serializable @SerialName("notif.reply")
    data class NotifReply(val req: String, val key: String, val text: String) : Msg

    @Serializable @SerialName("file.accept") data class FileAccept(val id: String) : Msg
    @Serializable @SerialName("file.reject") data class FileReject(val id: String, val reason: String = "") : Msg
    @Serializable @SerialName("find") data object Find : Msg

    // ---- phone -> PC ----
    @Serializable @SerialName("notif.posted")
    data class NotifPosted(
        val key: String,
        val pkg: String,
        val app: String,
        val title: String,
        val text: String,
        val sub: String,
        @SerialName("when") val whenMs: Long,
        val category: String,
        val ongoing: Boolean,
        /** Percent 0..100, or null when there is no (determinate) progress. */
        val progress: Int?,
        val icon: String?,
        val avatar: String?,
        val canReply: Boolean,
        val actions: List<Action>,
        val group: String?,
        /** Written to the wire only when true: the notification is re-sent after a reconnect. */
        val replay: Boolean = false,
    ) : Msg {
        @Serializable data class Action(val id: String, val title: String)
    }

    @Serializable @SerialName("notif.removed") data class NotifRemoved(val key: String) : Msg

    @Serializable @SerialName("status")
    data class Status(
        val battery: Int,
        val charging: Boolean,
        /** "wifi", "mobile" or "none". */
        val network: String,
        val ssid: String?,
        val dnd: Boolean,
        val time: Long,
        // ---- protocol v1.1, all optional: absent on the wire when unknown (older PCs ignore them) ----
        /** "none", "charging" or "full". */
        val charge: String? = null,
        /** "ac", "usb" or "wireless", only while plugged in. */
        val plug: String? = null,
        /** Battery temperature in deg C, one decimal. */
        val tempC: Double? = null,
        /** good, overheat, cold, dead, over_voltage, failure, unknown. */
        val health: String? = null,
        /** Minutes to a full battery, when the OS knows (API 28+). */
        val fullInMin: Int? = null,
        /** Internal storage in bytes. */
        val storage: Space? = null,
        /** Memory in MB. */
        val ram: Space? = null,
        val net: NetInfo? = null,
        val airplane: Boolean? = null,
        /** "sound", "vibrate" or "silent". */
        val ringer: String? = null,
        val screen: Boolean? = null,
        /** The phone's time zone id and its current UTC offset in minutes. */
        val tz: String? = null,
        val utcMin: Int? = null,
    ) : Msg {
        @Serializable data class Space(val free: Long, val total: Long)

        /** Details of the connection: Wi-Fi speed, or mobile carrier and generation; [bars] is the signal level 0..4. */
        @Serializable data class NetInfo(
            val type: String? = null,
            val mbps: Int? = null,
            val bars: Int? = null,
            val carrier: String? = null,
            val gen: String? = null,
        )
    }

    @Serializable @SerialName("reply.result")
    data class ReplyResult(val req: String, val ok: Boolean, val error: String? = null) : Msg

    @Serializable @SerialName("file.done") data class FileDone(val id: String, val ok: Boolean) : Msg
}

object Codec {
    @OptIn(ExperimentalSerializationApi::class)
    val json = Json {
        classDiscriminator = "t"
        ignoreUnknownKeys = true
        explicitNulls = true        // "progress":null must be on the wire
        encodeDefaults = false      // ...but replay=false and an absent error stay off it
        isLenient = true
    }

    fun encode(msg: Msg): String = json.encodeToString(Msg.serializer(), msg)

    /** Undecodable or unknown frames give null: the link ignores them, the protocol may grow. */
    fun decode(text: String): Msg? = try {
        json.decodeFromString(Msg.serializer(), text)
    } catch (_: Exception) {
        null
    }

    /** One string field of a small JSON body (error answers and the like) without declaring a class. */
    fun field(text: String, name: String): String? = try {
        ((json.parseToJsonElement(text) as? JsonObject)?.get(name) as? JsonPrimitive)?.content
    } catch (_: Exception) {
        null
    }
}

/** POST /api/pair body and answer. */
@Serializable
data class PairRequest(
    val code: String, val deviceId: String, val name: String,
    val model: String, val os: String, val appVersion: String,
)

@Serializable
data class PairResponse(val token: String, val pcName: String = "", val serverVersion: Int = 1)
