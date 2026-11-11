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
import app.pill.phone.net.LinkManager
import app.pill.phone.net.PcLink
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
import java.util.concurrent.atomic.AtomicInteger

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
    /** The PC this file goes to / comes from. */
    val pcId: String = "",
) {
    val progress: Float get() = if (size > 0) (bytes.toFloat() / size).coerceIn(0f, 1f) else 0f
}

/**
 * A file picked on the phone, already opened so it can still be read after the share activity is gone. One source can go to
 * several PCs at once (send to all): it is reference counted and read by position, so uploads to different PCs do not share a file offset.
 */
class Source(val name: String, val size: Long, val mime: String, val pfd: ParcelFileDescriptor) {
    private val refs = AtomicInteger(1)
    private val channel by lazy { FileInputStream(pfd.fileDescriptor).channel }

    /** One more user of this file (one more PC it is sent to). */
    fun retain(n: Int = 1) { refs.addAndGet(n) }

    /** A user is done; the last one closes the descriptor. */
    fun release() { if (refs.decrementAndGet() <= 0) runCatching { pfd.close() } }

    /** Reads up to [len] bytes at [pos] into [buf]; -1 at the end. */
    fun readAt(buf: ByteArray, len: Int, pos: Long): Int = channel.read(java.nio.ByteBuffer.wrap(buf, 0, len), pos)
}

/**
 * File transfer in both directions (protocol "file.*" messages and /api/up, /api/down).
 * Phone -> PC: offer, wait for accept, stream a PUT. PC -> phone: show an offer, download with Range retries into Downloads/Pill.
 */
object Transfers {
    private val _items = MutableStateFlow<List<Transfer>>(emptyList())
    val items: StateFlow<List<Transfer>> = _items

    private val answers = ConcurrentHashMap<String, CompletableDeferred<Msg>>()
    /** One upload queue and worker per PC, so a PC that is slow to answer never holds up a file going to another. */
    private val uploadQueues = ConcurrentHashMap<String, Channel<suspend () -> Unit>>()

    /** Ids are only unique per PC, so every map of running transfers is keyed by both. */
    private fun key(pcId: String, id: String) = "$pcId|$id"

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

    /**
     * Queue every source for every PC in [pcIds] (one PC, or all of them for "send to all"). Each (file, PC) pair is its own
     * transfer with its own row, so one PC saying no or being offline leaves the others untouched.
     */
    fun send(ctx: Context, sources: List<Source>, pcIds: List<String>) {
        val app = ctx.applicationContext
        val targets = pcIds.distinct()
        if (targets.isEmpty()) { sources.forEach { it.release() }; return }
        sources.forEach { it.retain(targets.size - 1) }
        for (pcId in targets) {
            val queue = queueFor(pcId)
            for (s in sources) {
                val id = UUID.randomUUID().toString()
                add(Transfer(id, s.name, s.size, true, TransferPhase.Waiting, mime = s.mime, pcId = pcId))
                queue.trySend { upload(app, id, pcId, s) }
            }
        }
    }

    private fun queueFor(pcId: String): Channel<suspend () -> Unit> = uploadQueues.getOrPut(pcId) {
        Channel<suspend () -> Unit>(Channel.UNLIMITED).also { q ->
            PillApp.scope.launch(Dispatchers.IO) {
                for (job in q) {
                    try { job() } catch (e: CancellationException) { throw e } catch (_: Exception) { }
                }
            }
        }
    }

    private suspend fun upload(ctx: Context, id: String, pcId: String, s: Source) {
        try {
            val p = Store.pc(pcId)
            val link = LinkManager.link(pcId)
            if (p == null || link == null || !link.isConnected) return fail(ctx, id, R.string.tr_err_not_connected)
            if (link.route == Route.Relay && tooBigForRelay(link, s.size)) return fail(ctx, id, R.string.tr_err_relay_big, link.relayMaxFileMB.value)
            val answer = CompletableDeferred<Msg>().also { answers[key(pcId, id)] = it }
            link.send(Msg.FileOffer(id, s.name, s.size, s.mime.ifBlank { Saver.guessMime(s.name, "") }))
            val reply = withTimeoutOrNull(ANSWER_TIMEOUT_MS) { answer.await() }
            answers.remove(key(pcId, id))
            when (reply) {
                is Msg.FileAccept -> Unit
                is Msg.FileReject -> return fail(ctx, id, R.string.tr_err_declined)
                else -> return fail(ctx, id, R.string.tr_err_no_answer)
            }
            patch(id) { it.copy(phase = TransferPhase.Active) }
            Notifs.progress(ctx, id, s.name, true, 0f)
            if (link.route == Route.Relay) return uploadViaRelay(ctx, id, link, s)
            val body = object : RequestBody() {
                override fun contentType() = (s.mime.ifBlank { "application/octet-stream" }).toMediaTypeOrNull()
                override fun contentLength() = s.size
                override fun writeTo(sink: BufferedSink) {
                    val buf = ByteArray(64 * 1024)
                    var sent = 0L
                    var lastUi = 0L
                    while (sent < s.size) {
                        val n = s.readAt(buf, minOf(buf.size.toLong(), s.size - sent).toInt(), sent)
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
        } finally {
            s.release()
        }
    }

    // ---- through the relay: file.chunk / file.ack / file.end / file.abort (docs/phone-relay.md), no HTTP side channel ----

    private fun tooBigForRelay(link: PcLink, size: Long) = size > link.relayMaxFileMB.value * 1024L * 1024L

    /** The data messages of a running relay transfer, by PC and file id. */
    private val flows = ConcurrentHashMap<String, Channel<Msg>>()

    private fun inbox(pcId: String, id: String) = Channel<Msg>(Channel.UNLIMITED).also { flows[key(pcId, id)] = it }

    /** Streams the file from its descriptor; never more than the window of chunks in memory. */
    private suspend fun uploadViaRelay(ctx: Context, id: String, link: PcLink, s: Source) {
        val box = inbox(link.pcId, id)
        try {
            link.holdRelay {
                val win = SendWindow()
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
                        if (m == null) { link.send(Msg.FileAbort(id, "stalled")); return@holdRelay fail(ctx, id, R.string.tr_err_io) }
                        if (handleAnswer(m)) return@holdRelay
                    }
                    while (true) { val m = box.tryReceive().getOrNull() ?: break; if (handleAnswer(m)) return@holdRelay }
                    val n = s.readAt(buf, minOf(buf.size.toLong(), s.size - sent).toInt(), sent)
                    if (n < 0) { link.send(Msg.FileAbort(id, "short_file")); return@holdRelay fail(ctx, id, R.string.tr_err_io) }
                    if (!link.send(Msg.FileChunk(id, sent, ChunkFlow.encode(buf, n)))) return@holdRelay fail(ctx, id, R.string.tr_err_io)
                    sent += n
                    win.onSent(sent)
                    delay(ChunkFlow.paceDelayMs(n))
                }
                if (!link.send(Msg.FileEnd(id, s.size))) return@holdRelay fail(ctx, id, R.string.tr_err_io)
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
            flows.remove(key(link.pcId, id))?.close()
        }
    }

    private suspend fun downloadViaRelay(ctx: Context, t: Transfer, link: PcLink) {
        if (tooBigForRelay(link, t.size)) return fail(ctx, t.id, R.string.tr_err_relay_big, link.relayMaxFileMB.value)
        val saved = try {
            Saver.open(ctx, t.name, t.mime)
        } catch (_: Exception) {
            return fail(ctx, t.id, R.string.tr_err_save)
        }
        val box = inbox(t.pcId, t.id)
        val rx = ChunkReceiver(t.size)
        var error: Int? = null
        try {
            link.holdRelay {
                if (!link.send(Msg.FileAccept(t.id))) { error = R.string.tr_err_io; return@holdRelay }
                var lastUi = 0L
                while (error == null) {
                    val m = withTimeoutOrNull(STALL_MS) { box.receive() }
                    when (m) {
                        null -> { link.send(Msg.FileAbort(t.id, "stalled")); error = R.string.tr_err_io }
                        is Msg.FileChunk -> {
                            val data = ChunkFlow.decode(m.d)
                            if (data == null || rx.accept(m.off, data.size) != ChunkReceiver.Result.Ok) {
                                link.send(Msg.FileAbort(t.id, "bad_chunk")); error = R.string.tr_err_io
                            } else {
                                saved.out.write(data)
                                link.send(Msg.FileAck(t.id, rx.received))
                                val now = System.currentTimeMillis()
                                if (now - lastUi > 150) { lastUi = now; progress(ctx, t.id, t.name, false, rx.received) }
                            }
                        }
                        is Msg.FileEnd -> {
                            if (rx.complete(m.size)) break
                            link.send(Msg.FileAbort(t.id, "size_mismatch")); error = R.string.tr_err_io
                        }
                        is Msg.FileAbort -> error = R.string.tr_err_declined
                        else -> Unit
                    }
                }
            }
        } catch (e: IOException) {
            error = R.string.tr_err_save
        } finally {
            flows.remove(key(t.pcId, t.id))?.close()
        }
        if (error != null) {
            saved.abort()
            link.send(Msg.FileDone(t.id, false))
            return fail(ctx, t.id, error!!)
        }
        saved.finish { uri -> succeed(ctx, t.id, uri) }
        link.send(Msg.FileDone(t.id, true))
    }

    private const val STALL_MS = 45_000L
    private const val FINISH_MS = 60_000L

    // ---- PC -> phone ----

    /** A file message from [pcId]; it only ever touches that PC's own transfers. */
    fun handle(ctx: Context, pcId: String, m: Msg) {
        when (m) {
            is Msg.FileChunk -> flows[key(pcId, m.id)]?.trySend(m)
            is Msg.FileAck -> flows[key(pcId, m.id)]?.trySend(m)
            is Msg.FileEnd -> flows[key(pcId, m.id)]?.trySend(m)
            is Msg.FileAbort -> flows[key(pcId, m.id)]?.trySend(m)
            is Msg.FileDone -> flows[key(pcId, m.id)]?.trySend(m)
            is Msg.FileAccept -> answers[key(pcId, m.id)]?.complete(m)
            is Msg.FileReject -> answers[key(pcId, m.id)]?.complete(m)
            is Msg.FileOffer -> {
                val name = FileNames.sanitize(m.name)
                val pcs = Store.pcs.value
                val pc = pcs.firstOrNull { it.pcId == pcId }
                add(Transfer(m.id, name, m.size, false, TransferPhase.Offered, mime = m.mime, pcId = pcId))
                // the notification names the PC only when there is a choice of PCs
                Notifs.offered(ctx, m.id, name, m.size, pc?.label?.takeIf { pcs.size >= 2 })
                if (pc?.autoReceives(Store.autoReceive.value) == true) accept(ctx, m.id)
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
        val p = Store.pc(t.pcId)
        val link = LinkManager.link(t.pcId)
        if (p == null || link == null) return fail(ctx, t.id, R.string.tr_err_not_connected)
        if (link.route == Route.Relay) return downloadViaRelay(ctx, t, link)
        val saved = try {
            Saver.open(ctx, t.name, t.mime)
        } catch (_: Exception) {
            link.send(Msg.FileDone(t.id, false))
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
            link.send(Msg.FileDone(t.id, false))
            return fail(ctx, t.id, error ?: R.string.tr_err_io, errorArg)
        }
        saved.finish { uri -> succeed(ctx, t.id, uri) }
        link.send(Msg.FileDone(t.id, true))
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
