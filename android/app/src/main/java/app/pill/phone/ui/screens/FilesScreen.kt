package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import app.pill.phone.ui.components.EmptyState
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.SectionLabel
import app.pill.phone.ui.components.SpringProgress
import app.pill.phone.ui.components.TopBar
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

/** Files: send some to the PC, and the transfers of this session (sent and received, newest first). */
@Composable
fun FilesScreen(ui: HomeUi, actions: HomeActions) {
    PillBackground {
        Column(Modifier.fillMaxSize()) {
            TopBar(stringResource(R.string.files_title), null)
            LazyColumn(
                Modifier.weight(1f),
                contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                item(key = "send") { PillButton(stringResource(R.string.btn_send_files), actions.onSendFiles, Modifier.fillMaxWidth().staggerIn(0), icon = Icons.Rounded.Upload) }
                if (ui.transfers.isEmpty()) {
                    item(key = "empty") {
                        GlassCard(Modifier.staggerIn(1)) { EmptyState(Icons.Rounded.FolderOpen, stringResource(R.string.files_empty_title), stringResource(R.string.files_empty_body)) }
                    }
                } else {
                    item(key = "label") { SectionLabel(stringResource(R.string.recent_title), Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm())) }
                    items(ui.transfers, key = { "t-" + it.id }) { t ->
                        TransferCard(t, actions, Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm()))
                    }
                }
                item(key = "end") { NavBarSpacer() }
            }
        }
    }
}

/** The status line of a transfer: "Sent  ·  4.2 MB  ·  PC name". */
@Composable
internal fun transferLine(t: TransferUi): String {
    val status = when (t.phase) {
        TransferPhase.Offered -> stringResource(R.string.tr_offered)
        TransferPhase.Waiting -> stringResource(R.string.tr_waiting)
        TransferPhase.Active -> stringResource(if (t.outgoing) R.string.tr_sending else R.string.tr_receiving)
        TransferPhase.Done -> stringResource(if (t.outgoing) R.string.tr_sent else R.string.tr_received)
        TransferPhase.Failed -> t.detail ?: stringResource(R.string.tr_failed)
    }
    return "$status  ·  ${t.sizeText}" + (t.pc?.let { "  ·  $it" } ?: "")
}

/** Direction arrow of a transfer, green when done, red when failed. */
@Composable
internal fun TransferBadge(t: TransferUi, size: Dp) {
    val c = Pill.colors
    IconBadge(
        when {
            t.phase == TransferPhase.Failed -> Icons.Rounded.ErrorOutline
            t.outgoing -> Icons.Rounded.ArrowUpward
            else -> Icons.Rounded.ArrowDownward
        },
        tint = when (t.phase) { TransferPhase.Failed -> c.bad; TransferPhase.Done -> c.good; else -> c.accentSoft }, size = size,
    )
}

@Composable
private fun TransferCard(t: TransferUi, a: HomeActions, modifier: Modifier) {
    val c = Pill.colors
    GlassCard(modifier.animateContentSize(Motion.soft()), contentPadding = PaddingValues(horizontal = 14.dp, vertical = 12.dp)) {
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            TransferBadge(t, 42.dp)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(t.name, style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(2.dp))
                Text(
                    transferLine(t), style = MaterialTheme.typography.bodySmall,
                    color = if (t.phase == TransferPhase.Failed) c.bad else c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            Spacer(Modifier.width(8.dp))
            TransferAction(t, a)
        }
        AnimatedVisibility(t.phase == TransferPhase.Active || t.phase == TransferPhase.Waiting, enter = Motion.enterItem, exit = Motion.exitItem) {
            Box(Modifier.padding(top = 12.dp)) { SpringProgress(if (t.phase == TransferPhase.Waiting) 0.04f else t.progress, height = 6.dp) }
        }
    }
}
