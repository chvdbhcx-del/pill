package app.pill.phone.ui.screens

import android.text.format.DateUtils
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material.icons.rounded.SystemUpdate
import androidx.compose.material.icons.rounded.Update
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.data.UpdateFail
import app.pill.phone.data.UpdatePhase
import app.pill.phone.ui.UpdateUi
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.Divider
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.PillSwitch
import app.pill.phone.ui.components.SettingRow
import app.pill.phone.ui.components.SmallButton
import app.pill.phone.ui.components.SpringProgress
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.theme.LocalReduceMotion
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

class UpdateActions(
    val onCheck: () -> Unit = {},
    val onUpdate: () -> Unit = {},
    val onAuto: (Boolean) -> Unit = {},
    val onDismiss: () -> Unit = {},
    /** The Home card's body: Settings, scrolled to the Updates section. */
    val onOpen: () -> Unit = {},
)

private val UpdatePhase.busy get() = this == UpdatePhase.Checking || this == UpdatePhase.Downloading || this == UpdatePhase.Verifying || this == UpdatePhase.Installing

/** One line each: what the user reads when something went wrong. */
@Composable
private fun failText(f: UpdateFail?): String = stringResource(
    when (f) {
        UpdateFail.Check, null -> R.string.upd_fail_check
        UpdateFail.Server -> R.string.upd_fail_server
        UpdateFail.Download -> R.string.upd_fail_download
        UpdateFail.Mismatch -> R.string.upd_fail_mismatch
        UpdateFail.Storage -> R.string.upd_fail_storage
        UpdateFail.Install -> R.string.upd_fail_install
    },
)

/** The sentence under the title: what is happening, or what to do. */
@Composable
private fun detail(ui: UpdateUi): String = when (ui.phase) {
    UpdatePhase.Downloading -> stringResource(R.string.upd_downloading_pct, (ui.progress * 100).toInt())
    UpdatePhase.Verifying -> stringResource(R.string.upd_status_verifying)
    UpdatePhase.Installing -> stringResource(R.string.upd_status_installing)
    UpdatePhase.NeedsPermission -> stringResource(R.string.upd_perm_body)
    UpdatePhase.Failed -> failText(ui.fail)
    else -> stringResource(R.string.upd_size_line, ui.sizeText)
}

/** An endless bar for the two steps that have no percentage; a still, full bar when motion is reduced. */
@Composable
private fun BusyBar(modifier: Modifier = Modifier) {
    val c = Pill.colors
    val reduce = LocalReduceMotion.current
    val x = if (reduce) 0f else rememberInfiniteTransition(label = "updBusy").animateFloat(
        -1f, 2.5f, infiniteRepeatable(tween(1300, easing = LinearEasing), RepeatMode.Restart), label = "updBusyX",
    ).value
    Box(modifier.fillMaxWidth().height(6.dp).clip(CircleShape).background(c.border.copy(alpha = 0.10f))) {
        Box(
            Modifier.fillMaxHeight().fillMaxWidth(if (reduce) 1f else 0.4f)
                .graphicsLayer { translationX = x * size.width }
                .clip(CircleShape).background(Brush.horizontalGradient(listOf(c.accentSoft, c.accent))),
        )
    }
}

/** Progress under the title while the update is on its way: percentage bar for the download, endless bar after. */
@Composable
private fun Progress(ui: UpdateUi, modifier: Modifier = Modifier) {
    AnimatedVisibility(
        ui.phase == UpdatePhase.Downloading || ui.phase == UpdatePhase.Verifying || ui.phase == UpdatePhase.Installing,
        modifier, enter = Motion.enterItem, exit = Motion.exitItem,
    ) {
        AnimatedContent(
            ui.phase == UpdatePhase.Downloading,
            transitionSpec = { fadeIn(Motion.calm()) togetherWith fadeOut(Motion.calm()) }, label = "updBar",
        ) { determinate ->
            if (determinate) SpringProgress(ui.progress, height = 6.dp) else BusyBar()
        }
    }
}

private fun statusSwap() = (fadeIn(Motion.calm()) + slideInVertically(Motion.soft()) { it / 3 }) togetherWith (fadeOut(Motion.calm()) + slideOutVertically(Motion.calm()) { -it / 3 })

/** Home: a small card under the hero, only while a newer build is on offer and the user has not closed it for that build. */
@Composable
fun UpdateCard(ui: UpdateUi, a: UpdateActions, modifier: Modifier = Modifier) {
    val c = Pill.colors
    val actionable = ui.phase == UpdatePhase.Available || ui.phase == UpdatePhase.NeedsPermission || ui.phase == UpdatePhase.Failed || ui.phase == UpdatePhase.Checking
    GlassCard(
        modifier.animateContentSize(Motion.soft()),
        onClick = a.onOpen,
        contentPadding = PaddingValues(start = 14.dp, end = 8.dp, top = 12.dp, bottom = 12.dp),
        accentBorder = ui.phase == UpdatePhase.Available,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBadge(Icons.Rounded.SystemUpdate, tint = if (ui.phase == UpdatePhase.Failed) c.bad else c.accentSoft, size = 38.dp)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.upd_card_title, ui.latest.orEmpty()), style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                AnimatedContent(ui.phase to ui.fail, transitionSpec = { statusSwap() }, label = "updCardLine") { _ ->
                    Text(
                        detail(ui), Modifier.padding(top = 2.dp), style = MaterialTheme.typography.bodySmall,
                        color = if (ui.phase == UpdatePhase.Failed) c.bad else c.textDim, maxLines = 3, overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            // busy steps cannot be dismissed (the install is on its way); the cross comes back with the buttons
            AnimatedVisibility(!ui.phase.busy || ui.phase == UpdatePhase.Checking, enter = Motion.enterItem, exit = Motion.exitItem) {
                Box(
                    Modifier.size(40.dp).clip(CircleShape).pressable(onClick = a.onDismiss),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Rounded.Close, stringResource(R.string.upd_dismiss), Modifier.size(18.dp), tint = c.textDim) }
            }
        }
        AnimatedVisibility(actionable && ui.phase != UpdatePhase.Checking, enter = Motion.enterItem, exit = Motion.exitItem) {
            Box(Modifier.padding(top = 10.dp, end = 6.dp)) {
                AnimatedContent(ui.phase, transitionSpec = { fadeIn(Motion.calm()) togetherWith fadeOut(Motion.calm()) }, label = "updCardBtn") { p ->
                    SmallButton(
                        stringResource(if (p == UpdatePhase.Failed) R.string.btn_try_again else if (p == UpdatePhase.NeedsPermission) R.string.upd_btn_allow else R.string.upd_btn_update),
                        a.onUpdate, Modifier.fillMaxWidth().heightIn(min = 44.dp),
                    )
                }
            }
        }
        Progress(ui, Modifier.padding(top = 12.dp, end = 6.dp))
    }
}

/** Settings > Updates: version, last check, status, "Check now", the automatic switch, and the update itself with its notes. */
@Composable
fun UpdatesCard(ui: UpdateUi, version: String, a: UpdateActions, modifier: Modifier = Modifier) {
    val c = Pill.colors
    GlassCard(modifier.animateContentSize(Motion.soft())) {
        val (title, icon, tint) = when (ui.phase) {
            UpdatePhase.Checking -> Triple(R.string.upd_status_checking, Icons.Rounded.Sync, c.accentSoft)
            UpdatePhase.UpToDate -> Triple(R.string.upd_status_current, Icons.Rounded.CheckCircle, c.good)
            UpdatePhase.Available -> Triple(R.string.upd_status_available, Icons.Rounded.SystemUpdate, c.accentSoft)
            UpdatePhase.Downloading -> Triple(R.string.upd_status_downloading, Icons.Rounded.SystemUpdate, c.accentSoft)
            UpdatePhase.Verifying -> Triple(R.string.upd_status_verifying, Icons.Rounded.SystemUpdate, c.accentSoft)
            UpdatePhase.Installing -> Triple(R.string.upd_status_installing, Icons.Rounded.SystemUpdate, c.accentSoft)
            UpdatePhase.NeedsPermission -> Triple(R.string.upd_status_permission, Icons.Rounded.Lock, c.warn)
            UpdatePhase.Failed -> Triple(R.string.upd_status_failed, Icons.Rounded.ErrorOutline, c.bad)
            UpdatePhase.Idle -> Triple(R.string.upd_status_never, Icons.Rounded.Update, c.accentSoft)
        }
        AnimatedContent(ui.phase to ui.fail, transitionSpec = { statusSwap() }, label = "updStatus") { (phase, _) ->
            val last = if (ui.lastCheck > 0) {
                stringResource(R.string.upd_last_checked, DateUtils.getRelativeTimeSpanString(ui.lastCheck, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString())
            } else stringResource(R.string.upd_never_checked)
            val sub = when (phase) {
                UpdatePhase.Failed, UpdatePhase.NeedsPermission -> detail(ui)
                else -> stringResource(R.string.settings_version, version) + "  ·  " + last
            }
            SettingRow(stringResource(title, ui.latest.orEmpty()), subtitle = sub, icon = icon, iconTint = tint)
        }
        // the update itself: notes, progress, the button
        AnimatedVisibility(ui.latest != null, enter = Motion.enterItem, exit = Motion.exitItem) {
            Column {
                if (ui.notes.isNotBlank()) {
                    Divider(Modifier.padding(vertical = 8.dp))
                    Text(stringResource(R.string.upd_notes_title).uppercase(), style = MaterialTheme.typography.labelSmall, color = c.textDim)
                    Text(ui.notes, Modifier.padding(top = 4.dp), style = MaterialTheme.typography.bodyMedium, color = c.text)
                }
                Progress(ui, Modifier.padding(top = 14.dp))
                Spacer(Modifier.padding(top = 14.dp))
                val busy = ui.phase.busy
                AnimatedContent((ui.phase == UpdatePhase.Failed) to (ui.phase == UpdatePhase.NeedsPermission), transitionSpec = { fadeIn(Motion.calm()) togetherWith fadeOut(Motion.calm()) }, label = "updBtn") { (failed, perm) ->
                    val text = when {
                        failed -> stringResource(R.string.btn_try_again)
                        perm -> stringResource(R.string.upd_btn_allow_installs)
                        else -> stringResource(R.string.upd_btn_update_to, ui.latest.orEmpty())
                    }
                    PillButton(text, a.onUpdate, Modifier.fillMaxWidth(), loading = busy, enabled = !busy)
                }
            }
        }
        Divider(Modifier.padding(vertical = 8.dp))
        PillButton(
            stringResource(R.string.upd_btn_check), a.onCheck, Modifier.fillMaxWidth(),
            kind = if (ui.latest != null) ButtonKind.Quiet else ButtonKind.Glass, loading = ui.phase == UpdatePhase.Checking, enabled = !ui.phase.busy,
        )
        Divider(Modifier.padding(vertical = 8.dp))
        SettingRow(stringResource(R.string.upd_auto), subtitle = stringResource(R.string.upd_auto_body), icon = Icons.Rounded.Update) {
            PillSwitch(ui.auto, a.onAuto)
        }
    }
}
