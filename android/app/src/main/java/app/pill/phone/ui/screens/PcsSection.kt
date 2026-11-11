package app.pill.phone.ui.screens

import android.text.format.DateUtils
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowForward
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.SaveAlt
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.pill.phone.R
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PcSettingsUi
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.Divider
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.PillSwitch
import app.pill.phone.ui.components.SettingRow
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

/** The "PCs" section: one card per paired PC with its own switches, and the way to add another. */
class PcsActions(
    val onRename: (String) -> Unit = {},
    val onMirror: (String, Boolean) -> Unit = { _, _ -> },
    val onAutoReceive: (String, Boolean) -> Unit = { _, _ -> },
    val onOwnFilter: (String, Boolean) -> Unit = { _, _ -> },
    val onChooseApps: (String) -> Unit = {},
    val onUnpair: (String) -> Unit = {},
)

@Composable
fun PcCard(pc: PcSettingsUi, a: PcsActions, modifier: Modifier = Modifier) {
    val c = Pill.colors
    GlassCard(modifier.animateContentSize(Motion.soft()), accentBorder = pc.link == LinkUi.Connected) {
        SettingRow(pc.name, subtitle = statusLine(pc), icon = Icons.Rounded.Computer) {
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(c.surfaceHigh).border(1.dp, c.border, CircleShape)
                    .pressable(onClick = { a.onRename(pc.pcId) }),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Rounded.Edit, stringResource(R.string.pcs_rename), Modifier.size(18.dp), tint = c.textDim) }
        }
        Divider(Modifier.padding(vertical = 8.dp))
        SettingRow(stringResource(R.string.pcs_mirror), subtitle = stringResource(R.string.pcs_mirror_body), icon = Icons.Rounded.NotificationsActive) {
            PillSwitch(pc.mirror, { a.onMirror(pc.pcId, it) })
        }
        Divider(Modifier.padding(vertical = 8.dp))
        SettingRow(stringResource(R.string.pcs_auto_receive), subtitle = stringResource(R.string.pcs_auto_receive_body), icon = Icons.Rounded.SaveAlt) {
            PillSwitch(pc.autoReceive, { a.onAutoReceive(pc.pcId, it) })
        }
        Divider(Modifier.padding(vertical = 8.dp))
        SettingRow(
            stringResource(R.string.pcs_filter),
            subtitle = stringResource(if (pc.ownFilter) R.string.pcs_filter_custom else R.string.pcs_filter_same),
            icon = Icons.Rounded.Tune,
        ) { PillSwitch(pc.ownFilter, { a.onOwnFilter(pc.pcId, it) }) }
        AnimatedVisibility(pc.ownFilter, enter = Motion.enterItem, exit = Motion.exitItem) {
            Column {
                Divider(Modifier.padding(vertical = 8.dp))
                SettingRow(
                    stringResource(R.string.pcs_filter_edit), subtitle = pc.filterCount?.let { stringResource(R.string.apps_count, it) },
                    onClick = { a.onChooseApps(pc.pcId) },
                ) { Icon(Icons.AutoMirrored.Rounded.ArrowForward, null, tint = c.textDim) }
            }
        }
        Spacer(Modifier.height(14.dp))
        PillButton(stringResource(R.string.btn_unpair), { a.onUnpair(pc.pcId) }, Modifier.fillMaxWidth(), kind = ButtonKind.Danger, icon = Icons.Rounded.LinkOff)
    }
}

/** "Connected · Home network", "Connecting…" or "Not reachable · Last seen 5 min. ago". */
@Composable
private fun statusLine(pc: PcSettingsUi): String = when (pc.link) {
    LinkUi.Connected -> stringResource(R.string.pcs_connected_via, routeWord(pc.route, true))
    LinkUi.Connecting -> stringResource(R.string.pcs_connecting)
    else -> {
        val seen = if (pc.lastSeen > 0) {
            stringResource(R.string.pcs_last_seen, DateUtils.getRelativeTimeSpanString(pc.lastSeen, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString())
        } else stringResource(R.string.pcs_never_seen)
        stringResource(R.string.pcs_offline) + "  ·  " + seen
    }
}

@Composable
fun AddPcButton(onClick: () -> Unit, modifier: Modifier = Modifier, kind: ButtonKind = ButtonKind.Glass) {
    PillButton(stringResource(R.string.pcs_add), onClick, modifier.fillMaxWidth(), kind = kind, icon = Icons.Rounded.Add)
}

/** Rename: the user's own name for a PC; empty goes back to the PC's name. */
@Composable
fun RenamePcDialog(current: String, pcName: String, onSave: (String) -> Unit, onDismiss: () -> Unit) {
    val c = Pill.colors
    var text by remember { mutableStateOf(current) }
    val visible = remember { MutableTransitionState(false).apply { targetState = true } }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        AnimatedVisibility(visible, enter = Motion.enterRise, exit = Motion.exitSink) {
            val shape = RoundedCornerShape(28.dp)
            Column(
                Modifier.padding(24.dp).fillMaxWidth().clip(shape).background(c.surfaceHigh).border(1.dp, c.border, shape).padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(stringResource(R.string.pcs_rename_title), style = MaterialTheme.typography.titleLarge, color = c.text, textAlign = TextAlign.Center)
                Spacer(Modifier.height(6.dp))
                Text(stringResource(R.string.pcs_rename_hint), style = MaterialTheme.typography.bodyMedium, color = c.textDim, textAlign = TextAlign.Center)
                Spacer(Modifier.height(18.dp))
                PillTextField(text, { text = it.take(40) }, stringResource(R.string.field_pc_name), pcName, KeyboardType.Text, ImeAction.Done, onDone = { onSave(text) })
                Spacer(Modifier.height(22.dp))
                PillButton(stringResource(R.string.btn_save), { onSave(text) }, Modifier.fillMaxWidth())
                Spacer(Modifier.height(8.dp))
                PillButton(stringResource(R.string.btn_cancel), onDismiss, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet)
            }
        }
    }
}
