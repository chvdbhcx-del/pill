package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.animateIntAsState
import androidx.compose.foundation.Canvas
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AirplanemodeActive
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.DoNotDisturbOn
import androidx.compose.material.icons.rounded.SignalCellularAlt
import androidx.compose.material.icons.rounded.SignalWifiOff
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.VolumeOff
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
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
import app.pill.phone.ui.components.Divider
import app.pill.phone.ui.components.SpringProgress
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

/**
 * "This phone" as ONE slim glass row: battery (bolt while charging) with a thin bar, and the network with its signal.
 * A tap opens the details in place on a spring: charge and health, storage, memory and only the modes that are NOT the
 * usual ones (Do Not Disturb, airplane, silent). Everything unknown (an older Android, a missing permission) is left out.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DeviceCard(s: Msg.Status, expanded: Boolean, onToggle: () -> Unit, modifier: Modifier = Modifier) {
    val c = Pill.colors
    val charging = s.charge == "charging" || (s.charge == null && s.charging)
    val pct by animateIntAsState(s.battery, Motion.soft(), label = "battery")
    val net = netInfoOf(s)
    val chevron by animateFloatAsState(if (expanded) 180f else 0f, Motion.bouncy(), label = "chevron")
    GlassCard(
        modifier.fillMaxWidth().animateContentSize(Motion.soft()), onClick = onToggle,
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                if (charging) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryStd, null, Modifier.size(20.dp),
                tint = if (charging) c.good else c.textDim,
            )
            Spacer(Modifier.width(6.dp))
            Text("$pct%", style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1)
            Spacer(Modifier.width(10.dp))
            SpringProgress(s.battery / 100f, Modifier.width(44.dp), height = 4.dp)
            Spacer(Modifier.weight(1f))
            if (net != null) {
                Icon(net.icon, null, Modifier.size(18.dp), tint = c.textDim)
                Spacer(Modifier.width(6.dp))
                Text(net.title, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.bodyMedium, color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis)
                net.bars?.let { Spacer(Modifier.width(8.dp)); SignalBars(it) }
            }
            Spacer(Modifier.width(6.dp))
            Icon(
                Icons.Rounded.KeyboardArrowDown,
                stringResource(if (expanded) R.string.dev_details_hide else R.string.dev_details_show),
                Modifier.size(22.dp).graphicsLayer { rotationZ = chevron }, tint = c.textDim,
            )
        }
        AnimatedVisibility(expanded, enter = Motion.enterItem, exit = Motion.exitItem) {
            Column {
                Spacer(Modifier.height(14.dp))
                Divider()
                Spacer(Modifier.height(14.dp))
                Text(chargeLine(s, charging), style = MaterialTheme.typography.bodyMedium, color = if (charging) c.good else c.text)
                batteryDetail(s)?.let {
                    Spacer(Modifier.height(2.dp))
                    Text(it, style = MaterialTheme.typography.bodySmall, color = c.textDim)
                }
                if (s.storage != null || s.ram != null) {
                    Spacer(Modifier.height(16.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        s.storage?.let { Usage(stringResource(R.string.dev_storage), it.free, it.total, Modifier.weight(1f)) }
                        s.ram?.let { Usage(stringResource(R.string.dev_memory), it.free * MB, it.total * MB, Modifier.weight(1f)) }
                    }
                }
                net?.detail?.let {
                    Spacer(Modifier.height(16.dp))
                    Text("${net.title} · $it", style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                // only what is out of the ordinary: DND, airplane, silent. "Screen on" and "Vibrate" are the normal state
                val silent = s.ringer == "silent"
                if (s.dnd || s.airplane == true || silent) {
                    Spacer(Modifier.height(12.dp))
                    FlowRow {
                        ModeChip(s.dnd, stringResource(R.string.dev_dnd), Icons.Rounded.DoNotDisturbOn)
                        ModeChip(s.airplane == true, stringResource(R.string.dev_airplane), Icons.Rounded.AirplanemodeActive)
                        ModeChip(silent, stringResource(R.string.dev_ringer_silent), Icons.Rounded.VolumeOff)
                    }
                }
            }
        }
    }
}

private const val MB = 1024L * 1024L

@Composable
private fun ModeChip(show: Boolean, text: String, icon: androidx.compose.ui.graphics.vector.ImageVector) {
    // the gap is part of the chip, so a hidden chip leaves no hole in the row
    AnimatedVisibility(show, enter = Motion.enterItem, exit = Motion.exitItem) { Chip(text, Modifier.padding(end = 8.dp, bottom = 4.dp), icon = icon) }
}

@Composable
private fun Usage(label: String, free: Long, total: Long, modifier: Modifier) {
    val c = Pill.colors
    val used = if (total > 0) 1f - free.toFloat() / total else 0f
    Column(modifier) {
        Text(label, style = MaterialTheme.typography.bodySmall, color = c.textDim)
        Spacer(Modifier.height(2.dp))
        Text(
            stringResource(R.string.dev_free, FileNames.humanSize(free)) + " " + stringResource(R.string.dev_of, FileNames.humanSize(total)),
            style = MaterialTheme.typography.bodySmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        Spacer(Modifier.height(8.dp))
        SpringProgress(used, height = 4.dp)
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
