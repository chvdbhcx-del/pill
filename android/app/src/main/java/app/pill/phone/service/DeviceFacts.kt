package app.pill.phone.service

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.Build
import android.os.Environment
import android.os.PowerManager
import android.os.StatFs
import android.provider.Settings
import android.telephony.TelephonyManager
import app.pill.phone.protocol.Msg
import java.util.TimeZone

/**
 * The optional part of the protocol's `status` message (docs/phone-protocol.md, v1.1): charging details, temperature, storage, memory,
 * connection details, airplane / ringer / screen state and the time zone.
 *
 * Only what Android hands out without a special permission is read; every read can fail or be unknown and then simply stays out of the
 * message. Values that wobble all the time (free memory, free storage, temperature) are rounded to coarse steps so that a [Msg.Status]
 * only differs, and is only sent, when something a person would notice has changed.
 */
object DeviceFacts {
    private const val MB = 1024L * 1024L
    /** Free storage is reported in steps of this many bytes (half a GiB), free memory in 128 MB, temperature in half degrees. */
    private const val STORAGE_STEP = 512L * MB
    private const val RAM_STEP_MB = 128L

    // ---- pure mappers (unit tested) ----

    fun chargeOf(batteryStatus: Int): String = when (batteryStatus) {
        BatteryManager.BATTERY_STATUS_CHARGING -> "charging"
        BatteryManager.BATTERY_STATUS_FULL -> "full"
        else -> "none"
    }

    /** `EXTRA_PLUGGED`; a dock counts as the mains. Null when unplugged. */
    fun plugOf(plugged: Int): String? = when (plugged) {
        BatteryManager.BATTERY_PLUGGED_AC, 8 /* BATTERY_PLUGGED_DOCK */ -> "ac"
        BatteryManager.BATTERY_PLUGGED_USB -> "usb"
        BatteryManager.BATTERY_PLUGGED_WIRELESS -> "wireless"
        else -> null
    }

    fun healthOf(health: Int): String = when (health) {
        BatteryManager.BATTERY_HEALTH_GOOD -> "good"
        BatteryManager.BATTERY_HEALTH_OVERHEAT -> "overheat"
        BatteryManager.BATTERY_HEALTH_COLD -> "cold"
        BatteryManager.BATTERY_HEALTH_DEAD -> "dead"
        BatteryManager.BATTERY_HEALTH_OVER_VOLTAGE -> "over_voltage"
        BatteryManager.BATTERY_HEALTH_UNSPECIFIED_FAILURE -> "failure"
        else -> "unknown"
    }

    /** `EXTRA_TEMPERATURE` is in tenths of a degree; rounded to half degrees, null when the phone reports nothing sensible. */
    fun tempC(tenths: Int): Double? =
        if (tenths == Int.MIN_VALUE || tenths <= -400 || tenths > 1200) null else Math.round(tenths / 5.0) / 2.0

    /** Time to a full battery in whole minutes from the OS value in ms (-1 = unknown); null unless it is a real, plausible estimate. */
    fun fullInMin(ms: Long, charging: Boolean): Int? =
        if (!charging || ms <= 0 || ms > 7L * 24 * 3_600_000) null else maxOf(1L, (ms + 30_000) / 60_000).toInt()

    fun ringerOf(mode: Int): String = when (mode) {
        AudioManager.RINGER_MODE_SILENT -> "silent"
        AudioManager.RINGER_MODE_VIBRATE -> "vibrate"
        else -> "sound"
    }

    /** Free storage rounded to the step, never above the total. */
    fun space(free: Long, total: Long): Msg.Status.Space? =
        if (total <= 0 || free < 0) null else Msg.Status.Space((((free + STORAGE_STEP / 2) / STORAGE_STEP) * STORAGE_STEP).coerceAtMost(total), total)

    /** Memory in MB from bytes, free rounded to the step. */
    fun ram(freeBytes: Long, totalBytes: Long): Msg.Status.Space? {
        val total = totalBytes / MB
        val free = freeBytes / MB
        return if (total <= 0 || free < 0) null else Msg.Status.Space(((free + RAM_STEP_MB / 2) / RAM_STEP_MB * RAM_STEP_MB).coerceAtMost(total), total)
    }

    /** "4G" / "5G" ...; null for a type that says nothing. */
    fun genOf(networkType: Int): String? = when (networkType) {
        TelephonyManager.NETWORK_TYPE_NR -> "5G"
        TelephonyManager.NETWORK_TYPE_LTE, TelephonyManager.NETWORK_TYPE_IWLAN -> "4G"
        TelephonyManager.NETWORK_TYPE_HSPAP, TelephonyManager.NETWORK_TYPE_HSDPA, TelephonyManager.NETWORK_TYPE_HSUPA,
        TelephonyManager.NETWORK_TYPE_HSPA, TelephonyManager.NETWORK_TYPE_UMTS, TelephonyManager.NETWORK_TYPE_EVDO_0,
        TelephonyManager.NETWORK_TYPE_EVDO_A, TelephonyManager.NETWORK_TYPE_EVDO_B, TelephonyManager.NETWORK_TYPE_TD_SCDMA -> "3G"
        TelephonyManager.NETWORK_TYPE_GPRS, TelephonyManager.NETWORK_TYPE_EDGE, TelephonyManager.NETWORK_TYPE_CDMA,
        TelephonyManager.NETWORK_TYPE_1xRTT, TelephonyManager.NETWORK_TYPE_GSM -> "2G"
        else -> null
    }

    // ---- readers (need a Context; each is safe to fail) ----

    /** Everything above for [Msg.Status]: [net] is the transport ("wifi", "mobile", "none") the caller already knows. */
    fun fill(ctx: Context, battery: Intent?, base: Msg.Status, net: String): Msg.Status {
        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING
        val charge = if (battery == null) null else chargeOf(status)
        val tz = TimeZone.getDefault()
        return base.copy(
            charge = charge,
            plug = battery?.let { plugOf(it.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0)) },
            tempC = battery?.let { tempC(it.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE)) },
            health = battery?.let { healthOf(it.getIntExtra(BatteryManager.EXTRA_HEALTH, 0)) },
            fullInMin = if (charging) runCatching { fullInMin(ctx.getSystemService(BatteryManager::class.java).computeChargeTimeRemaining(), true) }.getOrNull() else null,
            storage = runCatching { StatFs(Environment.getDataDirectory().path).let { space(it.availableBytes, it.totalBytes) } }.getOrNull(),
            ram = runCatching {
                val mi = ActivityManager.MemoryInfo()
                ctx.getSystemService(ActivityManager::class.java).getMemoryInfo(mi)
                ram(mi.availMem, mi.totalMem)
            }.getOrNull(),
            net = netInfo(ctx, net),
            airplane = airplane(ctx),
            ringer = runCatching { ringerOf(ctx.getSystemService(AudioManager::class.java).ringerMode) }.getOrNull(),
            screen = runCatching { ctx.getSystemService(PowerManager::class.java).isInteractive }.getOrNull(),
            tz = tz.id,
            utcMin = tz.getOffset(System.currentTimeMillis()) / 60_000,
        )
    }

    fun airplane(ctx: Context): Boolean? =
        runCatching { Settings.Global.getInt(ctx.contentResolver, Settings.Global.AIRPLANE_MODE_ON, 0) == 1 }.getOrNull()

    /** Wi-Fi speed and signal (neither needs the location permission; only the SSID does), or carrier / generation / signal on mobile data. */
    @Suppress("DEPRECATION")
    private fun netInfo(ctx: Context, kind: String): Msg.Status.NetInfo = when (kind) {
        "wifi" -> runCatching {
            val wm = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            val i = wm.connectionInfo
            val onWifi = i != null && i.networkId != -1
            Msg.Status.NetInfo(
                type = kind,
                mbps = if (onWifi) i.linkSpeed.takeIf { it > 0 } else null,
                bars = if (onWifi) WifiManager.calculateSignalLevel(i.rssi, 5).coerceIn(0, 4) else null,
            )
        }.getOrDefault(Msg.Status.NetInfo(type = kind))
        "mobile" -> {
            val tm = ctx.getSystemService(TelephonyManager::class.java)
            Msg.Status.NetInfo(
                type = kind,
                carrier = runCatching { tm.networkOperatorName?.trim()?.takeIf { it.isNotEmpty() } }.getOrNull(),
                // the generation needs READ_PHONE_STATE on newer Android: a SecurityException just leaves it out
                gen = runCatching { genOf(tm.dataNetworkType) }.getOrNull(),
                bars = if (Build.VERSION.SDK_INT >= 28) runCatching { tm.signalStrength?.level?.coerceIn(0, 4) }.getOrNull() else null,
            )
        }
        else -> Msg.Status.NetInfo(type = kind, bars = 0)
    }
}
