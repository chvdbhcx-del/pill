package app.pill.phone.transfer

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import androidx.annotation.StringRes
import app.pill.phone.PillApp
import app.pill.phone.R
import app.pill.phone.data.Store
import app.pill.phone.net.Link
import app.pill.phone.net.PcApi
import app.pill.phone.net.Route
import app.pill.phone.net.await
import app.pill.phone.protocol.FileNames
import app.pill.phone.protocol.Msg
import app.pill.phone.service.Notifs
import app.pill.phone.ui.TransferPhase
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.RequestBody
import okio.BufferedSink
import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/** One row of the Files list. */
data class Transfer(
    val id: String,
    val name: String,
    val size: Long,
    val outgoing: Boolean,
    val phase: TransferPhase,
    val bytes: Long = 0,
    @StringRes val error: Int? = null,
    val errorArg: Int = 0,
    val uri: Uri? = null,
    val mime: String = "",
) {
    val progress: Float get() = if (size > 0) (bytes.toFloat() / size).coerceIn(0f, 1f) else 0f
}

/** A file picked on the phone, already opened so it can still be read after the share activity is gone. */
class Source(val name: String, val size: Long, val mime: String, val pfd: ParcelFileDescriptor)

/**
 * File transfer in both directions (protocol "file.*" messages and /api/up, /api/down).
 * Phone -> PC: offer, wait for accept, stream a PUT. PC -> phone: show an offer, download with Range retries into Downloads/Pill.
 */
object Transfers {
    private val _items = MutableStateFlow<List<Transfer>>(emptyList())
    val items: StateFlow<List<Transfer>> = _items

    private val answers = ConcurrentHashMap<String, CompletableDeferred<Msg>>()
    private val uploadQueue = Channel<suspend () -> Unit>(Channel.UNLIMITED)
    private var worker = false

    private const val KEEP = 30
    private const val ANSWER_TIMEOUT_MS = 60_000L

    // ---- phone -> PC ----

    /** Open every uri now (a share grant ends with the activity) and read its name and size. */
    suspend fun prepare(ctx: Context, uris: List<Uri>): List<Source> = withContext(Dispatchers.IO) {
        uris.mapNotNull { runCatching { open(ctx, it) }.getOrNull() }
    }

    private fun open(ctx: Context, uri: Uri): Source {
        val r = ctx.contentResolver
        var name: String? = null
        var size = -1L
        runCatching {
            r.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    name = c.getString(0)
                    if (!c.isNull(1)) size = c.getLong(1)
                }
            }
        }
        val mime = r.getType(uri).orEmpty()
        var pfd = r.openFileDescriptor(uri, "r") ?: throw IOException("cannot open")
        if (size < 0) size = pfd.statSize
        if (size < 0) {
            // A stream without a length (some providers): stage it, the PC needs an exact Content-Length.
            val tmp = File.createTempFile("share", null, ctx.cacheDir)
            r.openInputStream(uri)!!.use { i -> tmp.outputStream().use { o -> i.copyTo(o) } }
            pfd.close()
            size = tmp.length()
            pfd = ParcelFileDescriptor.open(tmp, ParcelFileDescriptor.MODE_READ_ONLY)
            tmp.delete()   // the descriptor stays valid after the name is gone
        }
        val clean = FileNames.sanitize(name ?: uri.lastPathSegment, "file")
        return Source(clean, size, mime, pfd)
    }

    fun send(ctx: Context, sources: List<Source>) {
        val app = ctx.applicationContext
        ensureWorker()
        for (s in sources) {
            val id = UUID.randomUUID().toString()
            add(Transfer(id, s.name, s.size, true, TransferPhase.Waiting, mime = s.mime))
            uploadQueue.trySend { upload(app, id, s) }
        }
    }

    private fun ensureWorker() {
        synchronized(this) {
            if (worker) return
            worker = true
        }
        PillApp.scope.launch(Dispatchers.IO) {
            for (job in uploadQueue) {
                try { job() } catch (e: CancellationException) { throw e } catch (_: Exception) { }
            }
        }
    }

    private suspend fun upload(ctx: Context, id: String, s: Source) {
        s.pfd.use {
            val p = Store.paired.value
            if (p == null || !Link.isConnected) return fail(ctx, id, R.string.tr_err_not_connected)
            if (Link.route == Route.Relay && tooBigForRelay(s.size)) return fail(ctx, id, R.string.tr_err_relay_big, Link.relayMaxFileMB.value)
            val answer = CompletableDeferred<Msg>().also { answers[id] = it }
            Link.send(Msg.FileOffer(id, s.name, s.size, s.mime.ifBlank { Saver.guessMime(s.name, "") }))
            val reply = withTimeoutOrNull(ANSWER_TIMEOUT_MS) { answer.await() }
            answers.remove(id)
            when (reply) {
                is Msg.FileAccept -> Unit
                is Msg.FileReject -> return fail(ctx, id, R.string.tr_err_declined)
                else -> return fail(ctx, id, R.string.tr_err_no_answer)
            }
            patch(id) { it.copy(phase = TransferPhase.Active) }
            Notifs.progress(ctx, id, s.name, true, 0f)
            if (Link.route == Route.Relay) return uploadViaRelay(ctx, id, s)
            val body = object : RequestBody() {
                override fun contentType() = (s.mime.ifBlank { "application/octet-stream" }).toMediaTypeOrNull()
                override fun contentLength() = s.size
                override fun writeTo(sink: BufferedSink) {
                    val input = FileInputStream(s.pfd.fileDescriptor)
                    val buf = ByteArray(64 * 1024)
                    var sent = 0L
                    var lastUi = 0L
                    while (sent < s.size) {
                        val n = input.read(buf, 0, minOf(buf.size.toLong(), s.size - sent).toInt())
                        if (n < 0) throw IOException("file shorter than announced")
                        sink.write(buf, 0, n)
                        sent += n
                        val now = System.currentTimeMillis()
                        if (now - lastUi > 150) { lastUi = now; progress(ctx, id, s.name, true, sent) }
                    }
                }
            }
            try {
                PcApi.transfer.newCall(PcApi.authed(p, "/api/up/$id").put(body).build()).await().use { r ->
                    if (r.isSuccessful) succeed(ctx, id, null) else fail(ctx, id, R.string.tr_err_server, r.code)
                }
            } catch (_: IOException) {
                fail(ctx, id, R.string.tr_err_io)
            }
        }
    }

    // ---- through the relay: file.chunk / file.ack / file.end / file.abort (docs/phone-relay.md), no HTTP side channel ----

    private fun tooBigForRelay(size: Long) = size > Link.relayMaxFileMB.value * 1024L * 1024L

    /** The data messages of a running relay transfer, by file id. */
    private val flows = ConcurrentHashMap<String, Channel<Msg>>()

    private fun inbox(id: String) = Channel<Msg>(Channel.UNLIMITED).also { flows[id] = it }

    /** Streams the file from its descriptor; never more than the window of chunks in memory. */
    private suspend fun uploadViaRelay(ctx: Context, id: String, s: Source) {
        val box = inbox(id)
        try {
            Link.holdRelay {
                val win = SendWindow()
                val input = FileInputStream(s.pfd.fileDescriptor)
                val buf = ByteArray(ChunkFlow.CHUNK)
                var sent = 0L
                var lastUi = 0L
                // null = go on; otherwise the transfer is over (already reported)
                suspend fun handleAnswer(m: Msg): Boolean {
                    when (m) {
                        is Msg.FileAck -> {
                            win.onAck(m.upto)
                            val now = System.currentTimeMillis()
                            if (now - lastUi > 150) { lastUi = now; progress(ctx, id, s.name, true, win.acked) }
                        }
                        is Msg.FileAbort -> { fail(ctx, id, R.string.tr_err_declined); return true }
                        is Msg.FileDone -> if (!m.ok) { fail(ctx, id, R.string.tr_err_io); return true }
                        else -> Unit
                    }
                    return false
                }
                while (sent < s.size) {
                    while (!win.canSend()) {
                        val m = withTimeoutOrNull(STALL_MS) { box.receive() }
                        if (m == null) { Link.send(Msg.FileAbort(id, "stalled")); return@holdRelay fail(ctx, id, R.string.tr_err_io) }
                        if (handleAnswer(m)) return@holdRelay
                    }
                    while (true) { val m = box.tryReceive().getOrNull() ?: break; if (handleAnswer(m)) return@holdRelay }
                    val n = input.read(buf, 0, minOf(buf.size.toLong(), s.size - sent).toInt())
                    if (n < 0) { Link.send(Msg.FileAbort(id, "short_file")); return@holdRelay fail(ctx, id, R.string.tr_err_io) }
                    if (!Link.send(Msg.FileChunk(id, sent, ChunkFlow.encode(buf, n)))) return@holdRelay fail(ctx, id, R.string.tr_err_io)
                    sent += n
                    win.onSent(sent)
                    delay(ChunkFlow.paceDelayMs(n))
                }
                if (!Link.send(Msg.FileEnd(id, s.size))) return@holdRelay fail(ctx, id, R.string.tr_err_io)
                // done when the last byte is acknowledged (or the PC says file.done); the PC may also say it failed
                while (win.acked < s.size) {
                    val m = withTimeoutOrNull(FINISH_MS) { box.receive() } ?: return@holdRelay fail(ctx, id, R.string.tr_err_no_answer)
                    if (m is Msg.FileDone && m.ok) break
                    if (handleAnswer(m)) return@holdRelay
                }
                succeed(ctx, id, null)
            }
        } catch (e: IOException) {
            fail(ctx, id, R.string.tr_err_io)
        } finally {
            flows.remove(id)?.close()
        }
    }

    private suspend fun downloadViaRelay(ctx: Context, t: Transfer) {
        if (tooBigForRelay(t.size)) return fail(ctx, t.id, R.string.tr_err_relay_big, Link.relayMaxFileMB.value)
        val saved = try {
            Saver.open(ctx, t.name, t.mime)
        } catch (_: Exception) {
            return fail(ctx, t.id, R.string.tr_err_save)
        }
        val box = inbox(t.id)
        val rx = ChunkReceiver(t.size)
        var error: Int? = null
        try {
            Link.holdRelay {
                if (!Link.send(Msg.FileAccept(t.id))) { error = R.string.tr_err_io; return@holdRelay }
                var lastUi = 0L
                while (error == null) {
                    val m = withTimeoutOrNull(STALL_MS) { box.receive() }
                    when (m) {
                        null -> { Link.send(Msg.FileAbort(t.id, "stalled")); error = R.string.tr_err_io }
                        is Msg.FileChunk -> {
                            val data = ChunkFlow.decode(m.d)
                            if (data == null || rx.accept(m.off, data.size) != ChunkReceiver.Result.Ok) {
                                Link.send(Msg.FileAbort(t.id, "bad_chunk")); error = R.string.tr_err_io
                            } else {
                                saved.out.write(data)
                                Link.send(Msg.FileAck(t.id, rx.received))
                                val now = System.currentTimeMillis()
                                if (now - lastUi > 150) { lastUi = now; progress(ctx, t.id, t.name, false, rx.received) }
                            }
                        }
                        is Msg.FileEnd -> {
                            if (rx.complete(m.size)) break
                            Link.send(Msg.FileAbort(t.id, "size_mismatch")); error = R.string.tr_err_io
                        }
                        is Msg.FileAbort -> error = R.string.tr_err_declined
                        else -> Unit
                    }
                }
            }
        } catch (e: IOException) {
            error = R.string.tr_err_save
        } finally {
            flows.remove(t.id)?.close()
        }
        if (error != null) {
            saved.abort()
            Link.send(Msg.FileDone(t.id, false))
            return fail(ctx, t.id, error!!)
        }
        saved.finish { uri -> succeed(ctx, t.id, uri) }
        Link.send(Msg.FileDone(t.id, true))
    }

    private const val STALL_MS = 45_000L
    private const val FINISH_MS = 60_000L

    // ---- PC -> phone ----

    fun handle(ctx: Context, m: Msg) {
        when (m) {
            is Msg.FileChunk -> flows[m.id]?.trySend(m)
            is Msg.FileAck -> flows[m.id]?.trySend(m)
            is Msg.FileEnd -> flows[m.id]?.trySend(m)
            is Msg.FileAbort -> flows[m.id]?.trySend(m)
            is Msg.FileDone -> flows[m.id]?.trySend(m)
            is Msg.FileAccept -> answers[m.id]?.complete(m)
            is Msg.FileReject -> answers[m.id]?.complete(m)
            is Msg.FileOffer -> {
                val name = FileNames.sanitize(m.name)
                add(Transfer(m.id, name, m.size, false, TransferPhase.Offered, mime = m.mime))
                Notifs.offered(ctx, m.id, name, m.size)
                if (Store.autoReceive.value) accept(ctx, m.id)
            }
            else -> Unit
        }
    }

    /** The user (button or notification) wants an offered file. */
    fun accept(ctx: Context, id: String) {
        val t = _items.value.firstOrNull { it.id == id && !it.outgoing && it.phase == TransferPhase.Offered } ?: return
        patch(id) { it.copy(phase = TransferPhase.Active) }
        val app = ctx.applicationContext
        PillApp.scope.launch(Dispatchers.IO) { download(app, t) }
    }

    private suspend fun download(ctx: Context, t: Transfer) {
        val p = Store.paired.value ?: return fail(ctx, t.id, R.string.tr_err_not_connected)
        if (Link.route == Route.Relay) return downloadViaRelay(ctx, t)
        val saved = try {
            Saver.open(ctx, t.name, t.mime)
        } catch (_: Exception) {
            Link.send(Msg.FileDone(t.id, false))
            return fail(ctx, t.id, R.string.tr_err_save)
        }
        var written = 0L
        var failures = 0
        var error: Int? = null
        var errorArg = 0
        var lastUi = 0L
        try {
            while (written < t.size) {
                val req = PcApi.authed(p, "/api/down/${t.id}").apply { if (written > 0) header("Range", "bytes=$written-") }.build()
                try {
                    PcApi.transfer.newCall(req).await().use { r ->
                        if (!r.isSuccessful) { error = R.string.tr_err_server; errorArg = r.code; failures = Int.MAX_VALUE; return@use }
                        // A resumed request must be answered with 206; a plain 200 would restart the file.
                        if (written > 0 && r.code != 206) { error = R.string.tr_err_io; failures = Int.MAX_VALUE; return@use }
                        val input = r.body?.byteStream() ?: throw IOException("empty body")
                        val buf = ByteArray(64 * 1024)
                        while (written < t.size) {
                            val n = input.read(buf)
                            if (n < 0) throw IOException("connection closed early")
                            val take = minOf(n.toLong(), t.size - written).toInt()
                            saved.out.write(buf, 0, take)
                            written += take
                            val now = System.currentTimeMillis()
                            if (now - lastUi > 150) { lastUi = now; progress(ctx, t.id, t.name, false, written) }
                        }
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: IOException) {
                    if (++failures > 3) { error = R.string.tr_err_io; break }
                    delay(1000L * failures)   // brief pause, then resume from where we stopped with a Range request
                    continue
                }
                if (error != null) break
            }
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            error = R.string.tr_err_save
        }
        if (error != null || written < t.size) {
            saved.abort()
            Link.send(Msg.FileDone(t.id, false))
            return fail(ctx, t.id, error ?: R.string.tr_err_io, errorArg)
        }
        saved.finish { uri -> succeed(ctx, t.id, uri) }
        Link.send(Msg.FileDone(t.id, true))
    }

    // ---- state ----

    fun open(ctx: Context, id: String) {
        val t = _items.value.firstOrNull { it.id == id } ?: return
        val uri = t.uri ?: return
        val i = Intent(Intent.ACTION_VIEW).setDataAndType(uri, Saver.guessMime(t.name, t.mime))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
        try { ctx.startActivity(i) } catch (_: Exception) { }
    }

    private fun add(t: Transfer) = _items.update { (listOf(t) + it).take(KEEP) }

    private fun patch(id: String, f: (Transfer) -> Transfer) = _items.update { l -> l.map { if (it.id == id) f(it) else it } }

    private fun progress(ctx: Context, id: String, name: String, outgoing: Boolean, bytes: Long) {
        patch(id) { it.copy(bytes = bytes) }
        val t = _items.value.firstOrNull { it.id == id } ?: return
        Notifs.progress(ctx, id, name, outgoing, t.progress)
    }

    private fun succeed(ctx: Context, id: String, uri: Uri?) {
        patch(id) { it.copy(phase = TransferPhase.Done, bytes = it.size, uri = uri ?: it.uri) }
        val t = _items.value.firstOrNull { it.id == id } ?: return
        Notifs.finished(ctx, id, t.name, t.outgoing, true, uri, t.mime, null)
    }

    private fun fail(ctx: Context, id: String, @StringRes res: Int, arg: Int = 0) {
        patch(id) { it.copy(phase = TransferPhase.Failed, error = res, errorArg = arg) }
        val t = _items.value.firstOrNull { it.id == id } ?: return
        Notifs.finished(ctx, id, t.name, t.outgoing, false, null, t.mime, ctx.getString(res, arg))
    }
}
