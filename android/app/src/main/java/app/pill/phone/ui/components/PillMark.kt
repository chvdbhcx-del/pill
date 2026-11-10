package app.pill.phone.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.pill.phone.ui.theme.Pill

/**
 * The logo: a tilted pill capsule (two halves) with a droplet that has just left it.
 * Geometry is in a 108-unit square, the same numbers as res/drawable/ic_launcher_foreground.xml.
 */
@Composable
fun PillMark(modifier: Modifier = Modifier, size: Dp = 48.dp, dim: Float = 1f, dropletLift: Float = 0f, zoom: Float = 1.5f, mono: Boolean = false) {
    val c = Pill.colors
    Canvas(modifier.size(size)) {
        val k = this.size.minDimension / 108f * zoom
        val shift = (108f * (1f - zoom) / 2f) * (this.size.minDimension / 108f)
        val light = if (mono) Color.White.copy(alpha = 0.97f * dim) else c.accentSoft.copy(alpha = dim)
        val mid = if (mono) Color.White.copy(alpha = 0.82f * dim) else c.accent.copy(alpha = dim)
        val deep = if (mono) Color.White.copy(alpha = 0.52f * dim) else Color(0xFF5B47DB).copy(alpha = dim)
        withTransform({
            translate(shift, shift)
            scale(k, k, pivot = Offset.Zero)
            rotate(-40f, pivot = Offset(54f, 54f))
        }) {
            val corner = CornerRadius(14f, 14f)
            val capTopLeft = Offset(40f, 26f)
            val capSize = Size(28f, 56f)
            // upper half: bright
            clipRect(top = 26f, bottom = 54f) {
                drawRoundRect(
                    Brush.verticalGradient(listOf(light, mid), startY = 26f, endY = 54f),
                    capTopLeft, capSize, corner,
                )
            }
            // lower half: deeper violet, like the other half of a capsule
            clipRect(top = 54f, bottom = 82f) {
                drawRoundRect(
                    Brush.verticalGradient(listOf(mid, deep), startY = 54f, endY = 82f),
                    capTopLeft, capSize, corner,
                )
            }
            // seam and gloss
            drawLine(Color.White.copy(alpha = 0.28f * dim), Offset(40f, 54f), Offset(68f, 54f), strokeWidth = 1.2f)
            drawRoundRect(
                Color.White.copy(alpha = 0.5f * dim), Offset(44.5f, 32f), Size(3.6f, 15f), CornerRadius(1.8f, 1.8f),
            )
        }
        // the droplet (outside the tilt so it can float on its own)
        withTransform({
            translate(shift, shift)
            scale(k, k, pivot = Offset.Zero)
        }) {
            val dc = Offset(77f, 31f - dropletLift)
            drawCircle(Brush.radialGradient(listOf(light, mid), center = dc - Offset(1.5f, 1.5f), radius = 7f), 4.8f, dc)
            drawCircle(Color.White.copy(alpha = 0.55f * dim), 1.2f, dc - Offset(1.6f, 1.6f))
        }
    }
}
