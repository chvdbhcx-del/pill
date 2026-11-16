package app.pill.phone.data

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

/** The two files of the public rolling release; the manifest is the small one the app reads (docs/phone-updates.md). */
const val UPDATE_MANIFEST_URL = "https://github.com/chvdbhcx-del/pill/releases/download/phone-latest/PillPhone.json"
const val UPDATE_APK_URL = "https://github.com/chvdbhcx-del/pill/releases/download/phone-latest/PillPhone.apk"

/** `PillPhone.json`: what the newest published APK is and how to recognise it. */
data class UpdateManifest(
    val versionName: String,
    val versionCode: Long,
    /** Lower-case hex of the APK. */
    val sha256: String,
    val size: Long,
    val minSdk: Int = 1,
    val notes: String = "",
)

/** What went wrong, as the user should read it (never a raw exception). */
enum class UpdateFail {
    /** The version file could not be fetched. */
    Check,
    /** The version file is there but not usable (or the redirect left GitHub). */
    Server,
    /** The download broke off. */
    Download,
    /** The bytes are not the published ones (short, long, or another hash): discarded. */
    Mismatch,
    /** No room to keep the file. */
    Storage,
    /** Android refused the install (another signature, blocked, conflicting). */
    Install,
}

enum class UpdatePhase { Idle, Checking, UpToDate, Available, Downloading, Verifying, Installing, NeedsPermission, Failed }

/** [offer] is the newest build known to be newer than the installed one; it survives a failed check or download so Retry has something to retry. */
data class UpdateState(
    val phase: UpdatePhase = UpdatePhase.Idle,
    val offer: UpdateManifest? = null,
    val progress: Float = 0f,
    val fail: UpdateFail? = null,
)

/** The rules of the updater, free of Android so they can be tested on the JVM. */
object UpdatePolicy {
    const val MAX_MANIFEST_BYTES = 16 * 1024L
    const val MAX_APK_BYTES = 200L * 1024 * 1024
    const val MAX_NOTES = 600
    /** The automatic check runs at most this often. */
    const val CHECK_EVERY_MS = 12 * 3_600_000L

    /** github.com redirects a release download to its asset host; GitHub has used objects. and now release-assets. */
    private val HOSTS = setOf("github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com")
    private val VERSION = Regex("[0-9A-Za-z._+-]{1,32}")
    private val HEX64 = Regex("[0-9a-fA-F]{64}")

    /** Only HTTPS, and only the hosts a GitHub release is served from. */
    fun hostAllowed(scheme: String?, host: String?): Boolean = scheme.equals("https", ignoreCase = true) && host?.lowercase() in HOSTS

    /** Strict: a required field missing or of the wrong type makes the whole manifest unusable; unknown fields are ignored. */
    fun parse(text: String): UpdateManifest? {
        if (text.length > MAX_MANIFEST_BYTES) return null
        val o = try { Json.parseToJsonElement(text) as? JsonObject } catch (_: Exception) { null } ?: return null
        fun str(k: String): String? = (o[k] as? JsonPrimitive)?.takeIf { it.isString }?.content
        fun num(k: String): JsonPrimitive? = (o[k] as? JsonPrimitive)?.takeIf { !it.isString }
        val name = str("versionName")?.takeIf { VERSION.matches(it) } ?: return null
        val code = num("versionCode")?.longOrNull?.takeIf { it >= 1 } ?: return null
        val sha = str("sha256")?.takeIf { HEX64.matches(it) }?.lowercase() ?: return null
        val size = num("size")?.longOrNull?.takeIf { it in 1..MAX_APK_BYTES } ?: return null
        val minSdk = when (val m = o["minSdk"]) {
            null, is JsonNull -> 1
            else -> (m as? JsonPrimitive)?.takeIf { !it.isString }?.intOrNull?.takeIf { it in 1..99 } ?: return null
        }
        val notes = when (val n = o["notes"]) {
            null, is JsonNull -> ""
            else -> (n as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull ?: return null
        }.filter { it == '\n' || !it.isISOControl() }.trim().take(MAX_NOTES)
        return UpdateManifest(name, code, sha, size, minSdk, notes)
    }

    /** Offer a build only if it is newer than what is installed and this Android can run it. */
    fun offers(m: UpdateManifest, installedCode: Long, sdk: Int): Boolean = m.versionCode > installedCode && m.minSdk <= sdk

    /** On app start: auto-check on, and none in the last [CHECK_EVERY_MS]. A clock that went backwards counts as due. */
    fun due(auto: Boolean, lastCheck: Long, now: Long): Boolean = auto && (lastCheck <= 0 || now < lastCheck || now - lastCheck >= CHECK_EVERY_MS)
}
