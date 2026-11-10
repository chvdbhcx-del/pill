package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerDefaults
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.PageDots
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.PillMark
import app.pill.phone.ui.theme.LocalReduceMotion
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill
import app.pill.phone.ui.theme.PillColors
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.sin

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun OnboardingScreen(onFinish: () -> Unit, initialPage: Int = 0) {
    val c = Pill.colors
    val pager = rememberPagerState(initialPage = initialPage, pageCount = { 3 })
    val scope = rememberCoroutineScope()
    val last = pager.currentPage == 2
    PillBackground {
        Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                PillMark(size = 36.dp)
                Spacer(Modifier.weight(1f))
                AnimatedContent(last, transitionSpec = { fadeIn(Motion.calm()) togetherWith fadeOut(Motion.calm()) }, label = "skip") { l ->
                    if (!l) PillButton(stringResource(R.string.btn_skip), onFinish, Modifier.height(44.dp), kind = ButtonKind.Quiet)
                    else Spacer(Modifier.height(44.dp))
                }
            }
            HorizontalPager(
                pager, Modifier.weight(1f),
                flingBehavior = PagerDefaults.flingBehavior(pager, snapAnimationSpec = Motion.soft()),
            ) { page ->
                val offset = (pager.currentPage - page) + pager.currentPageOffsetFraction
                OnboardingPage(page, offset)
            }
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 20.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                PageDots(3, pager.currentPage)
                PillButton(
                    stringResource(if (last) R.string.btn_get_started else R.string.btn_next),
                    { if (last) onFinish() else scope.launch { pager.animateScrollToPage(pager.currentPage + 1, animationSpec = Motion.soft()) } },
                )
            }
        }
    }
}

@Composable
private fun OnboardingPage(page: Int, offset: Float) {
    val c = Pill.colors
    val titles = intArrayOf(R.string.ob1_title, R.string.ob2_title, R.string.ob3_title)
    val bodies = intArrayOf(R.string.ob1_body, R.string.ob2_body, R.string.ob3_body)
    Column(
        Modifier.fillMaxSize().padding(horizontal = 28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier.weight(1f).fillMaxWidth().graphicsLayer {
                translationX = offset * size.width * 0.35f
                alpha = (1f - abs(offset) * 1.2f).coerceIn(0f, 1f)
                val s = 1f - 0.18f * abs(offset).coerceAtMost(1f)
                scaleX = s; scaleY = s
            },
            contentAlignment = Alignment.Center,
        ) {
            when (page) {
                0 -> NotifFlowArt()
                1 -> ScanArt()
                else -> PermissionsArt()
            }
        }
        Column(
            Modifier.graphicsLayer {
                translationX = offset * size.width * 0.7f
                alpha = (1f - abs(offset) * 1.6f).coerceIn(0f, 1f)
            },
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(stringResource(titles[page]), style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
            Spacer(Modifier.height(12.dp))
            Text(stringResource(bodies[page]), style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
            Spacer(Modifier.height(14.dp))
        }
    }
}

/** 0..1 looping clock for the illustrations; frozen at [still] when motion is reduced. */
@Composable
private fun loop(durationMs: Int, still: Float = 0.5f): State<Float> {
    if (LocalReduceMotion.current) return remember { mutableFloatStateOf(still) }
    val t = rememberInfiniteTransition(label = "art")
    return t.animateFloat(0f, 1f, infiniteRepeatable(tween(durationMs, easing = LinearEasing), RepeatMode.Restart), label = "artT")
}

private fun DrawScope.glass(c: PillColors, topLeft: Offset, size: Size, radius: Float, alpha: Float = 1f) {
    drawRoundRect(c.surfaceHigh.copy(alpha = alpha), topLeft, size, CornerRadius(radius))
    drawRoundRect(c.border.copy(alpha = c.border.alpha * alpha), topLeft, size, CornerRadius(radius), style = Stroke(1.dp.toPx()))
}

/** Page 1: notifications drift from the phone to the PC. */
@Composable
private fun NotifFlowArt() {
    val c = Pill.colors
    val t by loop(4200, still = 0.42f)
    Box(contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(330.dp, 280.dp)) {
            val w = size.width; val h = size.height
            drawCircle(Brush.radialGradient(listOf(c.accent.copy(alpha = 0.28f), Color.Transparent), center = center, radius = w * 0.55f), w * 0.55f, center)
            // phone (left)
            val pw = w * 0.2f; val ph = h * 0.52f
            glass(c, Offset(w * 0.04f, h * 0.5f - ph / 2), Size(pw, ph), 14.dp.toPx())
            drawRoundRect(c.textDim.copy(alpha = 0.5f), Offset(w * 0.04f + pw * 0.3f, h * 0.5f - ph / 2 + 8.dp.toPx()), Size(pw * 0.4f, 3.dp.toPx()), CornerRadius(2.dp.toPx()))
            // PC (right)
            val mw = w * 0.38f; val mh = h * 0.42f
            val mx = w - mw - w * 0.03f; val my = h * 0.5f - mh / 2 - 8.dp.toPx()
            glass(c, Offset(mx, my), Size(mw, mh), 14.dp.toPx())
            drawRoundRect(c.textDim.copy(alpha = 0.4f), Offset(mx + mw * 0.38f, my + mh + 8.dp.toPx()), Size(mw * 0.24f, 5.dp.toPx()), CornerRadius(3.dp.toPx()))
            // toast that lands on the PC
            val landed = ((t - 0.55f) / 0.25f).coerceIn(0f, 1f) * (1f - ((t - 0.9f) / 0.1f).coerceIn(0f, 1f))
            val tw = mw * 0.72f
            drawRoundRect(c.accent.copy(alpha = 0.25f + 0.55f * landed), Offset(mx + mw * 0.14f, my + 12.dp.toPx() - 8.dp.toPx() * (1 - landed)), Size(tw, 22.dp.toPx()), CornerRadius(11.dp.toPx()))
            // three bubbles on the way
            for (i in 0..2) {
                val local = ((t + i / 3f) % 1f)
                val x0 = w * 0.04f + pw; val x1 = mx
                val x = x0 + (x1 - x0) * local
                val y = h * 0.5f + sin(local * 6.283f * 1.2f + i) * 16.dp.toPx() - 6.dp.toPx()
                val a = sin(local * 3.1416f).coerceIn(0f, 1f)
                val bw = 40.dp.toPx(); val bh = 20.dp.toPx()
                drawRoundRect(
                    Brush.horizontalGradient(listOf(c.accentSoft.copy(alpha = a), c.accent.copy(alpha = a)), startX = x, endX = x + bw),
                    Offset(x - bw / 2, y - bh / 2), Size(bw, bh), CornerRadius(bh / 2),
                )
                drawCircle(Color.White.copy(alpha = 0.8f * a), 3.dp.toPx(), Offset(x - bw / 2 + 9.dp.toPx(), y))
            }
        }
    }
}

/** Page 2: a QR code with a scan line. */
@Composable
private fun ScanArt() {
    val c = Pill.colors
    val t by loop(2600, still = 0.35f)
    Canvas(Modifier.size(250.dp)) {
        val s = size.minDimension
        drawCircle(Brush.radialGradient(listOf(c.accent.copy(alpha = 0.26f), Color.Transparent), center = center, radius = s * 0.7f), s * 0.7f, center)
        val card = s * 0.74f
        val tl = Offset((s - card) / 2, (s - card) / 2)
        glass(c, tl, Size(card, card), 26.dp.toPx())
        val n = 9
        val pad = card * 0.12f
        val cell = (card - 2 * pad) / n
        val ink = if (c.dark) c.text else Color(0xFF14141C)
        // deterministic pseudo-QR
        for (y in 0 until n) for (x in 0 until n) {
            val finder = (x < 3 && y < 3) || (x >= n - 3 && y < 3) || (x < 3 && y >= n - 3)
            val on = if (finder) false else ((x * 7 + y * 13 + x * y) % 3 != 0)
            if (on) drawRoundRect(ink.copy(alpha = 0.85f), Offset(tl.x + pad + x * cell + 1f, tl.y + pad + y * cell + 1f), Size(cell - 2f, cell - 2f), CornerRadius(cell * 0.28f))
        }
        for ((fx, fy) in listOf(0 to 0, n - 3 to 0, 0 to n - 3)) {
            val o = Offset(tl.x + pad + fx * cell, tl.y + pad + fy * cell)
            drawRoundRect(c.accent, o, Size(cell * 3, cell * 3), CornerRadius(cell * 0.8f))
            drawRoundRect(c.surfaceHigh, o + Offset(cell * 0.55f, cell * 0.55f), Size(cell * 1.9f, cell * 1.9f), CornerRadius(cell * 0.5f))
            drawRoundRect(c.accentSoft, o + Offset(cell * 1f, cell * 1f), Size(cell, cell), CornerRadius(cell * 0.3f))
        }
        // corner brackets
        val b = 26.dp.toPx(); val sw = 4.dp.toPx(); val m = card * 0.04f
        fun bracket(x: Float, y: Float, dx: Float, dy: Float) {
            drawLine(c.accentSoft, Offset(x, y), Offset(x + dx * b, y), sw, androidx.compose.ui.graphics.StrokeCap.Round)
            drawLine(c.accentSoft, Offset(x, y), Offset(x, y + dy * b), sw, androidx.compose.ui.graphics.StrokeCap.Round)
        }
        bracket(tl.x - m, tl.y - m, 1f, 1f); bracket(tl.x + card + m, tl.y - m, -1f, 1f)
        bracket(tl.x - m, tl.y + card + m, 1f, -1f); bracket(tl.x + card + m, tl.y + card + m, -1f, -1f)
        // scan line (ping-pong)
        val p = if (t < 0.5f) t * 2 else (1f - t) * 2
        val y = tl.y + card * (0.06f + 0.88f * p)
        drawRoundRect(
            Brush.horizontalGradient(listOf(Color.Transparent, c.accentSoft, c.accent, Color.Transparent), startX = tl.x, endX = tl.x + card),
            Offset(tl.x, y - 2.dp.toPx()), Size(card, 4.dp.toPx()), CornerRadius(2.dp.toPx()),
        )
        drawRect(Brush.verticalGradient(listOf(Color.Transparent, c.accent.copy(alpha = 0.16f)), startY = y - 40.dp.toPx(), endY = y), Offset(tl.x + 6.dp.toPx(), y - 40.dp.toPx()), Size(card - 12.dp.toPx(), 40.dp.toPx()))
    }
}

/** Page 3: three permission toggles that switch on one after another. */
@Composable
private fun PermissionsArt() {
    val c = Pill.colors
    val t by loop(4800, still = 0.95f)
    Canvas(Modifier.size(300.dp, 250.dp)) {
        val w = size.width; val h = size.height
        drawCircle(Brush.radialGradient(listOf(c.accent.copy(alpha = 0.22f), Color.Transparent), center = center, radius = w * 0.55f), w * 0.55f, center)
        val rowH = 56.dp.toPx(); val gap = 14.dp.toPx()
        val total = 3 * rowH + 2 * gap
        val top = (h - total) / 2
        for (i in 0..2) {
            val y = top + i * (rowH + gap)
            val on = ((t * 1.25f - (0.12f + i * 0.2f)) / 0.12f).coerceIn(0f, 1f)
            glass(c, Offset(0f, y), Size(w, rowH), 20.dp.toPx())
            drawCircle(c.accent.copy(alpha = 0.18f), 14.dp.toPx(), Offset(30.dp.toPx(), y + rowH / 2))
            drawCircle(c.accentSoft, 5.dp.toPx(), Offset(30.dp.toPx(), y + rowH / 2))
            drawRoundRect(c.textDim.copy(alpha = 0.55f), Offset(58.dp.toPx(), y + rowH / 2 - 9.dp.toPx()), Size(w * (0.36f - i * 0.04f), 6.dp.toPx()), CornerRadius(3.dp.toPx()))
            drawRoundRect(c.textDim.copy(alpha = 0.28f), Offset(58.dp.toPx(), y + rowH / 2 + 3.dp.toPx()), Size(w * (0.24f + i * 0.03f), 5.dp.toPx()), CornerRadius(3.dp.toPx()))
            // switch
            val sw = 50.dp.toPx(); val sh = 28.dp.toPx()
            val sx = w - sw - 18.dp.toPx(); val sy = y + (rowH - sh) / 2
            drawRoundRect(
                androidx.compose.ui.graphics.lerp(c.textDim.copy(alpha = 0.3f), c.accent, on),
                Offset(sx, sy), Size(sw, sh), CornerRadius(sh / 2),
            )
            val knobX = sx + sh / 2 + (sw - sh) * on
            drawCircle(Color.White, sh / 2 - 4.dp.toPx(), Offset(knobX, sy + sh / 2))
        }
    }
}
