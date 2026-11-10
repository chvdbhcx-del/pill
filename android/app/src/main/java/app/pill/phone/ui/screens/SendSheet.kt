package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.ui.components.BlobState
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.StateBlob
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

sealed interface SendUi {
    data object Preparing : SendUi
    data class Sending(val count: Int) : SendUi
    data object NotConnected : SendUi
    data object Nothing : SendUi
}

/** The "Send to PC" share target: a small glass sheet over the app the user came from. */
@Composable
fun SendSheet(state: SendUi, onOpenApp: () -> Unit, onClose: () -> Unit) {
    val c = Pill.colors
    val visible = remember { MutableTransitionState(false).apply { targetState = true } }
    Box(
        Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f))
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose),
        contentAlignment = Alignment.BottomCenter,
    ) {
        AnimatedVisibility(visible, enter = Motion.enterRise, exit = Motion.exitSink) {
            val shape = RoundedCornerShape(28.dp)
            Column(
                Modifier.navigationBarsPadding().padding(16.dp).fillMaxWidth().clip(shape).background(c.surface)
                    .border(1.dp, c.border, shape)
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { }
                    .padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                StateBlob(
                    when (state) {
                        SendUi.Preparing -> BlobState.Connecting
                        is SendUi.Sending -> BlobState.Connected
                        else -> BlobState.Offline
                    },
                    size = 132.dp,
                )
                AnimatedContent(
                    state,
                    transitionSpec = { (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.94f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm())) },
                    label = "sendText",
                ) { s ->
                    val (title, body) = when (s) {
                        SendUi.Preparing -> stringResource(R.string.send_title) to ""
                        is SendUi.Sending -> pluralStringResource(R.plurals.send_sending, s.count, s.count) to stringResource(R.string.send_title)
                        SendUi.NotConnected -> stringResource(R.string.send_not_connected_title) to stringResource(R.string.send_not_connected_body)
                        SendUi.Nothing -> stringResource(R.string.send_nothing_title) to stringResource(R.string.send_nothing_body)
                    }
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(title, style = MaterialTheme.typography.headlineSmall, color = c.text, textAlign = TextAlign.Center)
                        if (body.isNotEmpty()) {
                            Spacer(Modifier.height(6.dp))
                            Text(body, style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
                        }
                    }
                }
                if (state == SendUi.NotConnected || state == SendUi.Nothing) {
                    Spacer(Modifier.height(20.dp))
                    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        if (state == SendUi.NotConnected) PillButton(stringResource(R.string.send_open_app), onOpenApp, Modifier.fillMaxWidth())
                        PillButton(stringResource(R.string.send_close), onClose, Modifier.fillMaxWidth(), kind = ButtonKind.Glass)
                    }
                }
            }
        }
    }
}
