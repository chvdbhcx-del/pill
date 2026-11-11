package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowForward
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.data.PcOption
import app.pill.phone.net.Route
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

/** "Home network" / "Internet" / "Offline" for one PC. */
@Composable
fun routeWord(route: Route?, online: Boolean): String = stringResource(
    when {
        !online || route == null -> R.string.route_offline
        route == Route.Lan -> R.string.route_home
        else -> R.string.route_internet
    },
)

/**
 * The glass sheet that rises from the bottom over a dimmed screen; shared by the share target and the in-app overlay.
 * [visible] false plays the exit (scrim fades, panel sinks) before the content leaves the tree.
 */
@Composable
fun SheetShell(visible: Boolean, onClose: () -> Unit, content: @Composable () -> Unit) {
    val c = Pill.colors
    val state = remember { MutableTransitionState(false) }.apply { targetState = visible }
    AnimatedVisibility(state, enter = fadeIn(Motion.calm()), exit = fadeOut(Motion.calm())) {
        Box(
            Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f))
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose),
            contentAlignment = Alignment.BottomCenter,
        ) {
            AnimatedVisibility(state, enter = Motion.enterRise, exit = Motion.exitSink) {
                val shape = RoundedCornerShape(28.dp)
                Column(
                    Modifier.navigationBarsPadding().padding(16.dp).fillMaxWidth().clip(shape).background(c.surface)
                        .border(1.dp, c.border, shape)
                        .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { }
                        .padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) { content() }
            }
        }
    }
}

/** The question "send to which PC?": one row per PC with its route; offline PCs are dimmed and cannot be picked. */
@Composable
fun PcPickerContent(options: List<PcOption>, fileCount: Int, onPick: (List<String>) -> Unit, onClose: () -> Unit) {
    val c = Pill.colors
    val online = options.filter { it.online }
    Box(Modifier.clip(CircleShape).background(c.accent.copy(alpha = 0.14f)).padding(14.dp)) {
        Icon(Icons.Rounded.Devices, null, Modifier.size(26.dp), tint = if (c.dark) c.accentSoft else c.accent)
    }
    Spacer(Modifier.height(14.dp))
    Text(stringResource(R.string.pick_title), style = MaterialTheme.typography.headlineSmall, color = c.text, textAlign = TextAlign.Center)
    if (fileCount > 0) {
        Spacer(Modifier.height(4.dp))
        Text(pluralStringResource(R.plurals.pick_files, fileCount, fileCount), style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
    }
    Spacer(Modifier.height(18.dp))
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        options.forEachIndexed { i, o -> PcPickRow(o, { onPick(listOf(o.pcId)) }, Modifier.staggerIn(i)) }
    }
    if (online.isEmpty()) {
        Spacer(Modifier.height(14.dp))
        Text(stringResource(R.string.pick_none), style = MaterialTheme.typography.bodyMedium, color = c.textDim, textAlign = TextAlign.Center)
    }
    Spacer(Modifier.height(18.dp))
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        // "send to both": only worth offering while two or more can take it
        AnimatedVisibility(online.size >= 2, enter = Motion.enterItem, exit = Motion.exitItem) {
            PillButton(stringResource(R.string.pick_all), { onPick(online.map { it.pcId }) }, Modifier.fillMaxWidth())
        }
        PillButton(stringResource(R.string.btn_cancel), onClose, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet)
    }
}

@Composable
private fun PcPickRow(o: PcOption, onClick: () -> Unit, modifier: Modifier) {
    val c = Pill.colors
    val dot by animateColorAsState(if (o.online) c.good else c.textDim, Motion.calm(), label = "pickDot")
    val shape = RoundedCornerShape(20.dp)
    Row(
        modifier.fillMaxWidth().alpha(if (o.online) 1f else 0.45f).clip(shape).background(c.surfaceHigh).border(1.dp, c.border, shape)
            .pressable(enabled = o.online, pressedScale = 0.97f, onClick = onClick).padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Computer, null, Modifier.size(24.dp), tint = if (c.dark) c.accentSoft else c.accent)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(o.name, style = MaterialTheme.typography.titleSmall, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Spacer(Modifier.height(3.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(8.dp).clip(CircleShape).background(dot))
                Spacer(Modifier.width(7.dp))
                Text(routeWord(o.route, o.online), style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 1)
            }
        }
        if (o.online) Icon(Icons.AutoMirrored.Rounded.ArrowForward, null, Modifier.size(20.dp), tint = c.textDim)
    }
}

/** The in-app overlay version of the picker. */
@Composable
fun PcPickerSheet(visible: Boolean, options: List<PcOption>, fileCount: Int, onPick: (List<String>) -> Unit, onClose: () -> Unit) {
    SheetShell(visible, onClose) { PcPickerContent(options, fileCount, onPick, onClose) }
}
