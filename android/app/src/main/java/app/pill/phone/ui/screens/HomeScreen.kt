package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.net.RelayState
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import app.pill.phone.ui.components.BlobState
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.GlassIconButton
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.PillMark
import app.pill.phone.ui.components.RouteBadge
import app.pill.phone.ui.components.SectionLabel
import app.pill.phone.ui.components.SmallButton
import app.pill.phone.ui.components.SpringProgress
import app.pill.phone.ui.components.StateBlob
import app.pill.phone.ui.components.pressable
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
    val onTogglePhone: () -> Unit = {},
    val onOpenTransfer: (String) -> Unit = {},
    val onAcceptTransfer: (String) -> Unit = {},
    val perms: PermissionActions = PermissionActions(),
)

/**
 * Home is one calm column on a 16 dp rhythm: the hero (the state), one slim "This phone" row, one row of actions,
 * and the files only when there are some. Every other state (not paired, connecting, offline) keeps the same shape.
 */
@Composable
fun HomeScreen(ui: HomeUi, actions: HomeActions) {
    PillBackground {
        LazyColumn(
            Modifier.fillMaxWidth(),
            contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 8.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            item(key = "header") {
                Row(Modifier.statusBarsPadding().padding(top = 8.dp).staggerIn(0), verticalAlignment = Alignment.CenterVertically) {
                    PillMark(size = 36.dp)
                    Spacer(Modifier.width(10.dp))
                    Text(stringResource(R.string.app_name), Modifier.weight(1f), style = MaterialTheme.typography.titleLarge, color = Pill.colors.text)
                    GlassIconButton(Icons.Rounded.Settings, stringResource(R.string.settings_title), actions.onSettings)
                }
            }
            item(key = "hero") { Hero(ui, actions, Modifier.staggerIn(1)) }
            if (ui.link == LinkUi.Connected && ui.status != null) {
                item(key = "device") { DeviceCard(ui.status, ui.phoneExpanded, actions.onTogglePhone, Modifier.staggerIn(2)) }
            }
            if (ui.link != LinkUi.Unpaired) {
                item(key = "quick") { QuickActions(ui, actions, Modifier.staggerIn(3)) }
            }
            if (!ui.perms.required || !ui.perms.storage) {
                item(key = "perms") {
                    AnimatedVisibility(true, enter = Motion.enterItem, exit = Motion.exitItem) {
                        PermissionsCard(ui.perms, actions.perms, Modifier.staggerIn(4))
                    }
                }
            }
            // no files, no section: the label and the cards come and go together on a spring
            if (ui.link != LinkUi.Unpaired && ui.transfers.isNotEmpty()) {
                item(key = "files-label") {
                    SectionLabel(stringResource(R.string.files_title), Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm()))
                }
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
    val relayWaiting = ui.relay == RelayState.WaitingForPc
    val swap = { _: Any -> (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.94f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.97f)) }
    // a short line sits next to the drop; the longer explanation (offline, unpaired...) goes under it, full width
    val pc = ui.pcName ?: stringResource(R.string.your_pc)
    val explain = when (ui.link) {
        LinkUi.NoNetwork -> R.string.state_nonetwork_body
        LinkUi.NotFound -> if (relayWaiting) R.string.state_notfound_relay_body else R.string.state_notfound_body
        LinkUi.Revoked -> R.string.state_revoked_body
        LinkUi.Unpaired -> R.string.state_unpaired_body
        else -> null
    }
    GlassCard(
        modifier.animateContentSize(Motion.soft()),
        contentPadding = PaddingValues(start = 8.dp, end = 20.dp, top = 16.dp, bottom = 16.dp),
        accentBorder = ui.link == LinkUi.Connected,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            StateBlob(blob, size = 120.dp)
            Spacer(Modifier.width(4.dp))
            Column(Modifier.weight(1f)) {
                AnimatedContent(ui.link, transitionSpec = { swap(0) }, label = "heroText") { link ->
                    val title = when (link) {
                        LinkUi.Connected -> stringResource(R.string.state_connected)
                        LinkUi.Connecting -> stringResource(R.string.state_connecting)
                        LinkUi.NoNetwork -> stringResource(R.string.state_nonetwork)
                        LinkUi.NotFound -> stringResource(if (relayWaiting) R.string.state_notfound_pc else R.string.state_notfound)
                        LinkUi.Revoked -> stringResource(R.string.state_revoked)
                        LinkUi.Unpaired -> stringResource(R.string.state_unpaired)
                    }
                    val sub = when (link) {
                        LinkUi.Connected -> pc
                        LinkUi.Connecting -> stringResource(R.string.state_connecting_body, pc)
                        else -> null
                    }
                    Column {
                        Text(title, style = MaterialTheme.typography.titleLarge, color = c.text)
                        if (sub != null) {
                            Spacer(Modifier.height(2.dp))
                            Text(sub, style = MaterialTheme.typography.bodyMedium, color = c.textDim, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
                AnimatedVisibility(ui.link == LinkUi.Connected || ui.link == LinkUi.NotFound || ui.link == LinkUi.NoNetwork, enter = Motion.enterItem, exit = Motion.exitItem) {
                    RouteBadge(ui.route, Modifier.padding(top = 10.dp), waitingForPc = relayWaiting, compact = true)
                }
            }
        }
        AnimatedVisibility(explain != null, enter = Motion.enterItem, exit = Motion.exitItem) {
            // keep the last text while it fades out
            val last = remember { intArrayOf(0) }
            if (explain != null) last[0] = explain
            if (last[0] != 0) Text(
                stringResource(last[0]), Modifier.padding(start = 12.dp, top = 4.dp, bottom = 4.dp),
                style = MaterialTheme.typography.bodyMedium, color = c.textDim,
            )
        }
        HeroButtons(ui.link, actions)
    }
}

/** The way out of a state that is not "connected": one clear primary, the second action is quiet. */
@Composable
private fun HeroButtons(link: LinkUi, a: HomeActions) {
    val shown = link != LinkUi.Connecting && link != LinkUi.Connected
    AnimatedVisibility(shown, enter = Motion.enterItem, exit = Motion.exitItem) {
        Column(Modifier.fillMaxWidth().padding(start = 12.dp, top = 8.dp, bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            when (link) {
                LinkUi.Unpaired, LinkUi.Revoked -> {
                    PillButton(stringResource(if (link == LinkUi.Revoked) R.string.btn_pair_again else R.string.btn_scan_qr), a.onScan, Modifier.fillMaxWidth(), icon = Icons.Rounded.QrCodeScanner)
                    PillButton(stringResource(R.string.btn_enter_code), a.onEnterCode, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet, icon = Icons.Rounded.Keyboard)
                }
                LinkUi.NotFound -> {
                    PillButton(stringResource(R.string.btn_reconnect), a.onReconnect, Modifier.fillMaxWidth(), icon = Icons.Rounded.Refresh)
                    PillButton(stringResource(R.string.btn_scan_again), a.onScan, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet, icon = Icons.Rounded.QrCodeScanner)
                }
                LinkUi.NoNetwork -> PillButton(stringResource(R.string.btn_reconnect), a.onReconnect, Modifier.fillMaxWidth(), icon = Icons.Rounded.Refresh)
                LinkUi.Connecting, LinkUi.Connected -> Unit
            }
        }
    }
}

/** Send files and Apps: two equal glass pills (icon and a short label); the number of mirrored apps is a small badge. */
@Composable
private fun QuickActions(ui: HomeUi, a: HomeActions, modifier: Modifier) {
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        ActionPill(stringResource(R.string.btn_send_files), Icons.Rounded.Upload, a.onSendFiles, Modifier.weight(1f))
        ActionPill(stringResource(R.string.apps_card_title), Icons.Rounded.NotificationsActive, a.onApps, Modifier.weight(1f), badge = ui.mirroredApps)
    }
}

@Composable
private fun ActionPill(text: String, icon: ImageVector, onClick: () -> Unit, modifier: Modifier, badge: Int? = null) {
    val c = Pill.colors
    val tint = if (c.dark) c.accentSoft else c.accent
    // the pill is clipped to its shape, the badge sits on its rim (a notification-style count, never steals label width)
    Box(modifier.pressable(onClick = onClick)) {
        Row(
            Modifier.fillMaxWidth().height(52.dp).clip(CircleShape).background(c.surfaceHigh).border(1.dp, c.border, CircleShape).padding(horizontal = 12.dp),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, Modifier.size(20.dp), tint = tint)
            Spacer(Modifier.width(8.dp))
            Text(text, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.labelLarge, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        AnimatedVisibility(badge != null, Modifier.align(Alignment.TopEnd).offset(x = (-12).dp, y = (-6).dp), enter = Motion.enterItem, exit = Motion.exitItem) {
            Box(
                Modifier.defaultMinSize(minWidth = 22.dp, minHeight = 22.dp).clip(CircleShape).background(c.accent).padding(horizontal = 6.dp),
                contentAlignment = Alignment.Center,
            ) { Text("${badge ?: 0}", style = MaterialTheme.typography.labelSmall, color = c.onAccent, maxLines = 1) }
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
    GlassCard(modifier.animateContentSize(Motion.soft()), contentPadding = PaddingValues(horizontal = 14.dp, vertical = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBadge(icon, tint = tint, size = 38.dp)
            Spacer(Modifier.width(12.dp))
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
            Box(Modifier.padding(top = 12.dp)) { SpringProgress(if (t.phase == TransferPhase.Waiting) 0.04f else t.progress, height = 6.dp) }
        }
    }
}
