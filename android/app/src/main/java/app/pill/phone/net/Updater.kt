package app.pill.phone.net

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import app.pill.phone.BuildConfig
import app.pill.phone.PillApp
import app.pill.phone.data.Store
import app.pill.phone.data.UPDATE_APK_URL
import app.pill.phone.data.UPDATE_MANIFEST_URL
import app.pill.phone.data.UpdateFail
import app.pill.phone.data.UpdateManifest
import app.pill.phone.data.UpdatePhase
import app.pill.phone.data.UpdatePolicy
import app.pill.phone.data.UpdateState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import okhttp3.Response
import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit

/**
 * Over-the-air updates: reads PillPhone.json from the public phone-latest release, offers a newer build, downloads it (size and SHA-256
 * checked while streaming) and hands it to [ApkInstaller]. Runs only when the app asks (start, "Check now", "Update"): no service, no alarm.
 * The request carries nothing but a generic User-Agent; only github.com and its asset hosts are ever contacted, over HTTPS.
 */
object Updater {
    private val _state = MutableStateFlow(UpdateState())
    val state: StateFlow<UpdateState> = _state.asStateFlow()

    private val REDIRECTS = setOf(301, 302, 303, 307, 308)
    private val BUSY = setOf(UpdatePhase.Checking, UpdatePhase.Downloading, UpdatePhase.Verifying, UpdatePhase.Installing)

    /** Redirects are followed by hand so that every hop is checked against the allow-list before a byte is sent to it. */
    private val client by lazy {
        PcApi.http.newBuilder().followRedirects(false).followSslRedirects(false).callTimeout(0, TimeUnit.SECONDS)
            .connectTimeout(8, TimeUnit.SECONDS).readTimeout(25, TimeUnit.SECONDS).build()
    }

    /** The system's confirmation screen, kept until it has been shown (it cannot start while the app is in the background). */
    @Volatile private var confirm: Intent? = null
    @Volatile private var confirmShown = false

    /** At process start: drop what an earlier run left behind and bring back the offer that was waiting. */
    fun init(ctx: Context) {
        File(ctx.cacheDir, "updates").deleteRecursively()
        val saved = Store.updateOffer?.let(UpdatePolicy::parse)
        if (saved != null && UpdatePolicy.offers(saved, ApkInstaller.installedCode(ctx), Build.VERSION.SDK_INT)) {
            _state.value = UpdateState(UpdatePhase.Available, saved)
        } else Store.updateOffer = null
    }

    /** Called when the app comes to the foreground: the automatic check (if due), the way back from the permission screen, a waiting confirmation. */
    fun onForeground(ctx: Context) {
        val s = _state.value
        when {
            s.phase == UpdatePhase.NeedsPermission -> if (ApkInstaller.canInstall(ctx)) update(ctx)
            s.phase == UpdatePhase.Installing -> showConfirm(ctx)
            UpdatePolicy.due(Store.updateAuto.value, Store.updateLastCheck.value, System.currentTimeMillis()) -> check(ctx)
        }
    }

    private fun begin(to: UpdatePhase): Boolean {
        var ok = false
        _state.update { s -> ok = s.phase !in BUSY; if (ok) s.copy(phase = to, progress = 0f, fail = null) else s }
        return ok
    }

    private fun fail(f: UpdateFail) = _state.update { it.copy(phase = UpdatePhase.Failed, fail = f, progress = 0f) }

    private fun open(url0: String): Response {
        var url = url0.toHttpUrl()
        repeat(6) {
            if (!UpdatePolicy.hostAllowed(url.scheme, url.host)) throw UpdateFailure(UpdateFail.Server)
            val req = Request.Builder().url(url).header("User-Agent", "PillPhone/${BuildConfig.VERSION_NAME}").header("Accept", "*/*").build()
            val resp = client.newCall(req).execute()
            if (resp.code !in REDIRECTS) return resp
            val next = resp.header("Location")?.let { url.resolve(it) }
            resp.close()
            url = next ?: throw UpdateFailure(UpdateFail.Server)
        }
        throw UpdateFailure(UpdateFail.Server)
    }

    /** The manifest text, at most [UpdatePolicy.MAX_MANIFEST_BYTES] of it. */
    private fun fetchManifest(): String {
        val resp = try { open(UPDATE_MANIFEST_URL) } catch (e: UpdateFailure) { throw e } catch (e: IOException) { throw UpdateFailure(UpdateFail.Check, e) }
        resp.use {
            if (!it.isSuccessful) throw UpdateFailure(if (it.code in 400..499) UpdateFail.Server else UpdateFail.Check)
            val src = it.body?.source() ?: throw UpdateFailure(UpdateFail.Server)
            try {
                src.request(UpdatePolicy.MAX_MANIFEST_BYTES + 1)
                if (src.buffer.size > UpdatePolicy.MAX_MANIFEST_BYTES) throw UpdateFailure(UpdateFail.Server)
                return src.readUtf8()
            } catch (e: IOException) {
                if (e is UpdateFailure) throw e
                throw UpdateFailure(UpdateFail.Check, e)
            }
        }
    }

    /** Looks for a newer build. Does nothing while another step is running. */
    fun check(ctx: Context) {
        val app = ctx.applicationContext
        if (!begin(UpdatePhase.Checking)) return
        PillApp.scope.launch {
            Store.setUpdateLastCheck(System.currentTimeMillis())   // an attempt counts, so a dead network is not retried on every start
            try {
                val raw = withContext(Dispatchers.IO) { fetchManifest() }
                val m = UpdatePolicy.parse(raw) ?: throw UpdateFailure(UpdateFail.Server)
                if (UpdatePolicy.offers(m, ApkInstaller.installedCode(app), Build.VERSION.SDK_INT)) {
                    Store.updateOffer = raw
                    _state.value = UpdateState(UpdatePhase.Available, m)
                } else {
                    Store.updateOffer = null
                    _state.value = UpdateState(UpdatePhase.UpToDate)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: UpdateFailure) {
                fail(e.fail)
            } catch (e: Exception) {
                fail(UpdateFail.Check)
            }
        }
    }

    /** The user tapped Update (or Retry): permission first, then download, verify, install. */
    fun update(ctx: Context) {
        val app = ctx.applicationContext
        val offer = _state.value.offer ?: return
        if (_state.value.phase in BUSY) return
        if (!ApkInstaller.canInstall(app)) { _state.update { it.copy(phase = UpdatePhase.NeedsPermission, fail = null) }; return }
        if (!begin(UpdatePhase.Downloading)) return
        PillApp.scope.launch {
            var file: File? = null
            try {
                file = withContext(Dispatchers.IO) { download(app, offer) }
                _state.update { it.copy(phase = UpdatePhase.Verifying, progress = 1f) }
                withContext(Dispatchers.IO) { ApkInstaller.preflight(app, file, offer) }
                _state.update { it.copy(phase = UpdatePhase.Installing) }
                withContext(Dispatchers.IO) { ApkInstaller.install(app, file) }
                // from here the installer answers through onInstallStatus
            } catch (e: CancellationException) {
                throw e
            } catch (e: UpdateFailure) {
                fail(e.fail)
            } catch (e: Exception) {
                fail(UpdateFail.Install)
            } finally {
                file?.delete()   // the session holds its own copy
            }
        }
    }

    private fun download(app: Context, m: UpdateManifest): File {
        val dir = File(app.cacheDir, "updates").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val file = File(dir, "PillPhone-${m.versionCode}.apk")
        try {
            val resp = try { open(UPDATE_APK_URL) } catch (e: UpdateFailure) { throw e } catch (e: IOException) { throw UpdateFailure(UpdateFail.Download, e) }
            resp.use {
                if (!it.isSuccessful) throw UpdateFailure(if (it.code in 400..499) UpdateFail.Server else UpdateFail.Download)
                val body = it.body ?: throw UpdateFailure(UpdateFail.Server)
                if (body.contentLength() > m.size) throw UpdateFailure(UpdateFail.Mismatch)
                val out = try { file.outputStream().buffered() } catch (e: IOException) { throw UpdateFailure(UpdateFail.Storage, e) }
                var last = -1
                out.use { o ->
                    copyVerified(body.byteStream(), o, m.size, m.sha256) { n ->
                        val pct = (n * 100 / m.size).toInt()
                        if (pct != last) { last = pct; _state.update { s -> if (s.phase == UpdatePhase.Downloading) s.copy(progress = pct / 100f) else s } }
                    }
                }
            }
        } catch (e: Throwable) {
            file.delete()
            throw e
        }
        return file
    }

    /** What the installer said about the committed session. */
    fun onInstallStatus(ctx: Context, status: Int, confirmIntent: Intent?) {
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                confirm = confirmIntent
                confirmShown = false
                showConfirm(ctx)
            }
            PackageInstaller.STATUS_SUCCESS -> { confirm = null; _state.update { it.copy(phase = UpdatePhase.Idle) } }
            // the user said no on Android's screen: back to "available", not an error
            PackageInstaller.STATUS_FAILURE_ABORTED -> { confirm = null; _state.update { it.copy(phase = UpdatePhase.Available, fail = null) } }
            else -> { confirm = null; fail(UpdateFail.Install) }
        }
    }

    /** Starts Android's confirmation screen once, and only while the app is in front (a background start is blocked on recent Android). */
    private fun showConfirm(ctx: Context) {
        val i = confirm ?: return
        if (confirmShown || !app.pill.phone.service.ClipBridge.foreground) return
        confirmShown = true
        try { ctx.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } catch (_: Exception) { confirmShown = false }
    }
}
