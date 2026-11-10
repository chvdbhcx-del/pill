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
    data class Ok(val token: String, val pcName: String) : PairResult
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
        val body = Codec.json.encodeToString(
            PairRequest.serializer(),
            PairRequest(t.code, deviceId, name, model, "Android ${Build.VERSION.RELEASE}", BuildConfig.VERSION_NAME),
        ).toRequestBody(JSON)
        val req = Request.Builder().url("http://${t.host}:${t.port}/api/pair").post(body).build()
        return try {
            http.newCall(req).await().use { r ->
                val text = r.body?.string().orEmpty()
                when {
                    r.code == 200 -> {
                        val ok = try { Codec.json.decodeFromString(PairResponse.serializer(), text) } catch (_: Exception) { null }
                        if (ok == null || ok.token.isBlank()) PairResult.Refused(200) else PairResult.Ok(ok.token, ok.pcName.ifBlank { t.pcName ?: t.host })
                    }
                    r.code == 403 -> PairResult.BadCode
                    else -> PairResult.Refused(r.code)
                }
            }
        } catch (_: IOException) {
            PairResult.Unreachable
        }
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
