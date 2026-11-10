package app.pill.phone.service

import android.Manifest
import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.wifi.WifiManager
import android.os.BatteryManager
import androidx.core.content.ContextCompat
import app.pill.phone.net.Link
import app.pill.phone.net.Net
import app.pill.phone.protocol.Msg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/**
 * The protocol's `status` message: battery, charging, network, SSID (only with location permission) and Do Not Disturb.
 * Sent on connect and when something changes, at most once per 10 s. Also feeds the home screen chips.
 */
@SuppressLint("StaticFieldLeak") // holds only the application context
object StatusReporter {
    private const val MIN_GAP_MS = 10_000L

    private val _current = MutableStateFlow<Msg.Status?>(null)
    val current: StateFlow<Msg.Status?> = _current

    private val trigger = Channel<Unit>(Channel.CONFLATED)
    @Volatile private var forceNext = false
    private var receiver: BroadcastReceiver? = null
    private var appCtx: Context? = null

    fun start(ctx: Context, scope: CoroutineScope) {
        val app = ctx.applicationContext
        appCtx = app
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) { poke() }
        }
        receiver = r
        val f = IntentFilter().apply {
            addAction(Intent.ACTION_BATTERY_CHANGED)
            addAction(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED)
        }
        ContextCompat.registerReceiver(app, r, f, ContextCompat.RECEIVER_NOT_EXPORTED)
        scope.launch {
            var lastAt = 0L
            var lastSent: Msg.Status? = null
            while (true) {
                trigger.receive()
                if (forceNext) { lastSent = null; lastAt = 0L; forceNext = false }
                val wait = lastAt + MIN_GAP_MS - System.currentTimeMillis()
                if (wait > 0) delay(wait)
                val s = snapshot(app)
                _current.value = s
                if (Link.isConnected && s.copy(time = 0) != lastSent?.copy(time = 0) && Link.send(s)) {
                    lastSent = s
                    lastAt = System.currentTimeMillis()
                }
            }
        }
        poke()
    }

    fun stop() {
        val r = receiver ?: return
        runCatching { appCtx?.unregisterReceiver(r) }
        receiver = null
    }

    fun poke() { trigger.trySend(Unit) }

    /** A new session began (hello received): the PC knows nothing yet, send the status straight away. */
    fun onHello() {
        forceNext = true
        poke()
    }

    fun snapshot(ctx: Context): Msg.Status {
        val b = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = b?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = b?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val status = b?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
        val net = Net.kind(ctx)
        val nm = ctx.getSystemService(NotificationManager::class.java)
        val dnd = nm.currentInterruptionFilter.let { it != NotificationManager.INTERRUPTION_FILTER_ALL && it != NotificationManager.INTERRUPTION_FILTER_UNKNOWN }
        return Msg.Status(
            battery = if (level >= 0 && scale > 0) level * 100 / scale else 0,
            charging = charging,
            network = net,
            ssid = if (net == "wifi") ssid(ctx) else null,
            dnd = dnd,
            time = System.currentTimeMillis(),
        )
    }

    @Suppress("DEPRECATION")
    private fun ssid(ctx: Context): String? {
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) return null
        return try {
            val wm = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            wm.connectionInfo?.ssid?.trim('"')?.takeIf { it.isNotBlank() && it != "<unknown ssid>" }
        } catch (_: Exception) {
            null
        }
    }
}
