package app.pill.phone.ui.screens

import android.text.format.DateUtils
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateIntAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.SdStorage
import androidx.compose.material.icons.rounded.Thermostat
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.R
import app.pill.phone.protocol.FileNames
import app.pill.phone.protocol.Msg
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import app.pill.phone.ui.components.Divider
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.SmallButton
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import app.pill.phone.ui.theme.PillShapes
import kotlin.math.roundToInt

/** Four equal rounded-square buttons with a label under each; the count of mirrored apps rides on the corner of Apps. */
@Composable
internal fun QuickActions(mirroredApps: Int?, a: HomeActions, modifier: Modifier) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        QuickAction(R.string.qa_send_file, Icons.Rounded.Upload, a.onSendFiles, Modifier.weight(1f))
        QuickAction(R.string.qa_send_clip, Icons.AutoMirrored.Rounded.Send, a.onSendClipboard, Modifier.weight(1f))
        QuickAction(R.string.qa_history, Icons.Rounded.History, a.onClipboard, Modifier.weight(1f))
        QuickAction(R.string.apps_card_title, Icons.Rounded.Notifications, a.onApps, Modifier.weight(1f), badge = mirroredApps)
    }
}

@Composable
private fun QuickAction(label: Int, icon: ImageVector, onClick: () -> Unit, modifier: Modifier, badge: Int? = null) {
    val c = Pill.colors
    val shape = RoundedCornerShape(PillShapes.Card)
    val count = badge?.takeIf { it > 0 }
    val state = count?.let { stringResource(R.string.apps_count, it) }
    Column(
        modifier.pressable(pressedScale = 0.94f, onClick = onClick).semantics { if (state != null) stateDescription = state },
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(62.dp)) {
            Box(Modifier.matchParentSize().clip(shape).background(c.surface).border(1.dp, c.border, shape), contentAlignment = Alignment.Center) {
                Icon(icon, null, Modifier.size(24.dp), tint = c.text)
            }
            // on the corner of the tile, never on the label
            CountBadge(count, Modifier.align(Alignment.TopEnd).offset(x = 8.dp, y = (-6).dp))
        }
        Spacer(Modifier.height(7.dp))
        Text(stringResource(label), style = MaterialTheme.typography.bodySmall.copy(fontSize = 11.5.sp), color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
    }
}

/** A white count pill cut out of the page colour; it springs in and out, and the digits slide when the number changes. 1000 and more read "999+". */
@Composable
private fun CountBadge(count: Int?, modifier: Modifier) {
    val c = Pill.colors
    AnimatedVisibility(count != null, modifier.clearAndSetSemantics { }, enter = Motion.enterItem, exit = Motion.exitItem) {
        Box(Modifier.background(c.background, CircleShape).padding(2.dp)) {
            Box(Modifier.defaultMinSize(minWidth = 20.dp, minHeight = 20.dp).background(c.accent, CircleShape).padding(horizontal = 6.dp), contentAlignment = Alignment.Center) {
                AnimatedContent(
                    count ?: 0,
                    transitionSpec = { (slideInVertically(Motion.soft()) { it } + fadeIn(Motion.calm())) togetherWith (slideOutVertically(Motion.calm()) { -it } + fadeOut(Motion.calm())) },
                    label = "count",
                ) { n -> Text(if (n > 999) "999+" else "$n", style = MaterialTheme.typography.labelSmall, color = c.onAccent, maxLines = 1) }
            }
        }
    }
}

/** "Recent": the newest clipboard text and the newest file in one grouped card. What has no source (mirrored notifications are not stored) is not shown. */
@Composable
internal fun Recent(ui: HomeUi, a: HomeActions, nowMs: Long?, modifier: Modifier) {
    val c = Pill.colors
    val clip = ui.lastClip
    val file = ui.transfers.firstOrNull()
    val now = remember(clip) { nowMs ?: System.currentTimeMillis() }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.recent_title), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, color = c.text)
            if (clip != null || file != null) {
                Box(Modifier.defaultMinSize(minHeight = 48.dp).clip(CircleShape).pressable(onClick = if (clip != null) a.onClipboard else a.onFiles).padding(horizontal = 8.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.recent_see_all), style = MaterialTheme.typography.labelMedium, color = c.textDim)
                }
            }
        }
        val shape = RoundedCornerShape(PillShapes.Row)
        Column(Modifier.fillMaxWidth().animateContentSize(Motion.soft()).clip(shape).background(c.surface).border(1.dp, c.border, shape)) {
            if (clip != null) {
                val sub = stringResource(R.string.recent_clip, clip.from ?: stringResource(R.string.hist_this_phone), DateUtils.getRelativeTimeSpanString(clip.at, now, DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE))
                RecentRow({ IconBadge(Icons.Rounded.ContentPaste, size = 42.dp) }, clip.text.trim().replace(Regex("\\s+"), " "), sub, a.onClipboard, Modifier.staggerIn(0))
            }
            if (clip != null && file != null) Divider()
            if (file != null) {
                RecentRow(
                    { TransferBadge(file, 42.dp) }, file.name, transferLine(file), a.onFiles, Modifier.staggerIn(0),
                    trailing = { TransferAction(file, a) },
                )
            }
            if (clip == null && file == null) {
                Text(stringResource(R.string.recent_empty), Modifier.padding(16.dp), style = MaterialTheme.typography.bodyMedium, color = c.textDim)
            }
        }
    }
}

@Composable
private fun RecentRow(badge: @Composable () -> Unit, title: String, sub: String, onClick: () -> Unit, modifier: Modifier, trailing: @Composable (() -> Unit)? = null) {
    val c = Pill.colors
    Row(modifier.fillMaxWidth().pressable(pressedScale = 0.99f, onClick = onClick).padding(horizontal = 16.dp, vertical = 13.dp), verticalAlignment = Alignment.CenterVertically) {
        badge()
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall.copy(fontSize = 16.sp), color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(sub, style = MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp), color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (trailing != null) {
            Spacer(Modifier.width(8.dp))
            trailing()
        }
    }
}

/** Receive / Open for a transfer, when there is something to do with it. */
@Composable
internal fun TransferAction(t: TransferUi, a: HomeActions) {
    when {
        t.phase == TransferPhase.Offered && !t.outgoing -> SmallButton(stringResource(R.string.btn_receive), { a.onAcceptTransfer(t.id) })
        t.phase == TransferPhase.Done && t.canOpen -> SmallButton(stringResource(R.string.btn_open), { a.onOpenTransfer(t.id) }, filled = false)
    }
}

/** The phone's own numbers in one slim rounded strip: battery, free storage, free memory, temperature (whatever the phone reports). */
@Composable
internal fun StatsStrip(s: Msg.Status, modifier: Modifier) {
    val c = Pill.colors
    val charging = s.charge == "charging" || (s.charge == null && s.charging)
    val pct by animateIntAsState(s.battery, Motion.soft(), label = "battery")
    val shape = RoundedCornerShape(PillShapes.Card)
    Row(
        modifier.fillMaxWidth().clip(shape).background(c.surface).border(1.dp, c.border, shape).padding(horizontal = 16.dp, vertical = 14.dp),
        horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
    ) {
        Stat(if (charging) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryStd, "$pct%", stringResource(R.string.stat_battery), if (charging) c.good else c.textDim)
        s.storage?.let { Stat(Icons.Rounded.SdStorage, FileNames.humanSize(it.free), stringResource(R.string.dev_storage) + ": " + stringResource(R.string.dev_free, FileNames.humanSize(it.free))) }
        s.ram?.let { Stat(Icons.Rounded.Memory, FileNames.humanSize(it.free * MB), stringResource(R.string.dev_memory) + ": " + stringResource(R.string.dev_free, FileNames.humanSize(it.free * MB))) }
        s.tempC?.let { Stat(Icons.Rounded.Thermostat, stringResource(R.string.dev_temp, it.roundToInt().toString()), stringResource(R.string.stat_temp)) }
    }
}

private const val MB = 1024L * 1024L

@Composable
private fun Stat(icon: ImageVector, text: String, description: String, tint: androidx.compose.ui.graphics.Color = Pill.colors.textDim) {
    Row(Modifier.semantics(mergeDescendants = true) { contentDescription = "$description $text" }, verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(16.dp), tint = tint)
        Spacer(Modifier.width(5.dp))
        Text(text, style = MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp), color = Pill.colors.textDim, maxLines = 1)
    }
}
