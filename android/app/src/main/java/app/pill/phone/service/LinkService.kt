package app.pill.phone.service

import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.IBinder
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.pill.phone.data.Paired
import app.pill.phone.data.Store
import app.pill.phone.net.LinkManager
import app.pill.phone.net.PcView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

private data class LinkSnap(val views: Map<String, PcView>, val pcs: List<Paired>, val revoked: List<String>, val clip: Boolean)

/**
 * Keeps the link to the PC alive: owns the WebSocket (via [Link]), shows the calm "Connected to ..." notification,
 * follows the network and reports the phone status. Started when the app opens, after pairing and at boot.
 *
 * Type note: Android 15 limits `dataSync` services to 6 hours a day, which would cut an always-on link. From Android 14
 * the service therefore runs as `specialUse` (no limit); older versions use `dataSync` as the manifest also declares.
 */
class LinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var netCallback: ConnectivityManager.NetworkCallback? = null
    private var lastNetwork: Network? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val pcs = Store.pcs.value
        enterForeground(Notifs.link(this, pcs, emptyMap()))
        if (pcs.isEmpty()) {
            stopSelf()
            return
        }
        LinkManager.start(scope)
        StatusReporter.start(this, scope)
        watchNetwork()
        scope.launch {
            // The headline follows all PCs: "Connected to AENEAS", "Connected to 2 PCs", "1 of 2 PCs reachable".
            combine(LinkManager.views, Store.pcs, LinkManager.revoked, Store.clipEnabled) { v, p, r, c -> LinkSnap(v, p, r, c) }.collect { (views, list, revoked, clip) ->
                if (list.isEmpty() && revoked.isEmpty()) { stopSelf(); return@collect }
                val n = Notifs.link(this@LinkService, list, views, revoked = revoked.isNotEmpty(), clip = clip)
                getSystemService(NotificationManager::class.java).notify(Notifs.ID_LINK, n)
                // Every PC removed us: nothing left to keep alive, the notification stays only until the user looks.
                if (list.isEmpty()) stopForegroundKeepingNotification()
            }
        }
        scope.launch {
            // One of several PCs removed this phone: say so once, the others keep running
            var seen = 0
            LinkManager.revoked.collect { names ->
                if (names.size > seen && Store.pcs.value.isNotEmpty()) Notifs.pcRemoved(this@LinkService, names.last())
                seen = names.size
            }
        }
    }

    private fun enterForeground(n: android.app.Notification) {
        val type = when {
            Build.VERSION.SDK_INT >= 34 -> ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            Build.VERSION.SDK_INT >= 29 -> ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            else -> 0
        }
        ServiceCompat.startForeground(this, Notifs.ID_LINK, n, type)
    }

    private fun stopForegroundKeepingNotification() {
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_DETACH)
        stopSelf()
    }

    private fun watchNetwork() {
        val cm = getSystemService(ConnectivityManager::class.java) ?: return
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                val changed = lastNetwork != null && lastNetwork != network
                lastNetwork = network
                if (changed) LinkManager.networkChanged() else LinkManager.nudge()
                StatusReporter.poke()
            }

            override fun onLost(network: Network) {
                if (lastNetwork == network) lastNetwork = null
                StatusReporter.poke()
            }

            override fun onCapabilitiesChanged(network: Network, caps: android.net.NetworkCapabilities) = StatusReporter.poke()
        }
        try {
            cm.registerDefaultNetworkCallback(cb)
            netCallback = cb
        } catch (_: Exception) {
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    // Android 15 calls this when a dataSync budget runs out; we do not run as dataSync there, but stay correct if it does.
    override fun onTimeout(startId: Int, fgsType: Int) {
        stopSelf()
    }

    override fun onDestroy() {
        netCallback?.let { runCatching { getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(it) } }
        StatusReporter.stop()
        LinkManager.stop()
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        /** Start (or keep) the link. Safe to call from anywhere; if the system forbids it right now, it is simply skipped. */
        fun start(ctx: Context) {
            if (Store.pcs.value.isEmpty()) return
            try {
                ContextCompat.startForegroundService(ctx, Intent(ctx, LinkService::class.java))
            } catch (_: Exception) {
                // Background start not allowed at this moment (Android 12+): the next app open or boot starts it.
            }
        }

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, LinkService::class.java))
        }
    }
}
