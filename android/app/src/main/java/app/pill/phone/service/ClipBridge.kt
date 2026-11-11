package app.pill.phone.service

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import app.pill.phone.R
import app.pill.phone.data.ClipSync
import app.pill.phone.data.ClipboardAccess
import app.pill.phone.data.Store
import app.pill.phone.net.LinkManager
import app.pill.phone.protocol.Msg

/** The real clipboard. Reading only works while one of our windows has focus (Android 10+); writing works from the background. */
private class AndroidClipboard(private val app: Context) : ClipboardAccess {
    private val main = Handler(Looper.getMainLooper())
    private val cm get() = app.getSystemService(ClipboardManager::class.java)

    override fun read(): String? = try {
        val clip = cm?.primaryClip
        val textual = clip?.description?.let { it.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN) || it.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML) } == true
        if (textual && clip!!.itemCount > 0) clip.getItemAt(0).coerceToText(app)?.toString() else null
    } catch (_: Exception) {
        null
    }

    override fun write(text: String) {
        // not marked sensitive: the user chose to share this text with their own PC
        main.post { try { cm?.setPrimaryClip(ClipData.newPlainText("", text)) } catch (_: Exception) { } }
    }
}

/** Clipboard sync glue: the one [ClipSync] of the process, the caps announcement and the incoming clips. Clip text is never logged. */
object ClipBridge {
    @Volatile private var inst: ClipSync? = null

    /** True while the app's main screen is visible: only then a quiet toast says the clipboard changed. */
    @Volatile var foreground = false

    fun sync(ctx: Context): ClipSync = inst ?: synchronized(this) {
        inst ?: ClipSync(AndroidClipboard(ctx.applicationContext), { Store.clipEnabled.value }, { Store.clipReceive.value }).also { inst = it }
    }

    /** A PC session just started: tell it we can do clip (nothing at all while the feature is off). */
    fun announce(ctx: Context, pcId: String) {
        sync(ctx).caps()?.let { LinkManager.send(pcId, it) }
    }

    /** The feature was just switched on: every PC that is up hears about it now, not at the next reconnect. */
    fun announceAll(ctx: Context) {
        for (id in LinkManager.connectedIds()) announce(ctx, id)
    }

    fun handle(ctx: Context, m: Msg) {
        if (m !is Msg.Clip) return
        if (sync(ctx).receive(m) && foreground) {
            val app = ctx.applicationContext
            Handler(Looper.getMainLooper()).post { Toast.makeText(app, R.string.clip_received_toast, Toast.LENGTH_SHORT).show() }
        }
    }
}
