package app.pill.phone.service

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.os.Build
import android.os.Bundle
import android.service.notification.StatusBarNotification
import android.util.Log
import androidx.core.app.NotificationCompat
import app.pill.phone.data.AppFilter
import app.pill.phone.data.Store
import app.pill.phone.net.Link
import app.pill.phone.protocol.Msg
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import app.pill.phone.PillApp
import java.util.concurrent.ConcurrentHashMap

/**
 * Phone notifications <-> PC. Builds `notif.posted` / `notif.removed` from what the listener sees and carries out what
 * the PC asks (`notif.dismiss`, `notif.read`, `notif.reply`). Nothing is queued while the PC is away: the protocol has the
 * app replay the active notifications on reconnect instead.
 */
object NotifBridge {
    private const val TAG = "PillReply"

    /** What we need to act on a notification later; lives in memory only (a reply after a process restart is "expired"). */
    private class Entry(val reply: Notification.Action?, val read: PendingIntent?)

    private val entries = ConcurrentHashMap<String, Entry>()
    private val sentHash = ConcurrentHashMap<String, Int>()
    private val appIcons = ConcurrentHashMap<String, String?>()

    private val READ_TITLE = Regex("mark.*read|^read$|прочитан|прочесть", RegexOption.IGNORE_CASE)

    fun posted(ctx: Context, sbn: StatusBarNotification, replay: Boolean) {
        if (!Link.isConnected) return
        val (msg, entry) = build(ctx, sbn, replay) ?: return
        if (!replay && sentHash[msg.key] == msg.hashCode()) return   // same content again (progress ticks, re-posts)
        entries[msg.key] = entry
        if (Link.send(msg)) sentHash[msg.key] = msg.hashCode()
    }

    fun removed(key: String) {
        entries.remove(key)
        if (sentHash.remove(key) != null) Link.send(Msg.NotifRemoved(key))
    }

    /** On (re)connect: the PC has seen nothing, so every active notification goes out again, flagged as a replay. */
    fun replayAll(ctx: Context) {
        val l = PillNotificationListener.instance ?: return
        sentHash.clear()
        val active = try { l.activeNotifications } catch (_: Exception) { null } ?: return
        for (sbn in active) posted(ctx, sbn, replay = true)
    }

    fun handle(ctx: Context, m: Msg) {
        when (m) {
            is Msg.Hello -> PillApp.scope.launch(Dispatchers.Default) { replayAll(ctx) }
            is Msg.NotifDismiss -> PillNotificationListener.instance?.cancelNotificationSafe(m.key)
            is Msg.NotifRead -> read(m.key)
            is Msg.NotifReply -> {
                val err = reply(ctx, m.key, m.text)
                Link.send(Msg.ReplyResult(m.req, err == null, err))
            }
            else -> Unit
        }
    }

    private fun read(key: String) {
        val pi = entries[key]?.read
        if (pi != null) {
            try { pi.send(); return } catch (_: PendingIntent.CanceledException) { }
        }
        PillNotificationListener.instance?.cancelNotificationSafe(key)
    }

    /** Null on success, otherwise the protocol's error word. */
    private fun reply(ctx: Context, key: String, text: String): String? {
        val e = entries[key] ?: return "expired"
        val action = e.reply ?: return "no_action"
        val inputs = action.remoteInputs ?: return "no_action"
        val results = Bundle().also { b -> inputs.forEach { b.putCharSequence(it.resultKey, text) } }
        val fill = Intent()
        RemoteInput.addResultsToIntent(inputs, fill, results)
        // diagnostics for "the answer does not reach the other person": who owns the action, which keys, and how the delivery ended (never the text)
        Log.i(TAG, "reply -> ${action.actionIntent.creatorPackage} keys=${inputs.map { it.resultKey }} len=${text.length} source=${if (Build.VERSION.SDK_INT >= 28) RemoteInput.getResultsSource(fill) else -1}")
        return try {
            action.actionIntent.send(ctx, 0, fill, PendingIntent.OnFinished { _, _, code, _, _ -> Log.i(TAG, "reply delivered to the app, result code $code") }, null)
            null
        } catch (_: PendingIntent.CanceledException) {
            Log.w(TAG, "reply: the notification's action was cancelled")
            "expired"
        }
    }

    // ---- building ----

    private fun build(ctx: Context, sbn: StatusBarNotification, replay: Boolean): Pair<Msg.NotifPosted, Entry>? {
        val pkg = sbn.packageName
        val n = sbn.notification
        if (pkg == AppFilter.SELF) return null
        if (n.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        val ongoing = n.flags and (Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE) != 0
        if (ongoing && !Store.includeOngoing.value) return null

        val pm = ctx.packageManager
        val info = try { pm.getApplicationInfo(pkg, 0) } catch (_: PackageManager.NameNotFoundException) { null }
        val system = info != null && info.flags and ApplicationInfo.FLAG_SYSTEM != 0 && info.flags and ApplicationInfo.FLAG_UPDATED_SYSTEM_APP == 0
        if (!AppFilter.isEnabled(pkg, system, Store.overrides.value)) return null

        val extras = n.extras
        val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n)
        val last = style?.messages?.lastOrNull()

        val title = (extras.getCharSequence(Notification.EXTRA_TITLE_BIG) ?: extras.getCharSequence(Notification.EXTRA_TITLE))?.toString()
            ?: style?.conversationTitle?.toString() ?: ""
        val body = if (last != null) {
            val who = last.person?.name?.toString()
            val t = last.text?.toString().orEmpty()
            if (style.isGroupConversation && !who.isNullOrBlank()) "$who: $t" else t
        } else {
            (extras.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: extras.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
        }
        if (title.isBlank() && body.isBlank()) return null

        val max = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0)
        val progress = if (max > 0 && !extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE, false)) {
            (extras.getInt(Notification.EXTRA_PROGRESS, 0) * 100 / max).coerceIn(0, 100)
        } else null

        var reply: Notification.Action? = null
        var read: PendingIntent? = null
        val others = ArrayList<Msg.NotifPosted.Action>()
        n.actions.orEmpty().forEachIndexed { i, a ->
            val title0 = a.title?.toString().orEmpty()
            val canReply = a.remoteInputs?.any { it.allowFreeFormInput } == true
            val semantic = if (Build.VERSION.SDK_INT >= 28) a.semanticAction else 0
            when {
                canReply && reply == null -> reply = a
                semantic == Notification.Action.SEMANTIC_ACTION_MARK_AS_READ || READ_TITLE.containsMatchIn(title0) -> {
                    if (read == null) read = a.actionIntent
                    others += Msg.NotifPosted.Action("a$i", title0)
                }
                title0.isNotBlank() -> others += Msg.NotifPosted.Action("a$i", title0)
            }
        }

        val avatar = avatar(ctx, n, last?.person?.icon?.toIcon(ctx))
        val msg = Msg.NotifPosted(
            key = sbn.key,
            pkg = pkg,
            app = try { info?.let { pm.getApplicationLabel(it).toString() } } catch (_: Exception) { null } ?: pkg,
            title = title.take(200),
            text = body.take(2000),
            sub = extras.getCharSequence(Notification.EXTRA_SUB_TEXT)?.toString().orEmpty().take(200),
            whenMs = if (n.`when` > 0) n.`when` else sbn.postTime,
            category = category(n.category),
            ongoing = ongoing,
            progress = progress,
            icon = appIcons.getOrPut(pkg) { appIcon(pm, pkg) },
            avatar = avatar,
            canReply = reply != null,
            actions = others,
            group = n.group,
            replay = replay,
        )
        return msg to Entry(reply, read)
    }

    fun category(c: String?): String = when (c) {
        Notification.CATEGORY_MESSAGE -> "msg"
        Notification.CATEGORY_CALL, Notification.CATEGORY_MISSED_CALL -> "call"
        Notification.CATEGORY_EMAIL -> "email"
        Notification.CATEGORY_SOCIAL -> "social"
        Notification.CATEGORY_EVENT, Notification.CATEGORY_ALARM, Notification.CATEGORY_REMINDER -> "event"
        Notification.CATEGORY_PROGRESS -> "progress"
        Notification.CATEGORY_TRANSPORT -> "transport"
        Notification.CATEGORY_SYSTEM, Notification.CATEGORY_SERVICE, Notification.CATEGORY_STATUS, Notification.CATEGORY_ERROR -> "system"
        else -> "other"
    }

    private fun appIcon(pm: PackageManager, pkg: String): String? = try {
        IconCodec.toBase64Png(pm.getApplicationIcon(pkg), square = true)
    } catch (_: Exception) {
        null
    }

    /** The sender's picture (messaging style) or the large icon, as a small PNG; null when there is none. */
    private fun avatar(ctx: Context, n: Notification, sender: android.graphics.drawable.Icon?): String? = try {
        val icon = sender ?: n.getLargeIcon()
        val d: Drawable? = icon?.loadDrawable(ctx)
        d?.let { IconCodec.toBase64Png(it, square = false) }
    } catch (_: Exception) {
        null
    }
}
