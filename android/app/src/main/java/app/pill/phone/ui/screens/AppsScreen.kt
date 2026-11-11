package app.pill.phone.ui.screens

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.Image
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.SearchOff
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.R
import app.pill.phone.ui.AppRowUi
import app.pill.phone.ui.components.Banner
import app.pill.phone.ui.components.EmptyState
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillSwitch
import app.pill.phone.ui.components.TopBar
import app.pill.phone.ui.components.pressable
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class AppsActions(
    val onBack: () -> Unit = {},
    val onQuery: (String) -> Unit = {},
    val onToggle: (String, Boolean) -> Unit = { _, _ -> },
    val onShowSystem: (Boolean) -> Unit = {},
)

@Composable
fun AppsScreen(
    rows: List<AppRowUi>,
    query: String,
    showSystem: Boolean,
    enabledCount: Int,
    actions: AppsActions,
    iconLoader: suspend (String) -> ImageBitmap? = { null },
    /** Set when the list is one PC's own filter. */
    pcName: String? = null,
) {
    val c = Pill.colors
    PillBackground {
        Column(Modifier.fillMaxSize()) {
            TopBar(if (pcName != null) stringResource(R.string.apps_title_for, pcName) else stringResource(R.string.apps_title), actions.onBack)
            LazyColumn(
                Modifier.weight(1f),
                contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                item(key = "explain") {
                    Banner(stringResource(R.string.apps_explainer), Icons.Rounded.Info, Modifier.staggerIn(0), tint = c.accentSoft.let { if (c.dark) it else c.accent })
                }
                item(key = "search") {
                    Column(Modifier.padding(top = 6.dp).staggerIn(1)) {
                        SearchField(query, actions.onQuery)
                        Spacer(Modifier.height(12.dp))
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                stringResource(R.string.apps_count, enabledCount), Modifier.weight(1f).padding(start = 6.dp),
                                style = MaterialTheme.typography.labelMedium, color = c.textDim,
                            )
                            Text(stringResource(R.string.apps_show_system), style = MaterialTheme.typography.labelMedium, color = c.textDim)
                            Spacer(Modifier.width(8.dp))
                            PillSwitch(showSystem, actions.onShowSystem)
                        }
                    }
                }
                if (rows.isEmpty()) {
                    item(key = "empty") {
                        GlassCard(Modifier.padding(top = 8.dp)) {
                            EmptyState(Icons.Rounded.SearchOff, stringResource(R.string.apps_empty_title), stringResource(R.string.apps_empty_body, query))
                        }
                    }
                }
                items(rows, key = { it.pkg }) { row ->
                    AppRow(row, iconLoader, actions.onToggle, Modifier.animateItem(fadeInSpec = Motion.calm(), placementSpec = Motion.soft(), fadeOutSpec = Motion.calm()))
                }
                item(key = "end") { NavBarSpacer() }
            }
        }
    }
}

@Composable
private fun AppRow(row: AppRowUi, iconLoader: suspend (String) -> ImageBitmap?, onToggle: (String, Boolean) -> Unit, modifier: Modifier) {
    val c = Pill.colors
    val shape = RoundedCornerShape(20.dp)
    val border by animateColorAsState(if (row.enabled) c.accent.copy(alpha = 0.35f) else c.border, Motion.calm(), label = "rowBorder")
    Row(
        modifier.fillMaxWidth().clip(shape).background(c.surface).border(1.dp, border, shape)
            .pressable(pressedScale = 0.98f) { onToggle(row.pkg, !row.enabled) }.padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AppIcon(row, iconLoader)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(row.label, style = MaterialTheme.typography.titleSmall.copy(fontSize = 16.sp), color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(row.pkg, style = MaterialTheme.typography.bodySmall, color = c.textDim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Spacer(Modifier.width(8.dp))
        PillSwitch(row.enabled, { onToggle(row.pkg, it) })
    }
}

@Composable
private fun AppIcon(row: AppRowUi, loader: suspend (String) -> ImageBitmap?) {
    val c = Pill.colors
    var bmp by remember(row.pkg) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(row.pkg) { bmp = withContext(Dispatchers.IO) { loader(row.pkg) } }
    Box(Modifier.size(42.dp).clip(RoundedCornerShape(13.dp)).background(c.accent.copy(alpha = 0.16f)), contentAlignment = Alignment.Center) {
        val b = bmp
        if (b != null) Image(b, null, Modifier.fillMaxSize())
        else Text(row.label.take(1).uppercase(), style = MaterialTheme.typography.titleMedium, color = if (c.dark) c.accentSoft else c.accent)
    }
}

@Composable
private fun SearchField(query: String, onQuery: (String) -> Unit) {
    val c = Pill.colors
    Row(
        Modifier.fillMaxWidth().clip(CircleShape).background(c.surface).border(1.dp, c.border, CircleShape).padding(horizontal = 18.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Search, null, Modifier.size(20.dp), tint = c.textDim)
        Spacer(Modifier.width(10.dp))
        Box(Modifier.weight(1f)) {
            if (query.isEmpty()) Text(stringResource(R.string.apps_search), style = MaterialTheme.typography.bodyLarge, color = c.textDim.copy(alpha = 0.7f))
            BasicTextField(
                query, onQuery, singleLine = true, textStyle = MaterialTheme.typography.bodyLarge.copy(color = c.text),
                cursorBrush = SolidColor(c.accent), modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search, autoCorrectEnabled = false),
            )
        }
        if (query.isNotEmpty()) {
            Spacer(Modifier.width(8.dp))
            Icon(Icons.Rounded.Close, stringResource(R.string.clear), Modifier.size(20.dp).pressable { onQuery("") }, tint = c.textDim)
        }
    }
}
