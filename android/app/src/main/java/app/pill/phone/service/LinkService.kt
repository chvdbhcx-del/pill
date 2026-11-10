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
import app.pill.phone.data.Store
import app.pill.phone.net.Link
import app.pill.phone.net.LinkState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

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
        val paired = Store.paired.value
        enterForeground(Notifs.link(this, LinkState.Connecting, paired?.pcName.orEmpty()))
        if (paired == null) {
            stopSelf()
            return
        }
        Link.start(scope)
        StatusReporter.start(this, scope)
        watchNetwork()
        scope.launch {
            combine(Link.state, Store.paired) { s, p -> s to p }.collect { (state, p) ->
                if (p == null && state !is LinkState.Revoked) { stopSelf(); return@collect }
                val n = Notifs.link(this@LinkService, state, p?.pcName.orEmpty())
                getSystemService(NotificationManager::class.java).notify(Notifs.ID_LINK, n)
                // The PC revoked us: nothing left to keep alive, the notification stays only until the user looks.
                if (state is LinkState.Revoked) stopForegroundKeepingNotification()
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
                if (changed) Link.networkChanged() else Link.nudge()
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
        Link.stop()
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        /** Start (or keep) the link. Safe to call from anywhere; if the system forbids it right now, it is simply skipped. */
        fun start(ctx: Context) {
            if (Store.paired.value == null) return
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
