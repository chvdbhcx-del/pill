package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CameraAlt
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.FlashOff
import androidx.compose.material.icons.rounded.FlashOn
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.NoPhotography
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.GlassIconButton
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.LocalReduceMotion
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

enum class ScanPhase { Scanning, Rejected, Success }

sealed interface ScanUi {
    /** First time: explain before the system dialog. */
    data object Rationale : ScanUi
    data object Denied : ScanUi
    /** The system will not ask again: only the app settings can fix it. */
    data object DeniedForever : ScanUi
    data class Camera(val phase: ScanPhase = ScanPhase.Scanning, val torch: Boolean = false, val hasTorch: Boolean = true) : ScanUi
}

class ScannerActions(
    val onClose: () -> Unit = {},
    val onTorch: () -> Unit = {},
    val onAllow: () -> Unit = {},
    val onOpenSettings: () -> Unit = {},
    val onEnterCode: () -> Unit = {},
)

// The corner colours of the Google scanner the user knows; they stay as they are.
private val BracketBlue = Color(0xFF4285F4)
private val BracketYellow = Color(0xFFFBBC05)
private val BracketGreen = Color(0xFF34A853)
private val BracketRed = Color(0xFFEA4335)

/**
 * Own scanner screen: the app background everywhere, the camera only inside the square window.
 * [preview] draws the camera (a plain colour in screenshot tests); [rejectCount] shakes the brackets once per bump.
 */
@Composable
fun ScannerScreen(ui: ScanUi, actions: ScannerActions, rejectCount: Int = 0, preview: @Composable (Modifier) -> Unit) {
    PillBackground {
        when (ui) {
            is ScanUi.Camera -> CameraLayout(ui, actions, rejectCount, preview)
            else -> PermissionLayout(ui, actions)
        }
        // top bar sits above both layouts
        Row(
            Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlassIconButton(Icons.Rounded.Close, stringResource(R.string.close), actions.onClose)
            Spacer(Modifier.weight(1f))
            if (ui is ScanUi.Camera && ui.hasTorch) {
                GlassIconButton(
                    if (ui.torch) Icons.Rounded.FlashOn else Icons.Rounded.FlashOff,
                    stringResource(R.string.scan_torch), actions.onTorch, active = ui.torch,
                )
            }
        }
    }
}

@Composable
private fun CameraLayout(ui: ScanUi.Camera, a: ScannerActions, rejectCount: Int, preview: @Composable (Modifier) -> Unit) {
    val c = Pill.colors
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val frame = maxWidth * 0.6f          // the square the user knows from the Google scanner
        val window = frame + 8.dp            // the camera window is a touch larger: the brackets sit on its edge
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.height(104.dp).padding(horizontal = 32.dp), contentAlignment = Alignment.BottomCenter) {
                Text(
                    stringResource(R.string.scan_title), Modifier.staggerIn(0),
                    style = MaterialTheme.typography.titleLarge, color = c.text, textAlign = TextAlign.Center,
                )
            }
            Spacer(Modifier.height(24.dp))
            Viewfinder(window, ui.phase, rejectCount, preview)
            Spacer(Modifier.height(24.dp))
            Box(Modifier.height(104.dp).padding(horizontal = 40.dp), contentAlignment = Alignment.TopCenter) {
                AnimatedContent(
                    ui.phase == ScanPhase.Rejected,
                    transitionSpec = { (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.96f)) togetherWith fadeOut(Motion.calm()) },
                    label = "scanHint",
                ) { rejected ->
                    Text(
                        stringResource(if (rejected) R.string.scan_hint_wrong else R.string.scan_hint),
                        style = MaterialTheme.typography.bodyLarge, color = if (rejected) c.warn else c.textDim, textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

@Composable
private fun Viewfinder(window: Dp, phase: ScanPhase, rejectCount: Int, preview: @Composable (Modifier) -> Unit) {
    val c = Pill.colors
    val reduce = LocalReduceMotion.current
    val pad = 14.dp          // room around the window so brackets are never clipped
    val radius = 28.dp

    // brackets spring in when the screen opens
    val settle = remember { Animatable(if (reduce) 1f else 0f) }
    LaunchedEffect(Unit) { if (!reduce) settle.animateTo(1f, spring(dampingRatio = 0.5f, stiffness = 170f)) }

    // gentle shake on a QR that is not ours
    val shake = remember { Animatable(0f) }
    LaunchedEffect(rejectCount) {
        if (rejectCount > 0 && !reduce) {
            for (target in floatArrayOf(18f, -14f, 9f, -5f, 0f)) shake.animateTo(target, spring(dampingRatio = 0.6f, stiffness = 900f))
        }
    }

    val success = phase == ScanPhase.Success
    val pulse by animateFloatAsState(if (success) 1.05f else 1f, Motion.bouncy(), label = "pulse")
    val blue by animateColorAsState(if (success) c.accent else BracketBlue, Motion.calm(), label = "tl")
    val yellow by animateColorAsState(if (success) c.accent else BracketYellow, Motion.calm(), label = "tr")
    val green by animateColorAsState(if (success) c.accent else BracketGreen, Motion.calm(), label = "bl")
    val red by animateColorAsState(if (success) c.accentSoft else BracketRed, Motion.calm(), label = "br")
    val veil by animateFloatAsState(if (success) 0.32f else 0f, Motion.calm(), label = "veil")

    Box(
        Modifier.size(window + pad * 2).graphicsLayer { translationX = shake.value.dp.toPx(); scaleX = pulse; scaleY = pulse },
        contentAlignment = Alignment.Center,
    ) {
        // the only place where the camera is visible
        Box(Modifier.size(window).clip(RoundedCornerShape(radius)).background(c.surface)) {
            preview(Modifier.fillMaxSize())
            if (phase != ScanPhase.Success) ScanLine(c.accentSoft)
            Box(Modifier.fillMaxSize().background(c.accent.copy(alpha = veil)))
        }
        Canvas(Modifier.size(window + pad * 2)) {
            val s = settle.value
            val w = window.toPx()
            val r = radius.toPx()
            val arm = 40.dp.toPx()
            val stroke = Stroke(6.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round)
            val out = (1f - s) * 34.dp.toPx()    // starts wider, springs onto the window edge (overshoots a little)
            val alpha = s.coerceIn(0f, 1f)
            translate(pad.toPx(), pad.toPx()) {
                translate(-out, -out) { drawCorner(Corner.TL, w, r, arm, blue.copy(alpha = alpha), stroke) }
                translate(out, -out) { drawCorner(Corner.TR, w, r, arm, yellow.copy(alpha = alpha), stroke) }
                translate(-out, out) { drawCorner(Corner.BL, w, r, arm, green.copy(alpha = alpha), stroke) }
                translate(out, out) { drawCorner(Corner.BR, w, r, arm, red.copy(alpha = alpha), stroke) }
            }
        }
        androidx.compose.animation.AnimatedVisibility(
            success, enter = fadeIn(Motion.calm()) + scaleIn(Motion.bouncy(), initialScale = 0.3f), exit = fadeOut(Motion.calm()),
        ) {
            Box(Modifier.size(84.dp).clip(CircleShape).background(Brush.linearGradient(listOf(c.accentSoft, c.accent))), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.Check, stringResource(R.string.scan_found), Modifier.size(46.dp), tint = Color.White)
            }
        }
    }
}

private enum class Corner { TL, TR, BL, BR }

/** One bracket: an arm, the rounded corner (same radius as the window) and the other arm. */
private fun DrawScope.drawCorner(corner: Corner, w: Float, r: Float, arm: Float, color: Color, stroke: Stroke) {
    val p = Path()
    when (corner) {
        Corner.TL -> {
            p.moveTo(0f, r + arm); p.lineTo(0f, r)
            p.arcTo(Rect(0f, 0f, 2 * r, 2 * r), 180f, 90f, false); p.lineTo(r + arm, 0f)
        }
        Corner.TR -> {
            p.moveTo(w - r - arm, 0f); p.lineTo(w - r, 0f)
            p.arcTo(Rect(w - 2 * r, 0f, w, 2 * r), 270f, 90f, false); p.lineTo(w, r + arm)
        }
        Corner.BR -> {
            p.moveTo(w, w - r - arm); p.lineTo(w, w - r)
            p.arcTo(Rect(w - 2 * r, w - 2 * r, w, w), 0f, 90f, false); p.lineTo(w - r - arm, w)
        }
        Corner.BL -> {
            p.moveTo(r + arm, w); p.lineTo(r, w)
            p.arcTo(Rect(0f, w - 2 * r, 2 * r, w), 90f, 90f, false); p.lineTo(0f, w - r - arm)
        }
    }
    drawPath(p, color, style = stroke)
}

/** A soft line that sweeps up and down inside the window. */
@Composable
private fun ScanLine(color: Color) {
    if (LocalReduceMotion.current) return
    val t = rememberInfiniteTransition(label = "scanLine")
    val p by t.animateFloat(0f, 1f, infiniteRepeatable(tween(2400, easing = LinearEasing), RepeatMode.Reverse), label = "scanLineP")
    Canvas(Modifier.fillMaxSize()) {
        val y = size.height * (0.08f + 0.84f * p)
        drawRect(Brush.verticalGradient(listOf(Color.Transparent, color.copy(alpha = 0.22f)), startY = y - 56.dp.toPx(), endY = y), Offset(0f, y - 56.dp.toPx()), androidx.compose.ui.geometry.Size(size.width, 56.dp.toPx()))
        drawRect(
            Brush.horizontalGradient(listOf(Color.Transparent, color, Color.Transparent)),
            Offset(size.width * 0.08f, y - 1.5.dp.toPx()), androidx.compose.ui.geometry.Size(size.width * 0.84f, 3.dp.toPx()),
        )
    }
}

@Composable
private fun PermissionLayout(ui: ScanUi, a: ScannerActions) {
    val c = Pill.colors
    Column(
        Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(horizontal = 20.dp),
        verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        GlassCard(Modifier.fillMaxWidth().staggerIn(0), contentPadding = androidx.compose.foundation.layout.PaddingValues(24.dp)) {
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                IconBadge(if (ui is ScanUi.Rationale) Icons.Rounded.CameraAlt else Icons.Rounded.NoPhotography, size = 84.dp, tint = if (ui is ScanUi.Rationale) c.accentSoft else c.warn)
                Spacer(Modifier.height(20.dp))
                Text(
                    stringResource(if (ui is ScanUi.Rationale) R.string.cam_title else R.string.cam_denied_title),
                    style = MaterialTheme.typography.headlineSmall, color = c.text, textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(10.dp))
                Text(
                    stringResource(
                        when (ui) {
                            ScanUi.Rationale -> R.string.cam_body
                            ScanUi.Denied -> R.string.cam_denied_body
                            else -> R.string.cam_forever_body
                        },
                    ),
                    style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(24.dp))
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (ui is ScanUi.DeniedForever) {
                        PillButton(stringResource(R.string.btn_open_settings), a.onOpenSettings, Modifier.fillMaxWidth())
                    } else {
                        PillButton(stringResource(R.string.btn_allow_camera), a.onAllow, Modifier.fillMaxWidth(), icon = Icons.Rounded.CameraAlt)
                    }
                    PillButton(stringResource(R.string.btn_enter_code), a.onEnterCode, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.Keyboard)
                }
            }
        }
    }
}
