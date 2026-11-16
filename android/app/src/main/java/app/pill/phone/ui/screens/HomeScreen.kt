package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DesktopWindows
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.net.RelayState
import app.pill.phone.net.Route
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PcUi
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.GlassIconButton
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.TopBar
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.components.rememberPulse
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

class HomeActions(
    val onSettings: () -> Unit = {},
    val onScan: () -> Unit = {},
    val onEnterCode: () -> Unit = {},
    val onSendFiles: () -> Unit = {},
    val onSendClipboard: () -> Unit = {},
    val onApps: () -> Unit = {},
    val onFiles: () -> Unit = {},
    val onClipboard: () -> Unit = {},
    val onReconnect: () -> Unit = {},
    val onSelectPc: (String) -> Unit = {},
    val onOpenTransfer: (String) -> Unit = {},
    val onAcceptTransfer: (String) -> Unit = {},
    val update: UpdateActions = UpdateActions(),
    val perms: PermissionActions = PermissionActions(),
)

/**
 * Home: the title and the gear, the hero (which PC and how the link is), four quick actions, the latest items and a slim strip
 * of the phone's own numbers. Every state (not paired, connecting, offline) keeps the same shape; unpaired shows only the hero.
 */
@Composable
fun HomeScreen(ui: HomeUi, actions: HomeActions, nowMs: Long? = null) {
    val paired = ui.link != LinkUi.Unpaired
    PillBackground {
        Column(Modifier.fillMaxSize()) {
            TopBar(stringResource(R.string.app_name), null) { GlassIconButton(Icons.Rounded.Settings, stringResource(R.string.settings_title), actions.onSettings) }
            LazyColumn(
                Modifier.weight(1f),
                contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 16.dp),
                verticalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                item(key = "hero") { Hero(ui, actions, Modifier.staggerIn(0)) }
                // a newer build is out (and this one was not dismissed): a small card that comes and goes on a spring
                if (ui.update.showCard) {
                    item(key = "update") {
                        UpdateCard(ui.update, actions.update, Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm()).staggerIn(1))
                    }
                }
                if (paired) item(key = "quick") { QuickActions(ui.mirroredApps, actions, Modifier.staggerIn(1)) }
                if (!ui.perms.required || !ui.perms.storage) {
                    item(key = "perms") {
                        AnimatedVisibility(true, enter = Motion.enterItem, exit = Motion.exitItem) { PermissionsCard(ui.perms, actions.perms, Modifier.staggerIn(2)) }
                    }
                }
                if (paired) item(key = "recent") { Recent(ui, actions, nowMs, Modifier.staggerIn(2)) }
                if (paired && ui.status != null) item(key = "stats") { StatsStrip(ui.status, Modifier.staggerIn(3)) }
            }
        }
    }
}

@Composable
private fun Hero(ui: HomeUi, actions: HomeActions, modifier: Modifier) {
    val c = Pill.colors
    val relayWaiting = ui.relay == RelayState.WaitingForPc
    // the longer explanation (offline, unpaired...) goes under the row, full width
    val explain = when (ui.link) {
        LinkUi.NoNetwork -> R.string.state_nonetwork_body
        LinkUi.NotFound -> if (relayWaiting) R.string.state_notfound_relay_body else R.string.state_notfound_body
        LinkUi.Revoked -> R.string.state_revoked_body
        LinkUi.Unpaired -> R.string.state_unpaired_body
        else -> null
    }
    GlassCard(modifier.animateContentSize(Motion.soft()), contentPadding = PaddingValues(18.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            PcBadge(ui.link)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Crossfade(ui.pcName ?: stringResource(R.string.hero_no_pc), animationSpec = Motion.calm(), label = "heroName") {
                    Text(it, style = MaterialTheme.typography.headlineSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Spacer(Modifier.height(3.dp))
                HeroLine(ui)
                Spacer(Modifier.height(9.dp))
                StatusPill(ui.link, relayWaiting)
            }
        }
        // with two or more PCs: small pills choose which one the card describes
        AnimatedVisibility(ui.pcs.size >= 2, enter = Motion.enterItem, exit = Motion.exitItem) {
            PcSwitcher(ui.pcs, ui.selectedPcId, actions.onSelectPc, Modifier.padding(top = 14.dp))
        }
        AnimatedVisibility(explain != null, enter = Motion.enterItem, exit = Motion.exitItem) {
            // keep the last text while it fades out
            val last = remember { intArrayOf(0) }
            if (explain != null) last[0] = explain
            if (last[0] != 0) Text(stringResource(last[0]), Modifier.padding(top = 14.dp), style = MaterialTheme.typography.bodyMedium, color = c.textDim)
        }
        HeroButtons(ui.link, actions)
    }
}

/** The muted line under the name: how the PC is reached ("Home network" / "Internet") while connected, else its address. Nothing when neither is known. */
@Composable
private fun HeroLine(ui: HomeUi) {
    val c = Pill.colors
    val way = if (ui.link == LinkUi.Connected) ui.route else null
    Crossfade(way to ui.address, animationSpec = Motion.calm(), label = "heroLine") { (route, address) ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (route != null) {
                Icon(if (route == Route.Lan) Icons.Rounded.Home else Icons.Rounded.Public, null, Modifier.size(15.dp), tint = c.textDim)
                Spacer(Modifier.width(5.dp))
            }
            Text(
                if (route != null) stringResource(if (route == Route.Lan) R.string.route_home else R.string.route_internet) else address.orEmpty(),
                style = MaterialTheme.typography.bodyMedium, color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** The round PC mark with a thin outer ring: solid accent while connected, grey otherwise; the ring pulses while connecting. */
@Composable
private fun PcBadge(link: LinkUi) {
    val c = Pill.colors
    val on = link == LinkUi.Connected
    val fill by animateColorAsState(if (on) c.accent else c.surfaceHigh, Motion.calm(), label = "badgeFill")
    val ink by animateColorAsState(if (on) c.onAccent else c.textDim, Motion.calm(), label = "badgeInk")
    val pulse = rememberPulse(link == LinkUi.Connecting)
    Box(Modifier.size(78.dp), contentAlignment = Alignment.Center) {
        Box(
            Modifier.matchParentSize().graphicsLayer { val k = 1f + 0.12f * pulse.value; scaleX = k; scaleY = k; alpha = 1f - 0.7f * pulse.value }
                .border(1.dp, c.accent.copy(alpha = 0.2f), CircleShape),
        )
        Box(Modifier.size(64.dp).clip(CircleShape).background(fill), contentAlignment = Alignment.Center) { Icon(Icons.Rounded.DesktopWindows, null, Modifier.size(28.dp), tint = ink) }
    }
}

/** "Connected" (green), "Connecting" (a dot that pulses), "Offline" (amber), "Not paired" (grey): one pill, its colour and text change on a spring. */
@Composable
private fun StatusPill(link: LinkUi, relayWaiting: Boolean) {
    val c = Pill.colors
    val tint by animateColorAsState(
        when (link) {
            LinkUi.Connected -> c.good
            LinkUi.Connecting -> c.text
            LinkUi.NoNetwork, LinkUi.NotFound -> c.warn
            LinkUi.Revoked -> c.bad
            LinkUi.Unpaired -> c.textDim
        },
        Motion.calm(), label = "pillTint",
    )
    val pulse = rememberPulse(link == LinkUi.Connecting, 1200)
    Row(Modifier.clip(CircleShape).background(tint.copy(alpha = 0.13f)).padding(horizontal = 10.dp, vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(14.dp), contentAlignment = Alignment.Center) {
            Box(Modifier.matchParentSize().graphicsLayer { val k = 1f + 0.4f * pulse.value; scaleX = k; scaleY = k; alpha = 1f - pulse.value }.background(tint.copy(alpha = 0.25f), CircleShape))
            Box(Modifier.size(8.dp).background(tint, CircleShape))
        }
        Spacer(Modifier.width(6.dp))
        AnimatedContent(
            link,
            transitionSpec = { (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.9f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.95f)) },
            label = "pillText",
        ) { l ->
            Text(
                stringResource(
                    when (l) {
                        LinkUi.Connected -> R.string.state_connected
                        LinkUi.Connecting -> R.string.state_connecting
                        LinkUi.NoNetwork -> R.string.state_nonetwork
                        LinkUi.NotFound -> if (relayWaiting) R.string.route_offline else R.string.pill_notfound
                        LinkUi.Revoked -> R.string.pill_revoked
                        LinkUi.Unpaired -> R.string.state_unpaired
                    },
                ),
                style = MaterialTheme.typography.labelMedium, color = tint, maxLines = 1,
            )
        }
    }
}

/** Small pills, one per PC: a status dot and the name. The selected one is tinted; the selection and the dot colour move on springs. */
@Composable
private fun PcSwitcher(pcs: List<PcUi>, selected: String?, onSelect: (String) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        pcs.forEach { PcPill(it, it.pcId == selected, { onSelect(it.pcId) }) }
    }
}

@Composable
private fun PcPill(pc: PcUi, selected: Boolean, onClick: () -> Unit) {
    val c = Pill.colors
    val accent = if (c.dark) c.accentSoft else c.accent
    val fill by animateColorAsState(if (selected) accent.copy(alpha = 0.16f) else c.surfaceHigh, Motion.calm(), label = "pcFill")
    val line by animateColorAsState(if (selected) accent.copy(alpha = 0.55f) else c.border, Motion.calm(), label = "pcLine")
    val text by animateColorAsState(if (selected) c.text else c.textDim, Motion.calm(), label = "pcText")
    val dotColor by animateColorAsState(
        when (pc.link) { LinkUi.Connected -> c.good; LinkUi.Connecting -> accent; else -> c.textDim },
        Motion.calm(), label = "pcDot",
    )
    val dot by animateDpAsState(if (selected) 9.dp else 7.dp, Motion.bouncy(), label = "pcDotSize")
    Row(
        Modifier.clip(CircleShape).background(fill).border(1.dp, line, CircleShape).pressable(pressedScale = 0.94f, onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(9.dp), contentAlignment = Alignment.Center) { Box(Modifier.size(dot).clip(CircleShape).background(dotColor)) }
        Spacer(Modifier.width(7.dp))
        Text(pc.name, Modifier.widthIn(max = 150.dp), style = MaterialTheme.typography.labelMedium, color = text, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** The way out of a state that is not "connected": one clear primary, the second action is quiet. */
@Composable
private fun HeroButtons(link: LinkUi, a: HomeActions) {
    val shown = link != LinkUi.Connecting && link != LinkUi.Connected
    AnimatedVisibility(shown, enter = Motion.enterItem, exit = Motion.exitItem) {
        Column(Modifier.fillMaxWidth().padding(top = 14.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
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
