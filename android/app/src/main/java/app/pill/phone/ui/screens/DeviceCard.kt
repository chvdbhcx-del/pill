package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.animateIntAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AirplanemodeActive
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.DoNotDisturbOn
import androidx.compose.material.icons.rounded.SignalCellularAlt
import androidx.compose.material.icons.rounded.SignalWifiOff
import androidx.compose.material.icons.rounded.Smartphone
import androidx.compose.material.icons.rounded.Vibration
import androidx.compose.material.icons.rounded.VolumeOff
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.protocol.FileNames
import app.pill.phone.protocol.Msg
import app.pill.phone.ui.components.Chip
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.SpringProgress
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

/**
 * "This phone": the facts the PC sees, in the same tiles as the island's Phone tab. Battery with charge line and
 * health, storage and memory bars, the network with its signal, and chips for the modes that are on. Everything that is
 * unknown (an older Android, a missing permission) is just left out; every number and bar glides to its new value.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DeviceCard(s: Msg.Status, modifier: Modifier = Modifier) {
    val c = Pill.colors
    GlassCard(modifier.animateContentSize(Motion.soft()), contentPadding = PaddingValues(18.dp)) {
        val charging = s.charge == "charging" || (s.charge == null && s.charging)
        val pct by animateIntAsState(s.battery, Motion.soft(), label = "battery")
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBadge(if (charging) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryStd, tint = if (charging) c.good else c.accentSoft)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text("$pct%", style = MaterialTheme.typography.titleLarge, color = c.text)
                Spacer(Modifier.height(8.dp))
                SpringProgress(s.battery / 100f)
            }
        }
        Spacer(Modifier.height(10.dp))
        Text(chargeLine(s, charging), style = MaterialTheme.typography.bodySmall, color = if (charging) c.good else c.textDim)
        batteryDetail(s)?.let {
            Spacer(Modifier.height(2.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = c.textDim)
        }
        if (s.storage != null || s.ram != null) {
            Spacer(Modifier.height(14.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                s.storage?.let { UsageTile(stringResource(R.string.dev_storage), it.free, it.total, Modifier.weight(1f)) }
                s.ram?.let { UsageTile(stringResource(R.string.dev_memory), it.free * MB, it.total * MB, Modifier.weight(1f)) }
            }
        }
        netInfoOf(s)?.let { n ->
            Spacer(Modifier.height(10.dp))
            NetTile(s, n)
        }
        val ringer = s.ringer?.takeIf { it != "sound" }
        if (s.dnd || s.airplane == true || ringer != null || s.screen != null) {
            Spacer(Modifier.height(10.dp))
            FlowRow {
                ModeChip(s.dnd, stringResource(R.string.dev_dnd), Icons.Rounded.DoNotDisturbOn)
                ModeChip(s.airplane == true, stringResource(R.string.dev_airplane), Icons.Rounded.AirplanemodeActive)
                ModeChip(ringer != null, stringResource(if (ringer == "silent") R.string.dev_ringer_silent else R.string.dev_ringer_vibrate), if (ringer == "silent") Icons.Rounded.VolumeOff else Icons.Rounded.Vibration)
                ModeChip(s.screen != null, stringResource(if (s.screen == false) R.string.dev_screen_off else R.string.dev_screen_on), Icons.Rounded.Smartphone, dot = if (s.screen == true) c.good else null)
            }
        }
    }
}

private const val MB = 1024L * 1024L

@Composable
private fun ModeChip(show: Boolean, text: String, icon: androidx.compose.ui.graphics.vector.ImageVector, dot: androidx.compose.ui.graphics.Color? = null) {
    // the gap is part of the chip, so a hidden chip leaves no hole in the row
    AnimatedVisibility(show, enter = Motion.enterItem, exit = Motion.exitItem) { Chip(text, Modifier.padding(end = 8.dp, bottom = 8.dp), icon = icon, dot = dot) }
}

@Composable
private fun UsageTile(label: String, free: Long, total: Long, modifier: Modifier) {
    val c = Pill.colors
    val shape = RoundedCornerShape(18.dp)
    val used = if (total > 0) 1f - free.toFloat() / total else 0f
    Column(modifier.clip(shape).background(c.surfaceHigh).border(1.dp, c.border, shape).padding(12.dp)) {
        Text(label.uppercase(), style = MaterialTheme.typography.labelSmall, color = c.textDim)
        Spacer(Modifier.height(4.dp))
        Text(stringResource(R.string.dev_free, FileNames.humanSize(free)), style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(stringResource(R.string.dev_of, FileNames.humanSize(total)), style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 1)
        Spacer(Modifier.height(8.dp))
        SpringProgress(used, height = 6.dp)
    }
}

@Composable
private fun NetTile(s: Msg.Status, n: NetLine) {
    val c = Pill.colors
    val shape = RoundedCornerShape(18.dp)
    Row(Modifier.fillMaxWidth().clip(shape).background(c.surfaceHigh).border(1.dp, c.border, shape).padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(n.icon, null, Modifier.size(22.dp), tint = c.textDim)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(n.title, style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            n.detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        n.bars?.let { SignalBars(it) }
    }
}

/** Four bars, lit up to [level] (0..4); each bar fades to its new state. */
@Composable
fun SignalBars(level: Int, modifier: Modifier = Modifier) {
    val c = Pill.colors
    val on = c.text
    val off = c.border
    val a = (0..3).map { animateFloatAsState(if (level > it) 1f else 0f, Motion.calm(), label = "bar$it").value }
    Canvas(modifier.size(width = 22.dp, height = 18.dp)) {
        val w = 4.dp.toPx()
        val gap = 2.dp.toPx()
        for (i in 0..3) {
            val h = (6 + 4 * i).dp.toPx()
            val color = androidx.compose.ui.graphics.lerp(off, on, a[i] * 0.9f)
            drawRoundRect(color, Offset(i * (w + gap), size.height - h), Size(w, h), CornerRadius(1.5.dp.toPx()))
        }
    }
}

/** What the network tile shows. */
data class NetLine(val icon: androidx.compose.ui.graphics.vector.ImageVector, val title: String, val detail: String?, val bars: Int?)

@Composable
private fun netInfoOf(s: Msg.Status): NetLine? {
    if (s.airplane == true) return NetLine(Icons.Rounded.AirplanemodeActive, stringResource(R.string.dev_airplane), null, 0)
    val n = s.net
    return when (s.network) {
        "wifi" -> NetLine(
            Icons.Rounded.Wifi, s.ssid?.takeIf { it.isNotBlank() && it != "<unknown ssid>" } ?: stringResource(R.string.net_wifi),
            n?.mbps?.let { stringResource(R.string.dev_wifi_speed, it) }, n?.bars,
        )
        "mobile" -> NetLine(Icons.Rounded.SignalCellularAlt, n?.carrier ?: stringResource(R.string.net_mobile), n?.gen, n?.bars)
        else -> NetLine(Icons.Rounded.SignalWifiOff, stringResource(R.string.net_none), null, 0)
    }
}

@Composable
private fun durationText(minutes: Int): String =
    if (minutes >= 60) stringResource(R.string.dev_dur_hm, minutes / 60, minutes % 60) else stringResource(R.string.dev_dur_m, minutes)

/** "Charging · mains · 42 min to full", "Fully charged", "On battery". */
@Composable
private fun chargeLine(s: Msg.Status, charging: Boolean): String {
    if (s.charge == "full") return stringResource(R.string.dev_full)
    if (!charging) return stringResource(R.string.dev_on_battery)
    val parts = mutableListOf(stringResource(R.string.dev_charging))
    when (s.plug) {
        "ac" -> parts += stringResource(R.string.dev_plug_ac)
        "usb" -> parts += stringResource(R.string.dev_plug_usb)
        "wireless" -> parts += stringResource(R.string.dev_plug_wireless)
    }
    s.fullInMin?.takeIf { it > 0 }?.let { parts += stringResource(R.string.dev_to_full, durationText(it)) }
    return parts.joinToString(" · ")
}

/** "31.5 °C · health good", or null when the phone told neither. */
@Composable
private fun batteryDetail(s: Msg.Status): String? {
    val parts = mutableListOf<String>()
    s.tempC?.let { parts += stringResource(R.string.dev_temp, String.format(java.util.Locale.ROOT, "%.1f", it)) }
    val health = when (s.health) {
        "good" -> R.string.dev_health_good
        "overheat" -> R.string.dev_health_overheat
        "cold" -> R.string.dev_health_cold
        "dead" -> R.string.dev_health_dead
        "over_voltage" -> R.string.dev_health_over_voltage
        "failure" -> R.string.dev_health_failure
        else -> null
    }
    health?.let { parts += stringResource(R.string.dev_health, stringResource(it)) }
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}
