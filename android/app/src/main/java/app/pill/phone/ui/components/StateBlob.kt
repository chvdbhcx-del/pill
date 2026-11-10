package app.pill.phone.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.pill.phone.ui.theme.LocalReduceMotion
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

enum class BlobState { Idle, Connecting, Connected, Offline, Error }

/**
 * The connection state as a living drop: it breathes (slow swell and ripples) while connected,
 * wobbles (fast, lumpy outline and an orbiting bead) while connecting, sleeps when idle,
 * goes flat and grey when offline and warm red on an error.
 */
@Composable
fun StateBlob(state: BlobState, modifier: Modifier = Modifier, size: Dp = 200.dp, frozenPhase: Float? = null) {
    val c = Pill.colors
    val reduce = LocalReduceMotion.current

    val wobble by animateFloatAsState(
        when (state) { BlobState.Connecting -> 0.085f; BlobState.Connected -> 0.03f; BlobState.Idle -> 0.04f; BlobState.Error -> 0.02f; BlobState.Offline -> 0f },
        Motion.soft(), label = "wobble",
    )
    val speed by animateFloatAsState(
        when (state) { BlobState.Connecting -> 3.2f; BlobState.Connected -> 0.9f; BlobState.Idle -> 0.5f; else -> 0.35f },
        Motion.soft(), label = "speed",
    )
    val body by animateColorAsState(
        when (state) { BlobState.Offline -> c.textDim.copy(alpha = 0.5f); BlobState.Error -> c.bad; else -> c.accent },
        Motion.calm(), label = "body",
    )
    val bodyLight by animateColorAsState(
        when (state) { BlobState.Offline -> c.textDim.copy(alpha = 0.75f); BlobState.Error -> c.bad.copy(alpha = 0.7f).compositeOver(Color.White); else -> c.accentSoft },
        Motion.calm(), label = "bodyLight",
    )
    val glowAlpha by animateFloatAsState(
        when (state) { BlobState.Connected -> 0.55f; BlobState.Connecting -> 0.45f; BlobState.Idle -> 0.28f; BlobState.Error -> 0.4f; BlobState.Offline -> 0.08f },
        Motion.soft(), label = "glow",
    )

    var phase by remember { mutableFloatStateOf(frozenPhase ?: 0f) }
    val liveSpeed by rememberUpdatedState(speed)
    if (!reduce && frozenPhase == null) {
        LaunchedEffect(Unit) {
            var last = 0L
            while (true) {
                withFrameNanos { now ->
                    if (last != 0L) phase += (now - last) / 1_000_000_000f * liveSpeed
                    last = now
                }
            }
        }
    }

    Box(modifier.size(size), contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(size)) {
            val r0 = this.size.minDimension * 0.5f * 0.56f
            val center = Offset(this.size.width / 2, this.size.height / 2)
            val breathe = if (state == BlobState.Connected) 1f + 0.035f * sin(phase * 1.4f) else 1f
            val r = r0 * breathe

            // ambient glow
            drawCircle(
                Brush.radialGradient(
                    listOf(body.copy(alpha = glowAlpha), body.copy(alpha = glowAlpha * 0.35f), Color.Transparent),
                    center = center, radius = this.size.minDimension * 0.5f,
                ),
                this.size.minDimension * 0.5f, center,
            )

            // ripples while connected
            if (state == BlobState.Connected) {
                for (i in 0..1) {
                    val f = (((phase * 0.22f) + i * 0.5f) % 1f)
                    drawCircle(body.copy(alpha = (1f - f) * 0.28f), r * (1.04f + 0.42f * f), center, style = Stroke(width = 1.5.dp.toPx()))
                }
            }

            // the drop: a circle with a lumpy, moving outline
            val path = Path()
            val n = 72
            for (i in 0..n) {
                val a = (i % n) * 2f * PI.toFloat() / n
                val lump = 1f + wobble * (0.6f * sin(2f * a + phase * 1.3f) + 0.4f * sin(3f * a - phase * 1.9f) + 0.25f * sin(5f * a + phase * 2.6f))
                val x = center.x + cos(a) * r * lump
                val y = center.y + sin(a) * r * lump
                if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
            }
            path.close()
            drawPath(
                path,
                Brush.linearGradient(listOf(bodyLight, body, body.copy(alpha = 0.78f)), start = center - Offset(r, r), end = center + Offset(r, r)),
            )
            drawPath(path, Color.White.copy(alpha = 0.22f), style = Stroke(width = 1.dp.toPx()))
            // gloss
            drawCircle(
                Brush.radialGradient(listOf(Color.White.copy(alpha = 0.32f), Color.Transparent), center = center - Offset(r * 0.38f, r * 0.45f), radius = r * 0.7f),
                r * 0.7f, center - Offset(r * 0.38f, r * 0.45f),
            )

            // a bead orbiting the drop while connecting
            if (state == BlobState.Connecting) {
                val a = phase * 1.6f
                val o = Offset(center.x + cos(a) * r * 1.38f, center.y + sin(a) * r * 1.38f)
                drawCircle(Brush.radialGradient(listOf(bodyLight, Color.Transparent), center = o, radius = r * 0.2f), r * 0.2f, o)
                drawCircle(Color.White.copy(alpha = 0.9f), r * 0.055f, o)
            }
        }
        val dim = if (state == BlobState.Offline) 0.55f else 1f
        val lift = if (state == BlobState.Connected) 1.2f * (1f + sin(phase * 1.4f)) else 0f
        PillMark(size = size * 0.5f, dim = dim, dropletLift = lift, mono = true)
    }
}
