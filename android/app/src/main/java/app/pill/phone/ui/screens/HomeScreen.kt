package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.BatteryStd
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.SignalWifiOff
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.SignalCellularAlt
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import app.pill.phone.ui.components.BlobState
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.Chip
import app.pill.phone.ui.components.EmptyState
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.GlassIconButton
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.PillMark
import app.pill.phone.ui.components.SectionLabel
import app.pill.phone.ui.components.SmallButton
import app.pill.phone.ui.components.SpringProgress
import app.pill.phone.ui.components.StateBlob
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

class HomeActions(
    val onSettings: () -> Unit = {},
    val onScan: () -> Unit = {},
    val onEnterCode: () -> Unit = {},
    val onSendFiles: () -> Unit = {},
    val onApps: () -> Unit = {},
    val onReconnect: () -> Unit = {},
    val onOpenTransfer: (String) -> Unit = {},
    val onAcceptTransfer: (String) -> Unit = {},
    val perms: PermissionActions = PermissionActions(),
)

@Composable
fun HomeScreen(ui: HomeUi, actions: HomeActions) {
    val c = Pill.colors
    PillBackground {
        LazyColumn(
            Modifier.fillMaxWidth(),
            contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 8.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            item(key = "header") {
                Row(Modifier.statusBarsPadding().padding(top = 10.dp).staggerIn(0), verticalAlignment = Alignment.CenterVertically) {
                    PillMark(size = 40.dp)
                    Spacer(Modifier.width(10.dp))
                    Text(stringResource(R.string.app_name), Modifier.weight(1f), style = MaterialTheme.typography.titleLarge, color = c.text)
                    GlassIconButton(Icons.Rounded.Settings, stringResource(R.string.settings_title), actions.onSettings)
                }
            }
            item(key = "hero") { Hero(ui, actions, Modifier.staggerIn(1)) }
            if (!ui.perms.required || !ui.perms.storage) {
                item(key = "perms") {
                    AnimatedVisibility(true, enter = Motion.enterItem, exit = Motion.exitItem) {
                        PermissionsCard(ui.perms, actions.perms, Modifier.staggerIn(2))
                    }
                }
            }
            if (ui.link != LinkUi.Unpaired) {
                item(key = "quick") { QuickActions(ui, actions, Modifier.staggerIn(3)) }
            }
            if (ui.link != LinkUi.Unpaired) item(key = "files-label") { SectionLabel(stringResource(R.string.files_title), Modifier.staggerIn(4)) }
            if (ui.link == LinkUi.Unpaired) {
                // nothing to show before pairing
            } else if (ui.transfers.isEmpty()) {
                item(key = "files-empty") {
                    GlassCard(Modifier.staggerIn(5).animateContentSize(Motion.soft())) {
                        EmptyState(Icons.Rounded.FolderOpen, stringResource(R.string.files_empty_title), stringResource(R.string.files_empty_body))
                    }
                }
            } else {
                items(ui.transfers, key = { "t-" + it.id }) { t ->
                    TransferCard(t, actions, Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm()))
                }
            }
            item(key = "end") { NavBarSpacer() }
        }
    }
}

@Composable
private fun Hero(ui: HomeUi, actions: HomeActions, modifier: Modifier) {
    val c = Pill.colors
    val blob = when (ui.link) {
        LinkUi.Connected -> BlobState.Connected
        LinkUi.Connecting -> BlobState.Connecting
        LinkUi.Unpaired -> BlobState.Idle
        LinkUi.NoNetwork, LinkUi.NotFound -> BlobState.Offline
        LinkUi.Revoked -> BlobState.Error
    }
    GlassCard(modifier.animateContentSize(Motion.soft()), contentPadding = PaddingValues(horizontal = 20.dp, vertical = 24.dp), accentBorder = ui.link == LinkUi.Connected) {
        Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
            StateBlob(blob, size = 176.dp)
            Spacer(Modifier.height(8.dp))
            AnimatedContent(
                ui.link,
                transitionSpec = { (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.92f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.96f)) },
                label = "heroText",
            ) { link ->
                val pc = ui.pcName ?: stringResource(R.string.your_pc)
                val (title, body) = when (link) {
                    LinkUi.Connected -> stringResource(R.string.state_connected) to pc
                    LinkUi.Connecting -> stringResource(R.string.state_connecting) to stringResource(R.string.state_connecting_body, pc)
                    LinkUi.NoNetwork -> stringResource(R.string.state_nonetwork) to stringResource(R.string.state_nonetwork_body)
                    LinkUi.NotFound -> stringResource(R.string.state_notfound) to stringResource(R.string.state_notfound_body)
                    LinkUi.Revoked -> stringResource(R.string.state_revoked) to stringResource(R.string.state_revoked_body)
                    LinkUi.Unpaired -> stringResource(R.string.state_unpaired) to stringResource(R.string.state_unpaired_body)
                }
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(title, style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
                    Spacer(Modifier.height(6.dp))
                    Text(body, style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 6.dp))
                }
            }
            AnimatedVisibility(ui.link == LinkUi.Connected, enter = Motion.enterItem, exit = Motion.exitItem) {
                Row(Modifier.padding(top = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    ui.battery?.let { b ->
                        Chip("$b%", icon = if (ui.charging) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryStd)
                    }
                    val net = when (ui.network) {
                        "wifi" -> ui.ssid?.takeIf { it.isNotBlank() && it != "<unknown ssid>" } ?: stringResource(R.string.net_wifi)
                        "mobile" -> stringResource(R.string.net_mobile)
                        else -> stringResource(R.string.net_none)
                    }
                    Chip(net, icon = when (ui.network) { "wifi" -> Icons.Rounded.Wifi; "mobile" -> Icons.Rounded.SignalCellularAlt; else -> Icons.Rounded.SignalWifiOff })
                }
            }
            Spacer(Modifier.height(18.dp))
            HeroButtons(ui.link, actions)
        }
    }
}

@Composable
private fun HeroButtons(link: LinkUi, a: HomeActions) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        when (link) {
            LinkUi.Unpaired, LinkUi.Revoked -> {
                PillButton(stringResource(if (link == LinkUi.Revoked) R.string.btn_pair_again else R.string.btn_scan_qr), a.onScan, Modifier.fillMaxWidth(), icon = Icons.Rounded.QrCodeScanner)
                PillButton(stringResource(R.string.btn_enter_code), a.onEnterCode, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.Keyboard)
            }
            LinkUi.NotFound -> {
                PillButton(stringResource(R.string.btn_scan_again), a.onScan, Modifier.fillMaxWidth(), icon = Icons.Rounded.QrCodeScanner)
                PillButton(stringResource(R.string.btn_reconnect), a.onReconnect, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.Refresh)
            }
            LinkUi.NoNetwork -> PillButton(stringResource(R.string.btn_reconnect), a.onReconnect, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.Refresh)
            LinkUi.Connecting, LinkUi.Connected -> Unit
        }
    }
}

@Composable
private fun QuickActions(ui: HomeUi, a: HomeActions, modifier: Modifier) {
    val c = Pill.colors
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        GlassCard(Modifier.weight(1f), onClick = a.onSendFiles, contentPadding = PaddingValues(18.dp)) {
            IconBadge(Icons.Rounded.Upload)
            Spacer(Modifier.height(14.dp))
            Text(stringResource(R.string.btn_send_files), style = MaterialTheme.typography.titleMedium, color = c.text)
            Spacer(Modifier.height(2.dp))
            Text(stringResource(R.string.send_files_hint), style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        GlassCard(Modifier.weight(1f), onClick = a.onApps, contentPadding = PaddingValues(18.dp)) {
            IconBadge(Icons.Rounded.NotificationsActive)
            Spacer(Modifier.height(14.dp))
            Text(stringResource(R.string.apps_card_title), style = MaterialTheme.typography.titleMedium, color = c.text)
            Spacer(Modifier.height(2.dp))
            Text(
                ui.mirroredApps?.let { pluralStringResource(R.plurals.apps_on, it, it) } ?: stringResource(R.string.apps_card_hint),
                style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 2, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun TransferCard(t: TransferUi, a: HomeActions, modifier: Modifier) {
    val c = Pill.colors
    val tint = when (t.phase) {
        TransferPhase.Failed -> c.bad
        TransferPhase.Done -> c.good
        else -> c.accentSoft
    }
    val icon = when {
        t.phase == TransferPhase.Failed -> Icons.Rounded.ErrorOutline
        t.outgoing -> Icons.Rounded.ArrowUpward
        else -> Icons.Rounded.ArrowDownward
    }
    GlassCard(modifier.animateContentSize(Motion.soft()), contentPadding = PaddingValues(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBadge(icon, tint = tint)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(t.name, style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(2.dp))
                val status = when (t.phase) {
                    TransferPhase.Offered -> stringResource(R.string.tr_offered)
                    TransferPhase.Waiting -> stringResource(R.string.tr_waiting)
                    TransferPhase.Active -> stringResource(if (t.outgoing) R.string.tr_sending else R.string.tr_receiving)
                    TransferPhase.Done -> stringResource(if (t.outgoing) R.string.tr_sent else R.string.tr_received)
                    TransferPhase.Failed -> t.detail ?: stringResource(R.string.tr_failed)
                }
                Text(
                    "$status  ·  ${t.sizeText}", style = MaterialTheme.typography.bodySmall,
                    color = if (t.phase == TransferPhase.Failed) c.bad else c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            when {
                t.phase == TransferPhase.Offered && !t.outgoing -> SmallButton(stringResource(R.string.btn_receive), { a.onAcceptTransfer(t.id) })
                t.phase == TransferPhase.Done && t.canOpen -> SmallButton(stringResource(R.string.btn_open), { a.onOpenTransfer(t.id) }, filled = false)
            }
        }
        AnimatedVisibility(t.phase == TransferPhase.Active || t.phase == TransferPhase.Waiting, enter = Motion.enterItem, exit = Motion.exitItem) {
            Box(Modifier.padding(top = 14.dp)) { SpringProgress(if (t.phase == TransferPhase.Waiting) 0.04f else t.progress) }
        }
    }
}
