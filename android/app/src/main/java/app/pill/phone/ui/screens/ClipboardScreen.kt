package app.pill.phone.ui.screens

import android.text.format.DateUtils
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.SearchOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.data.ClipItem
import app.pill.phone.ui.ClipUi
import app.pill.phone.ui.components.Banner
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.EmptyState
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.GlassIconButton
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.TopBar
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import app.pill.phone.ui.theme.PillShapes
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow

class ClipboardActions(
    /** Null on the bottom-bar tab: no back arrow. */
    val onBack: (() -> Unit)? = null,
    val onAddCurrent: () -> Unit = {},
    val onSend: (String) -> Unit = {},
    val onDelete: (ClipItem) -> Unit = {},
    val onClear: () -> Unit = {},
)

/** More items than this and a search field appears. */
private const val SEARCH_FROM = 8

/** The clipboard history: tap an item to copy it, send it to a PC, delete it, or clear all. [notices] are string resources to flash (the result of a send). */
@Composable
fun ClipboardScreen(ui: ClipUi, actions: ClipboardActions, notices: Flow<Int> = emptyFlow(), nowMs: Long? = null) {
    val c = Pill.colors
    val ctx = LocalContext.current
    val board = LocalClipboardManager.current
    var query by remember { mutableStateOf("") }
    var askClear by remember { mutableStateOf(false) }
    var toast by remember { mutableStateOf<String?>(null) }
    val now by produceState(nowMs ?: System.currentTimeMillis()) { while (nowMs == null) { delay(30_000); value = System.currentTimeMillis() } }
    LaunchedEffect(notices) { notices.collect { toast = ctx.getString(it) } }
    LaunchedEffect(toast) { if (toast != null) { delay(1600); toast = null } }
    val shown = ui.items.filter { query.isBlank() || it.text.contains(query.trim(), ignoreCase = true) }
    if (askClear) ClearHistoryDialog({ askClear = false; actions.onClear() }, { askClear = false })

    PillBackground {
        Column(Modifier.fillMaxSize()) {
            TopBar(stringResource(R.string.hist_title), actions.onBack) {
                if (ui.items.isNotEmpty()) GlassIconButton(Icons.Rounded.DeleteSweep, stringResource(R.string.hist_clear_all), { askClear = true })
            }
            LazyColumn(
                Modifier.weight(1f),
                contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                item(key = "note") {
                    Column(Modifier.staggerIn(0), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Banner(stringResource(R.string.hist_note), Icons.Rounded.Info, tint = if (c.dark) c.accentSoft else c.accent)
                        if (ui.keep) PillButton(stringResource(R.string.hist_add_current), actions.onAddCurrent, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.ContentPaste)
                    }
                }
                if (ui.items.size > SEARCH_FROM) item(key = "search") { SearchField(query, { query = it }, stringResource(R.string.hist_search), Modifier.padding(top = 4.dp)) }
                if (shown.isEmpty()) item(key = "empty") {
                    GlassCard(Modifier.padding(top = 8.dp)) {
                        when {
                            !ui.keep -> EmptyState(Icons.Rounded.History, stringResource(R.string.hist_off_title), stringResource(R.string.hist_off_body))
                            ui.items.isEmpty() -> EmptyState(Icons.Rounded.History, stringResource(R.string.hist_empty_title), stringResource(R.string.hist_empty_body))
                            else -> EmptyState(Icons.Rounded.SearchOff, stringResource(R.string.hist_no_match_title), "")
                        }
                    }
                }
                items(shown, key = { "${it.at}:${it.text.hashCode()}" }) { item ->
                    ClipRow(
                        item, now, ui.canSend,
                        onCopy = { board.setText(AnnotatedString(item.text)); toast = ctx.getString(R.string.hist_copied) },
                        onSend = { actions.onSend(item.text) }, onDelete = { actions.onDelete(item) },
                        modifier = Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm()),
                    )
                }
                item(key = "end") { NavBarSpacer() }
            }
        }
        // the short confirmation floats over the list, above the gesture bar
        Box(Modifier.fillMaxSize().navigationBarsPadding().padding(bottom = 24.dp), contentAlignment = Alignment.BottomCenter) {
            AnimatedVisibility(toast != null, enter = Motion.enterRise, exit = Motion.exitSink) {
                Row(
                    Modifier.clip(CircleShape).background(c.surfaceHigh).border(1.dp, c.border, CircleShape).padding(horizontal = 18.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Rounded.Check, null, Modifier.size(18.dp), tint = c.good)
                    Spacer(Modifier.width(8.dp))
                    Text(toast.orEmpty(), style = MaterialTheme.typography.labelLarge, color = c.text)
                }
            }
        }
    }
}

@Composable
private fun ClipRow(item: ClipItem, now: Long, canSend: Boolean, onCopy: () -> Unit, onSend: () -> Unit, onDelete: () -> Unit, modifier: Modifier) {
    val c = Pill.colors
    val shape = RoundedCornerShape(PillShapes.Row)
    val meta = "${DateUtils.getRelativeTimeSpanString(item.at, now, DateUtils.MINUTE_IN_MILLIS)} · ${item.from ?: stringResource(R.string.hist_this_phone)}"
    Row(
        modifier.fillMaxWidth().clip(shape).background(c.surface).border(1.dp, c.border, shape)
            .pressable(pressedScale = 0.98f, onClick = onCopy).padding(start = 16.dp, top = 10.dp, bottom = 10.dp, end = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(item.text.trim(), style = MaterialTheme.typography.bodyLarge, color = c.text, maxLines = 3, overflow = TextOverflow.Ellipsis)
            Text(meta, Modifier.padding(top = 4.dp), style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (canSend) RowIcon(Icons.AutoMirrored.Rounded.Send, stringResource(R.string.hist_send), onSend)
        RowIcon(Icons.Rounded.Close, stringResource(R.string.hist_delete), onDelete)
    }
}

@Composable
private fun RowIcon(icon: ImageVector, description: String, onClick: () -> Unit) {
    Box(Modifier.size(44.dp).clip(CircleShape).pressable(onClick = onClick), contentAlignment = Alignment.Center) {
        Icon(icon, description, Modifier.size(20.dp), tint = Pill.colors.textDim)
    }
}

@Composable
fun ClearHistoryDialog(onConfirm: () -> Unit, onDismiss: () -> Unit) =
    ConfirmDialog(Icons.Rounded.DeleteSweep, stringResource(R.string.hist_clear_title), stringResource(R.string.hist_clear_body), stringResource(R.string.hist_clear_all), onConfirm, onDismiss)
