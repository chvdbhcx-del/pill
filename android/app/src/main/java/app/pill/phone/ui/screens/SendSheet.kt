package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.data.PcOption
import app.pill.phone.ui.components.BlobState
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.StateBlob
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

sealed interface SendUi {
    data object Preparing : SendUi
    /** Two or more PCs are paired: the user picks (or says all). [count] files wait. */
    data class Choose(val count: Int) : SendUi
    data class Sending(val count: Int) : SendUi
    data object NotConnected : SendUi
    data object Nothing : SendUi
    // ---- clipboard (the share target's text, the tile, the notification action, the Home button) ----
    data object ClipSent : SendUi
    data object ClipEmpty : SendUi
    data object ClipTooLarge : SendUi
    data object ClipOff : SendUi
}

/** The "Send to PC" share target: a small glass sheet over the app the user came from. */
@Composable
fun SendSheet(
    state: SendUi,
    onOpenApp: () -> Unit,
    onClose: () -> Unit,
    options: List<PcOption> = emptyList(),
    onPick: (List<String>) -> Unit = {},
) {
    val c = Pill.colors
    SheetShell(visible = true, onClose = onClose) {
        if (state is SendUi.Choose) {
            PcPickerContent(options, state.count, onPick, onClose)
            return@SheetShell
        }
        StateBlob(
            when (state) {
                SendUi.Preparing -> BlobState.Connecting
                is SendUi.Sending, SendUi.ClipSent -> BlobState.Connected
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
                SendUi.Preparing, is SendUi.Choose -> stringResource(R.string.send_title) to ""
                is SendUi.Sending -> pluralStringResource(R.plurals.send_sending, s.count, s.count) to stringResource(R.string.send_title)
                SendUi.NotConnected -> stringResource(R.string.send_not_connected_title) to stringResource(R.string.send_not_connected_body)
                SendUi.Nothing -> stringResource(R.string.send_nothing_title) to stringResource(R.string.send_nothing_body)
                SendUi.ClipSent -> stringResource(R.string.clip_sent_title) to stringResource(R.string.clip_sent_body)
                SendUi.ClipEmpty -> stringResource(R.string.clip_empty_title) to stringResource(R.string.clip_empty_body)
                SendUi.ClipTooLarge -> stringResource(R.string.clip_big_title) to stringResource(R.string.clip_big_body)
                SendUi.ClipOff -> stringResource(R.string.clip_off_title) to stringResource(R.string.clip_off_body)
            }
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(title, style = MaterialTheme.typography.headlineSmall, color = c.text, textAlign = TextAlign.Center)
                if (body.isNotEmpty()) {
                    Spacer(Modifier.height(6.dp))
                    Text(body, style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
                }
            }
        }
        if (state == SendUi.NotConnected || state == SendUi.Nothing || state == SendUi.ClipOff) {
            Spacer(Modifier.height(20.dp))
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                if (state == SendUi.NotConnected || state == SendUi.ClipOff) PillButton(stringResource(R.string.send_open_app), onOpenApp, Modifier.fillMaxWidth())
                PillButton(stringResource(R.string.send_close), onClose, Modifier.fillMaxWidth(), kind = ButtonKind.Glass)
            }
        }
    }
}
