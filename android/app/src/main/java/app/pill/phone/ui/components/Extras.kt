package app.pill.phone.ui.components

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.ui.theme.LocalReduceMotion
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import kotlinx.coroutines.delay

/** Items of a screen rise into place one after another, each on a soft spring. */
fun Modifier.staggerIn(index: Int, stepMs: Long = 55): Modifier = composed {
    val reduce = LocalReduceMotion.current
    val p = remember { Animatable(if (reduce) 1f else 0f) }
    LaunchedEffect(Unit) {
        if (!reduce) {
            delay(index * stepMs)
            p.animateTo(1f, Motion.soft())
        }
    }
    graphicsLayer {
        val v = p.value
        alpha = v.coerceIn(0f, 1f)
        translationY = (1f - v) * 28.dp.toPx()
        scaleX = 0.94f + 0.06f * v
        scaleY = 0.94f + 0.06f * v
    }
}

/** Compact pill button for rows (Allow, Open, Receive). */
@Composable
fun SmallButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, filled: Boolean = true) {
    val c = Pill.colors
    val shape = CircleShape
    val m = if (filled) modifier.clip(shape).background(c.accent) else modifier.clip(shape).background(c.accent.copy(alpha = 0.14f)).border(1.dp, c.accent.copy(alpha = 0.35f), shape)
    Box(m.pressable(onClick = onClick).padding(horizontal = 16.dp, vertical = 9.dp), contentAlignment = Alignment.Center) {
        Text(text, style = MaterialTheme.typography.labelMedium, color = if (filled) c.onAccent else (if (c.dark) c.accentSoft else c.accent), maxLines = 1)
    }
}

/** A round green tick: the "done" mark of a checklist. */
@Composable
fun DoneTick(modifier: Modifier = Modifier) {
    val c = Pill.colors
    Box(modifier.size(30.dp).clip(CircleShape).background(c.good.copy(alpha = 0.16f)).border(1.dp, c.good.copy(alpha = 0.4f), CircleShape), contentAlignment = Alignment.Center) {
        Icon(Icons.Rounded.Check, null, Modifier.size(18.dp), tint = c.good)
    }
}

@Composable
fun StepRow(icon: ImageVector, title: String, body: String, modifier: Modifier = Modifier) {
    val c = Pill.colors
    Row(modifier, verticalAlignment = Alignment.Top) {
        IconBadge(icon)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall.copy(fontSize = 16.sp), color = c.text)
            Spacer(Modifier.height(2.dp))
            Text(body, style = MaterialTheme.typography.bodyMedium, color = c.textDim)
        }
    }
}
