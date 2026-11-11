package app.pill.phone.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.pill.phone.ClipSendActivity
import app.pill.phone.MainActivity
import app.pill.phone.R
import app.pill.phone.data.Paired
import app.pill.phone.data.Routing
import app.pill.phone.net.LinkState
import app.pill.phone.net.PcView
import app.pill.phone.net.Route
import app.pill.phone.protocol.FileNames

/** Every notification the app itself posts: the calm connection one and the file transfer ones. */
object Notifs {
    const val CH_LINK = "link"
    const val CH_FILES = "files"
    const val CH_INCOMING = "incoming"
    const val ID_LINK = 1
    private const val ID_BASE = 1000

    fun createChannels(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_LINK, ctx.getString(R.string.ch_link), NotificationManager.IMPORTANCE_LOW).apply {
            description = ctx.getString(R.string.ch_link_desc); setShowBadge(false)
        })
        nm.createNotificationChannel(NotificationChannel(CH_FILES, ctx.getString(R.string.ch_files), NotificationManager.IMPORTANCE_LOW).apply {
            description = ctx.getString(R.string.ch_files_desc); setShowBadge(false)
        })
        nm.createNotificationChannel(NotificationChannel(CH_INCOMING, ctx.getString(R.string.ch_incoming), NotificationManager.IMPORTANCE_DEFAULT).apply {
            description = ctx.getString(R.string.ch_incoming_desc)
        })
    }

    private fun openApp(ctx: Context, action: String? = null, req: Int = 0): PendingIntent {
        val i = Intent(ctx, MainActivity::class.java).apply {
            this.action = action ?: Intent.ACTION_MAIN
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        return PendingIntent.getActivity(ctx, req, i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }

    /** The foreground notification: one line about the single PC, or, with several, how many are reachable. */
    fun link(ctx: Context, pcs: List<Paired>, views: Map<String, PcView>, revoked: Boolean = false, clip: Boolean = false): Notification {
        if (pcs.size <= 1) {
            val p = pcs.firstOrNull()
            val state = if (revoked && p == null) LinkState.Revoked else views[p?.pcId]?.state ?: LinkState.Connecting
            return link(ctx, state, p?.label.orEmpty(), clip)
        }
        val states = pcs.map { views[it.pcId]?.state ?: LinkState.Connecting }
        val up = states.count { it is LinkState.Connected }
        val title = when (val s = Routing.summary(up, pcs.size)) {
            is Routing.Summary.All -> ctx.getString(R.string.notif_connected_all, s.count)
            is Routing.Summary.Some -> ctx.getString(R.string.notif_connected_some, s.connected, s.total)
            Routing.Summary.None -> when {
                states.all { it == LinkState.NoNetwork } -> ctx.getString(R.string.notif_nonetwork)
                states.any { it == LinkState.Connecting } -> ctx.getString(R.string.notif_connecting_all)
                else -> ctx.getString(R.string.notif_notfound_all)
            }
        }
        return linkBuilder(ctx, title).also { if (clip && up > 0) it.addClipAction(ctx) }.build()
    }

    fun link(ctx: Context, state: LinkState, pcName: String, clip: Boolean = false): Notification {
        val title = when (state) {
            is LinkState.Connected -> ctx.getString(
                if (state.route == Route.Relay) R.string.notif_connected_relay else R.string.notif_connected,
                state.pcName.ifBlank { pcName },
            )
            LinkState.Connecting, LinkState.Unpaired -> ctx.getString(R.string.notif_connecting, pcName)
            LinkState.NoNetwork -> ctx.getString(R.string.notif_nonetwork)
            LinkState.NotFound -> ctx.getString(R.string.notif_notfound)
            LinkState.PcOffline -> ctx.getString(R.string.notif_pc_offline)
            LinkState.Revoked -> ctx.getString(R.string.notif_revoked)
        }
        val b = linkBuilder(ctx, title)
        if (clip && state is LinkState.Connected) b.addClipAction(ctx)
        if (state == LinkState.NotFound) b.addAction(0, ctx.getString(R.string.act_scan_again), openApp(ctx, MainActivity.ACTION_SCAN, 1))
        if (state == LinkState.Revoked) b.addAction(0, ctx.getString(R.string.act_pair_again), openApp(ctx, MainActivity.ACTION_SCAN, 1))
        return b.build()
    }

    /** One PC out of several removed this phone: a quiet note, the link to the others goes on. */
    fun pcRemoved(ctx: Context, pcName: String) {
        val n = NotificationCompat.Builder(ctx, CH_FILES)
            .setSmallIcon(R.drawable.ic_stat_pill)
            .setContentTitle(ctx.getString(R.string.notif_pc_removed, pcName))
            .setContentIntent(openApp(ctx)).setAutoCancel(true).setSilent(true)
            .build()
        notify(ctx, ID_BASE - 1 - (pcName.hashCode() and 0xFF), n)
    }

    /** "Send clipboard" on the connection notification: opens the see-through activity that can read the clipboard. */
    private fun NotificationCompat.Builder.addClipAction(ctx: Context) {
        val i = Intent(ctx, ClipSendActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        addAction(0, ctx.getString(R.string.btn_send_clipboard), PendingIntent.getActivity(ctx, 2, i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
    }

    private fun linkBuilder(ctx: Context, title: String): NotificationCompat.Builder {
        val b = NotificationCompat.Builder(ctx, CH_LINK)
            .setSmallIcon(R.drawable.ic_stat_pill)
            .setContentTitle(title)
            .setContentIntent(openApp(ctx))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        return b
    }

    private fun canPost(ctx: Context) =
        Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun notify(ctx: Context, id: Int, n: Notification) {
        if (!canPost(ctx)) return
        try {
            NotificationManagerCompat.from(ctx).notify(id, n)
        } catch (_: SecurityException) {
        }
    }

    private fun idFor(transferId: String) = ID_BASE + (transferId.hashCode() and 0x3FFFFFFF) % 100000

    fun progress(ctx: Context, transferId: String, name: String, outgoing: Boolean, fraction: Float) {
        val n = NotificationCompat.Builder(ctx, CH_FILES)
            .setSmallIcon(if (outgoing) android.R.drawable.stat_sys_upload else android.R.drawable.stat_sys_download)
            .setContentTitle(ctx.getString(if (outgoing) R.string.notif_sending else R.string.notif_receiving, name))
            .setProgress(1000, (fraction * 1000).toInt().coerceIn(0, 1000), false)
            .setOngoing(true).setOnlyAlertOnce(true).setSilent(true).setShowWhen(false)
            .setContentIntent(openApp(ctx))
            .build()
        notify(ctx, idFor(transferId), n)
    }

    fun clear(ctx: Context, transferId: String) {
        NotificationManagerCompat.from(ctx).cancel(idFor(transferId))
    }

    /** "File from PC": tap or the Receive action starts the download. */
    fun offered(ctx: Context, transferId: String, name: String, size: Long, pcName: String? = null) {
        val receive = PendingIntent.getBroadcast(
            ctx, idFor(transferId),
            Intent(ctx, TransferActionReceiver::class.java).setAction(TransferActionReceiver.ACTION_RECEIVE).putExtra(TransferActionReceiver.EXTRA_ID, transferId),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val n = NotificationCompat.Builder(ctx, CH_INCOMING)
            .setSmallIcon(R.drawable.ic_stat_pill)
            .setContentTitle(if (pcName != null) ctx.getString(R.string.file_from_named_pc, pcName) else ctx.getString(R.string.file_from_pc))
            .setContentText("$name  ·  ${FileNames.humanSize(size)}")
            .setContentIntent(openApp(ctx)).setAutoCancel(true)
            .addAction(0, ctx.getString(R.string.btn_receive), receive)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .build()
        notify(ctx, idFor(transferId), n)
    }

    fun finished(ctx: Context, transferId: String, name: String, outgoing: Boolean, ok: Boolean, openUri: Uri?, mime: String, errorText: String?) {
        val b = NotificationCompat.Builder(ctx, CH_FILES)
            .setSmallIcon(R.drawable.ic_stat_pill)
            .setContentTitle(
                when {
                    !ok -> ctx.getString(R.string.notif_transfer_failed)
                    outgoing -> ctx.getString(R.string.notif_sent)
                    else -> ctx.getString(R.string.notif_received)
                },
            )
            .setContentText(if (ok) name else listOfNotNull(name, errorText).joinToString("  ·  "))
            .setAutoCancel(true).setOnlyAlertOnce(true).setSilent(true)
        if (ok && !outgoing && openUri != null) {
            val view = Intent(Intent.ACTION_VIEW).setDataAndType(openUri, mime.ifBlank { "*/*" })
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
            val pi = PendingIntent.getActivity(ctx, idFor(transferId), view, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            b.setContentIntent(pi).addAction(0, ctx.getString(R.string.btn_open), pi)
        } else {
            b.setContentIntent(openApp(ctx))
        }
        notify(ctx, idFor(transferId), b.build())
    }
}
