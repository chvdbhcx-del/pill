package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.rounded.Public
import androidx.compose.runtime.getValue
import androidx.compose.ui.draw.alpha
import app.pill.phone.net.RelayState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowForward
import androidx.compose.material.icons.automirrored.rounded.InsertDriveFile
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.LinkOff
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.SaveAlt
import androidx.compose.material.icons.rounded.Smartphone
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.pill.phone.R
import app.pill.phone.ui.SettingsUi
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.Divider
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.PillMark
import app.pill.phone.ui.components.PillSwitch
import app.pill.phone.ui.components.SectionLabel
import app.pill.phone.ui.components.SettingRow
import app.pill.phone.ui.components.TopBar
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

class SettingsActions(
    val onBack: () -> Unit = {},
    val onApps: () -> Unit = {},
    val onAutoReceive: (Boolean) -> Unit = {},
    val onOngoing: (Boolean) -> Unit = {},
    val onRelay: (Boolean) -> Unit = {},
    val onUnpair: () -> Unit = {},
    val onPair: () -> Unit = {},
    val perms: PermissionActions = PermissionActions(),
)

@Composable
fun SettingsScreen(ui: SettingsUi, actions: SettingsActions, askUnpair: Boolean = false, onAskUnpair: (Boolean) -> Unit = {}) {
    val c = Pill.colors
    PillBackground {
        Column(Modifier.fillMaxSize()) {
            TopBar(stringResource(R.string.settings_title), actions.onBack)
            LazyColumn(
                Modifier.weight(1f),
                contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                item(key = "pc") {
                    GlassCard(Modifier.staggerIn(0)) {
                        SettingRow(
                            ui.pcName ?: stringResource(R.string.state_unpaired),
                            subtitle = ui.address ?: stringResource(R.string.settings_not_paired_body),
                            icon = Icons.Rounded.Computer,
                        )
                        Spacer(Modifier.height(14.dp))
                        if (ui.pcName != null) PillButton(stringResource(R.string.btn_unpair), { onAskUnpair(true) }, Modifier.fillMaxWidth(), kind = ButtonKind.Danger, icon = Icons.Rounded.LinkOff)
                        else PillButton(stringResource(R.string.btn_scan_qr), actions.onPair, Modifier.fillMaxWidth(), icon = Icons.Rounded.QrCodeScanner)
                    }
                }
                if (ui.pcName != null) {
                    item(key = "relay-label") { SectionLabel(stringResource(R.string.settings_relay_section)) }
                    item(key = "relay") { RelayCard(ui, actions.onRelay, Modifier.staggerIn(1)) }
                }
                if (ui.model.isNotBlank()) {
                    item(key = "phone-label") { SectionLabel(stringResource(R.string.dev_section)) }
                    item(key = "phone") {
                        GlassCard(Modifier.staggerIn(1)) {
                            SettingRow(ui.model, subtitle = stringResource(R.string.dev_model_line, ui.androidVersion), icon = Icons.Rounded.Smartphone)
                            Divider(Modifier.padding(vertical = 8.dp))
                            SettingRow(stringResource(R.string.dev_shared_title), subtitle = stringResource(R.string.dev_shared_body), icon = Icons.Rounded.Visibility)
                        }
                    }
                }
                item(key = "notif-label") { SectionLabel(stringResource(R.string.settings_notifications)) }
                item(key = "notif") {
                    GlassCard(Modifier.staggerIn(1)) {
                        SettingRow(stringResource(R.string.apps_title), subtitle = stringResource(R.string.settings_apps_body), icon = Icons.Rounded.Tune, onClick = actions.onApps) {
                            Icon(Icons.AutoMirrored.Rounded.ArrowForward, null, tint = c.textDim)
                        }
                        Divider(Modifier.padding(vertical = 8.dp))
                        SettingRow(stringResource(R.string.settings_ongoing), subtitle = stringResource(R.string.settings_ongoing_body), icon = Icons.Rounded.NotificationsActive) {
                            PillSwitch(ui.includeOngoing, actions.onOngoing)
                        }
                    }
                }
                item(key = "files-label") { SectionLabel(stringResource(R.string.files_title)) }
                item(key = "files") {
                    GlassCard(Modifier.staggerIn(2)) {
                        SettingRow(stringResource(R.string.settings_auto_receive), subtitle = stringResource(R.string.settings_auto_receive_body), icon = Icons.Rounded.SaveAlt) {
                            PillSwitch(ui.autoReceive, actions.onAutoReceive)
                        }
                        Divider(Modifier.padding(vertical = 8.dp))
                        SettingRow(stringResource(R.string.settings_saved_to), subtitle = "Downloads/Pill", icon = Icons.AutoMirrored.Rounded.InsertDriveFile)
                    }
                }
                item(key = "perms-label") { SectionLabel(stringResource(R.string.settings_permissions)) }
                item(key = "perms") { PermissionsCard(ui.perms, actions.perms, Modifier.staggerIn(3)) }
                item(key = "about-label") { SectionLabel(stringResource(R.string.settings_about)) }
                item(key = "about") {
                    GlassCard(Modifier.staggerIn(4)) {
                        VersionRow(ui.versionName)
                        Divider(Modifier.padding(vertical = 8.dp))
                        SettingRow(stringResource(R.string.settings_privacy), subtitle = stringResource(R.string.settings_privacy_body), icon = Icons.Rounded.Lock)
                        Divider(Modifier.padding(vertical = 8.dp))
                        Text(
                            stringResource(R.string.settings_proprietary), Modifier.padding(top = 4.dp).fillMaxWidth(),
                            style = MaterialTheme.typography.bodySmall, color = c.textDim, textAlign = TextAlign.Center,
                        )
                    }
                }
                item(key = "end") { NavBarSpacer() }
            }
        }
        if (askUnpair) UnpairDialog(ui.pcName, onConfirm = { onAskUnpair(false); actions.onUnpair() }, onDismiss = { onAskUnpair(false) })
    }
}

/** The internet link: the switch, the privacy note of the protocol, and where the link stands right now. */
@Composable
private fun RelayCard(ui: SettingsUi, onRelay: (Boolean) -> Unit, modifier: Modifier) {
    val c = Pill.colors
    val available = ui.relay != RelayState.Unavailable
    GlassCard(modifier.animateContentSize(Motion.soft())) {
        SettingRow(
            stringResource(R.string.settings_relay),
            subtitle = stringResource(R.string.settings_relay_short),
            icon = Icons.Rounded.Public,
        ) {
            PillSwitch(ui.relayEnabled && available, { if (available) onRelay(it) }, Modifier.alpha(if (available) 1f else 0.4f))
        }
        Divider(Modifier.padding(vertical = 8.dp))
        val (text, tint) = when (ui.relay) {
            RelayState.Unavailable -> R.string.relay_unavailable to c.textDim
            RelayState.Disabled -> R.string.relay_disabled to c.textDim
            RelayState.Standby -> R.string.relay_standby to c.good
            RelayState.Connecting -> R.string.relay_connecting to c.accentSoft
            RelayState.WaitingForPc -> R.string.relay_waiting to c.accentSoft
            RelayState.Active -> R.string.relay_active to c.accentSoft
            RelayState.Refused -> R.string.relay_refused to c.bad
        }
        val tintNow by animateColorAsState(tint, Motion.calm(), label = "relayTint")
        AnimatedContent(
            text,
            transitionSpec = { (fadeIn(Motion.calm()) + slideInVertically(Motion.soft()) { it / 3 }) togetherWith (fadeOut(Motion.calm()) + slideOutVertically(Motion.calm()) { -it / 3 }) },
            label = "relayText",
        ) { res ->
            Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(10.dp).clip(CircleShape).background(tintNow))
                Spacer(Modifier.width(12.dp))
                Text(stringResource(res), Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = c.text)
            }
        }
        AnimatedVisibility(available, enter = Motion.enterItem, exit = Motion.exitItem) {
            Text(
                stringResource(R.string.relay_file_cap, ui.relayMaxMb), Modifier.padding(top = 2.dp),
                style = MaterialTheme.typography.bodySmall, color = c.textDim,
            )
        }
        Text(
            stringResource(R.string.settings_relay_body), Modifier.padding(top = 10.dp),
            style = MaterialTheme.typography.bodySmall, color = c.textDim,
        )
    }
}

@Composable
private fun VersionRow(version: String) {
    SettingRow(stringResource(R.string.app_name), subtitle = stringResource(R.string.settings_version, version), icon = Icons.Rounded.Info) {
        PillMark(size = 36.dp)
    }
}

@Composable
fun UnpairDialog(pcName: String?, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    val c = Pill.colors
    val visible = remember { MutableTransitionState(false).apply { targetState = true } }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        AnimatedVisibility(visible, enter = Motion.enterRise, exit = Motion.exitSink) {
            val shape = RoundedCornerShape(28.dp)
            Column(
                Modifier.padding(24.dp).fillMaxWidth().clip(shape).background(c.surfaceHigh).border(1.dp, c.border, shape).padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Box(Modifier.clip(RoundedCornerShape(50)).background(c.bad.copy(alpha = 0.14f)).padding(16.dp)) {
                    Icon(Icons.Rounded.LinkOff, null, tint = c.bad)
                }
                Spacer(Modifier.height(16.dp))
                Text(stringResource(R.string.unpair_title, pcName ?: stringResource(R.string.your_pc)), style = MaterialTheme.typography.titleLarge, color = c.text, textAlign = TextAlign.Center)
                Spacer(Modifier.height(8.dp))
                Text(stringResource(R.string.unpair_body), style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
                Spacer(Modifier.height(22.dp))
                PillButton(stringResource(R.string.btn_unpair), onConfirm, Modifier.fillMaxWidth(), kind = ButtonKind.Danger)
                Spacer(Modifier.height(8.dp))
                PillButton(stringResource(R.string.btn_cancel), onDismiss, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet)
            }
        }
    }
}
