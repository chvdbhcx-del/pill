package app.pill.phone.ui.components

import androidx.annotation.StringRes
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.R
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

/** The four top-level screens, in bar order. */
enum class PillTab(val icon: ImageVector, @StringRes val label: Int) {
    Home(Icons.Rounded.Home, R.string.tab_home),
    Files(Icons.Rounded.Folder, R.string.files_title),
    Clipboard(Icons.Rounded.ContentPaste, R.string.tab_clipboard),
    Apps(Icons.Rounded.Notifications, R.string.apps_card_title),
}

/** Bottom navigation: the current tab is an accent pill (white on black, black on white) that springs in behind its icon. */
@Composable
fun BottomBar(selected: PillTab, onSelect: (PillTab) -> Unit, modifier: Modifier = Modifier) {
    val c = Pill.colors
    Column(modifier.background(c.background)) {
        Divider()
        Row(Modifier.fillMaxWidth().navigationBarsPadding().padding(top = 8.dp, bottom = 6.dp)) {
            PillTab.entries.forEach { TabItem(it, it == selected, { onSelect(it) }, Modifier.weight(1f)) }
        }
    }
}

@Composable
private fun TabItem(tab: PillTab, on: Boolean, onClick: () -> Unit, modifier: Modifier) {
    val c = Pill.colors
    val ink by animateColorAsState(if (on) c.onAccent else c.textDim, Motion.calm(), label = "tabInk")
    val label by animateColorAsState(if (on) c.text else c.textDim, Motion.calm(), label = "tabLabel")
    val width by animateFloatAsState(if (on) 1f else 0.4f, Motion.bouncy(), label = "tabPill")
    val fill by animateFloatAsState(if (on) 1f else 0f, Motion.calm(), label = "tabFill")
    Column(
        modifier.defaultMinSize(minHeight = 48.dp).semantics(mergeDescendants = true) { selected = on }.pressable(role = Role.Tab, pressedScale = 0.92f, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.width(56.dp).height(30.dp), contentAlignment = Alignment.Center) {
            Box(Modifier.size(56.dp, 30.dp).graphicsLayer { scaleX = width; alpha = fill }.clip(CircleShape).background(c.accent))
            Icon(tab.icon, null, Modifier.size(22.dp), tint = ink)
        }
        Spacer(Modifier.height(4.dp))
        Text(stringResource(tab.label), style = MaterialTheme.typography.labelSmall.copy(fontSize = 11.sp), color = label, maxLines = 1)
    }
}
