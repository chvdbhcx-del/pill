package app.pill.phone.net

import android.content.Context
import android.os.Build
import android.provider.Settings
import app.pill.phone.BuildConfig
import app.pill.phone.data.Paired
import app.pill.phone.protocol.Codec
import app.pill.phone.protocol.PairRequest
import app.pill.phone.protocol.PairResponse
import app.pill.phone.protocol.PairTarget
import app.pill.phone.protocol.RelayCfg
import app.pill.phone.protocol.V2
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

sealed interface PairResult {
    /** [relay] is null for a PC that cannot do the internet link (an old Pill, or no relay configured). */
    data class Ok(val token: String, val pcName: String, val relay: RelayCfg? = null) : PairResult
    /** The PC's relay block does not match what we derived ourselves: nothing is saved. */
    data object RelayMismatch : PairResult
    data object BadCode : PairResult
    data object Unreachable : PairResult
    data class Refused(val code: Int) : PairResult
}

/** Plain HTTP calls to the PC (protocol "HTTP endpoints"). LAN only, so no TLS; the bearer token guards everything. */
object PcApi {
    private val JSON = "application/json; charset=utf-8".toMediaType()

    /** Short calls: pairing, state, unpair. */
    val http: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(6, TimeUnit.SECONDS)
        .readTimeout(15, TimeUnit.SECONDS)
        .writeTimeout(15, TimeUnit.SECONDS)
        .retryOnConnectionFailure(true)
        .build()

    /** The WebSocket: it is silent for long stretches, so no read timeout (our own ping watchdog decides). */
    val socket: OkHttpClient = http.newBuilder().readTimeout(0, TimeUnit.SECONDS).build()

    /** The relay WebSocket: OkHttp's own ping (the relay wants a ping every 25 s anyway) also catches a dead NAT mapping. */
    val relay: OkHttpClient = socket.newBuilder().pingInterval(25, TimeUnit.SECONDS).build()

    /** File bodies: big and slow, so only a stall timeout. */
    val transfer: OkHttpClient = http.newBuilder()
        .readTimeout(60, TimeUnit.SECONDS).writeTimeout(60, TimeUnit.SECONDS).callTimeout(0, TimeUnit.SECONDS).build()

    fun url(p: Paired, path: String) = "http://${p.host}:${p.port}$path"

    fun authed(p: Paired, path: String): Request.Builder =
        Request.Builder().url(url(p, path)).header("Authorization", "Bearer ${p.token}")

    suspend fun pair(t: PairTarget, ctx: Context, deviceId: String): PairResult {
        val model = Build.MODEL ?: "Android"
        val name = try {
            Settings.Global.getString(ctx.contentResolver, "device_name")
        } catch (_: Exception) {
            null
        }?.takeIf { it.isNotBlank() } ?: model
        val keys = V2.newKeyPair()   // one per pairing; the secret half is stored (Keystore-backed) only if the PC answers with its half
        val body = Codec.json.encodeToString(
            PairRequest.serializer(),
            PairRequest(t.code, deviceId, name, model, "Android ${Build.VERSION.RELEASE}", BuildConfig.VERSION_NAME, V2.b64u(keys.public)),
        ).toRequestBody(JSON)
        val req = Request.Builder().url("http://${t.host}:${t.port}/api/pair").post(body).build()
        return try {
            http.newCall(req).await().use { r ->
                val text = r.body?.string().orEmpty()
                when {
                    r.code == 200 -> {
                        val ok = try { Codec.json.decodeFromString(PairResponse.serializer(), text) } catch (_: Exception) { null }
                        if (ok == null || ok.token.isBlank()) PairResult.Refused(200) else {
                            val name = ok.pcName.ifBlank { t.pcName ?: t.host }
                            when (val r = V2.completePairing(keys.secret, ok.pk, ok.relay, t.code, deviceId)) {
                                V2.Pairing.NotOffered -> PairResult.Ok(ok.token, name)
                                V2.Pairing.Mismatch -> PairResult.RelayMismatch
                                is V2.Pairing.Ok -> PairResult.Ok(ok.token, name, r.cfg)
                            }
                        }
                    }
                    r.code == 403 -> PairResult.BadCode
                    else -> PairResult.Refused(r.code)
                }
            }
        } catch (_: IOException) {
            PairResult.Unreachable
        }
    }

    /** Is the PC answering on its LAN address right now? A cheap authenticated GET with a short timeout (the "try the LAN again" probe while on the relay). */
    suspend fun lanReachable(p: Paired, timeoutMs: Long = 2_000): Boolean = try {
        val c = http.newBuilder().callTimeout(timeoutMs, TimeUnit.MILLISECONDS).retryOnConnectionFailure(false).build()
        c.newCall(authed(p, "/api/state").build()).await().use { it.isSuccessful }
    } catch (_: IOException) {
        false
    }

    /** Tell the PC to forget this phone. Best effort: the local pairing is dropped either way. */
    suspend fun unpair(p: Paired): Boolean = try {
        http.newCall(authed(p, "/api/device").delete().build()).await().use { it.isSuccessful }
    } catch (_: IOException) {
        false
    }
}

suspend fun Call.await(): Response = suspendCancellableCoroutine { c ->
    c.invokeOnCancellation { cancel() }
    enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) { if (c.isActive) c.resumeWithException(e) }
        override fun onResponse(call: Call, response: Response) { c.resume(response) { _, _, _ -> response.close() } }
    })
}
