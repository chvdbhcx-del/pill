package app.pill.phone.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.composed
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import app.pill.phone.ui.theme.PillShapes

/** Press feedback shared by every tappable thing: a springy squash, no ripple. */
fun Modifier.pressable(
    enabled: Boolean = true,
    role: Role = Role.Button,
    pressedScale: Float = 0.95f,
    onClick: () -> Unit,
): Modifier = composed {
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed && enabled) pressedScale else 1f, Motion.bouncy(), label = "press")
    graphicsLayer { scaleX = scale; scaleY = scale }
        .clickable(interactionSource = source, indication = null, enabled = enabled, role = role, onClick = onClick)
}

/** Page background with two soft violet glows, like the desktop island's ambient light. */
@Composable
fun PillBackground(modifier: Modifier = Modifier, content: @Composable BoxScope.() -> Unit) {
    val c = Pill.colors
    Box(
        modifier
            .fillMaxSize()
            .background(c.background)
            .drawAmbient(c.glow, c.dark),
        content = content,
    )
}

private fun Modifier.drawAmbient(glow: Color, dark: Boolean): Modifier = drawBehind {
    val w = size.width
    val h = size.height
    drawRect(
        Brush.radialGradient(
            listOf(glow.copy(alpha = if (dark) 0.20f else 0.16f), Color.Transparent),
            center = Offset(w * 0.15f, -h * 0.02f), radius = w * 1.15f,
        ),
    )
    drawRect(
        Brush.radialGradient(
            listOf(glow.copy(alpha = 0.10f), Color.Transparent),
            center = Offset(w * 1.0f, h * 1.0f), radius = w * 1.2f,
        ),
    )
}

@Composable
fun GlassCard(
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    contentPadding: PaddingValues = PaddingValues(20.dp),
    accentBorder: Boolean = false,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = Pill.colors
    val shape = RoundedCornerShape(PillShapes.Card)
    var m = modifier
    if (!c.dark) m = m.shadow(10.dp, shape, ambientColor = c.accent.copy(alpha = 0.10f), spotColor = c.accent.copy(alpha = 0.16f))
    m = m.clip(shape)
        .background(Brush.verticalGradient(if (c.dark) listOf(c.surfaceHigh.copy(alpha = 0.85f), c.surface) else listOf(c.surface, c.surface)))
        .border(BorderStroke(1.dp, if (accentBorder) c.accent.copy(alpha = 0.45f) else c.border), shape)
    if (onClick != null) m = m.pressable(pressedScale = 0.975f, onClick = onClick)
    Column(m.padding(contentPadding), content = content)
}

enum class ButtonKind { Primary, Glass, Danger, Quiet }

@Composable
fun PillButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: ButtonKind = ButtonKind.Primary,
    icon: ImageVector? = null,
    loading: Boolean = false,
    enabled: Boolean = true,
) {
    val c = Pill.colors
    val fg = when (kind) {
        ButtonKind.Primary -> c.onAccent
        ButtonKind.Glass -> c.text
        ButtonKind.Danger -> c.bad
        ButtonKind.Quiet -> c.accentSoft.let { if (c.dark) it else c.accent }
    }
    val shape = CircleShape
    val base = modifier.height(56.dp).alpha(if (enabled) 1f else 0.45f)
    val styled = when (kind) {
        ButtonKind.Primary -> base
            .shadow(if (enabled) 14.dp else 0.dp, shape, ambientColor = c.accent.copy(alpha = 0.3f), spotColor = c.accent.copy(alpha = 0.55f))
            .clip(shape)
            .background(Brush.horizontalGradient(if (c.dark) listOf(c.accentSoft, c.accent) else listOf(c.accent, Color(0xFF8E7BFF))))
        ButtonKind.Glass -> base.clip(shape).background(c.surfaceHigh).border(1.dp, c.border, shape)
        ButtonKind.Danger -> base.clip(shape).background(c.bad.copy(alpha = 0.12f)).border(1.dp, c.bad.copy(alpha = 0.35f), shape)
        ButtonKind.Quiet -> base.clip(shape)
    }
    Row(
        styled.pressable(enabled = enabled && !loading, onClick = onClick).padding(horizontal = 26.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (loading) {
            CircularProgressIndicator(Modifier.size(20.dp), color = fg, strokeWidth = 2.5.dp)
            Spacer(Modifier.width(10.dp))
        } else if (icon != null) {
            Icon(icon, null, Modifier.size(20.dp), tint = fg)
            Spacer(Modifier.width(10.dp))
        }
        Text(text, color = fg, style = MaterialTheme.typography.labelLarge.copy(fontSize = 16.sp), maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** Round glass icon button (back, settings, ...). */
@Composable
fun GlassIconButton(icon: ImageVector, description: String, onClick: () -> Unit, modifier: Modifier = Modifier, active: Boolean = false) {
    val c = Pill.colors
    Box(
        modifier
            .size(46.dp)
            .clip(CircleShape)
            .background(if (active) c.accent.copy(alpha = 0.28f) else c.surfaceHigh)
            .border(1.dp, if (active) c.accent else c.border, CircleShape)
            .semantics { contentDescription = description }
            .pressable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, Modifier.size(22.dp), tint = if (active) c.accentSoft else c.text) }
}

@Composable
fun TopBar(title: String, onBack: (() -> Unit)?, modifier: Modifier = Modifier, trailing: @Composable RowScope.() -> Unit = {}) {
    Row(
        modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            GlassIconButton(Icons.AutoMirrored.Rounded.ArrowBack, androidx.compose.ui.res.stringResource(app.pill.phone.R.string.back), onBack)
            Spacer(Modifier.width(14.dp))
        } else Spacer(Modifier.width(4.dp))
        Text(title, Modifier.weight(1f), style = MaterialTheme.typography.headlineSmall, color = Pill.colors.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
        trailing()
    }
}

/** The bottom inset for scrolling content (gesture bar). */
@Composable
fun NavBarSpacer() = Spacer(Modifier.navigationBarsPadding().height(24.dp))

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text.uppercase(), modifier.padding(start = 6.dp, top = 8.dp, bottom = 2.dp),
        style = MaterialTheme.typography.labelSmall, color = Pill.colors.textDim,
    )
}

@Composable
fun IconBadge(icon: ImageVector, modifier: Modifier = Modifier, tint: Color = Pill.colors.accentSoft, size: Dp = 44.dp) {
    val c = Pill.colors
    Box(
        modifier.size(size).clip(CircleShape).background(tint.copy(alpha = if (c.dark) 0.14f else 0.12f)),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, Modifier.size(size * 0.5f), tint = if (c.dark) tint else c.accent) }
}

/** A settings-style row inside a card: badge, title, optional subtitle, trailing control. */
@Composable
fun SettingRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    icon: ImageVector? = null,
    iconTint: Color = Pill.colors.accentSoft,
    onClick: (() -> Unit)? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    val c = Pill.colors
    val m = if (onClick != null) modifier.pressable(pressedScale = 0.98f, onClick = onClick, role = Role.Button) else modifier
    Row(m.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        if (icon != null) {
            IconBadge(icon, tint = iconTint)
            Spacer(Modifier.width(14.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall.copy(fontSize = 16.sp), color = c.text)
            if (subtitle != null) {
                Spacer(Modifier.height(2.dp))
                Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = c.textDim)
            }
        }
        if (trailing != null) {
            Spacer(Modifier.width(12.dp))
            trailing()
        }
    }
}

@Composable
fun PillSwitch(checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val c = Pill.colors
    Switch(
        checked, onCheckedChange, modifier,
        colors = SwitchDefaults.colors(
            checkedThumbColor = Color.White,
            checkedTrackColor = c.accent,
            checkedBorderColor = Color.Transparent,
            uncheckedThumbColor = if (c.dark) c.textDim else Color.White,
            uncheckedTrackColor = if (c.dark) c.surfaceHigh else Color(0xFFC9C7D8),
            uncheckedBorderColor = c.border,
        ),
    )
}

@Composable
fun Divider(modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth().height(1.dp).background(Pill.colors.border))
}

/** A small status chip: coloured dot plus text. */
@Composable
fun Chip(text: String, modifier: Modifier = Modifier, icon: ImageVector? = null, dot: Color? = null) {
    val c = Pill.colors
    Row(
        modifier.clip(CircleShape).background(c.surfaceHigh).border(1.dp, c.border, CircleShape).padding(horizontal = 12.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (dot != null) Box(Modifier.size(8.dp).clip(CircleShape).background(dot)).also { Spacer(Modifier.width(8.dp)) }
        if (icon != null) Icon(icon, null, Modifier.size(15.dp), tint = c.textDim).also { Spacer(Modifier.width(6.dp)) }
        Text(text, style = MaterialTheme.typography.labelMedium, color = c.text, maxLines = 1)
    }
}

@Composable
fun SpringProgress(progress: Float, modifier: Modifier = Modifier, height: Dp = 8.dp) {
    val c = Pill.colors
    val p by animateFloatAsState(progress.coerceIn(0f, 1f), Motion.soft(), label = "progress")
    Box(modifier.fillMaxWidth().height(height).clip(CircleShape).background(c.border.copy(alpha = if (c.dark) 0.10f else 0.10f))) {
        Box(
            Modifier.fillMaxSize().graphicsLayer {
                scaleX = p
                transformOrigin = androidx.compose.ui.graphics.TransformOrigin(0f, 0.5f)
            }.clip(CircleShape).background(Brush.horizontalGradient(listOf(c.accentSoft, c.accent))),
        )
    }
}

@Composable
fun EmptyState(icon: ImageVector, title: String, body: String, modifier: Modifier = Modifier, action: @Composable (() -> Unit)? = null) {
    val c = Pill.colors
    Column(modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        IconBadge(icon, size = 64.dp)
        Spacer(Modifier.height(14.dp))
        Text(title, style = MaterialTheme.typography.titleMedium, color = c.text, textAlign = TextAlign.Center)
        Spacer(Modifier.height(6.dp))
        Text(body, style = MaterialTheme.typography.bodyMedium, color = c.textDim, textAlign = TextAlign.Center)
        if (action != null) {
            Spacer(Modifier.height(16.dp))
            action()
        }
    }
}

@Composable
fun PageDots(count: Int, current: Int, modifier: Modifier = Modifier) {
    val c = Pill.colors
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        repeat(count) { i ->
            val on = i == current
            val w by animateDpAsState(if (on) 28.dp else 8.dp, Motion.bouncy(), label = "dotW")
            val col by animateColorAsState(if (on) c.accent else c.textDim.copy(alpha = 0.35f), Motion.calm(), label = "dotC")
            Box(Modifier.height(8.dp).width(w).clip(CircleShape).background(col))
        }
    }
}

/** Tinted, bordered banner for hints and problems inside a screen. */
@Composable
fun Banner(text: String, icon: ImageVector, modifier: Modifier = Modifier, tint: Color = Pill.colors.warn, title: String? = null) {
    val c = Pill.colors
    val shape = RoundedCornerShape(18.dp)
    Row(
        modifier.fillMaxWidth().clip(shape).background(tint.copy(alpha = 0.12f)).border(1.dp, tint.copy(alpha = 0.30f), shape).padding(14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(icon, null, Modifier.size(20.dp), tint = tint)
        Spacer(Modifier.width(12.dp))
        Column {
            if (title != null) {
                Text(title, style = MaterialTheme.typography.titleSmall, color = c.text)
                Spacer(Modifier.height(2.dp))
            }
            Text(text, style = MaterialTheme.typography.bodyMedium, color = if (title != null) c.textDim else c.text)
        }
    }
}
