package app.pill.phone.ui.theme

import android.provider.Settings
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.LineBreak
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.R

/** Design tokens shared with the desktop Pill (docs/phone-protocol.md, "App design notes"). */
@Immutable
data class PillColors(
    val dark: Boolean,
    val background: Color,
    val surface: Color,
    val surfaceHigh: Color,
    val border: Color,
    val text: Color,
    val textDim: Color,
    val accent: Color,
    val accentSoft: Color,
    val onAccent: Color,
    val good: Color,
    val warn: Color,
    val bad: Color,
    val glow: Color,
)

// The colours of the desktop Pill itself (glasskit `Theme::pill_dark` / `pill_light`, `theme.css`): a pure black body with neutral grey cards,
// white text and a white accent (near black in the light theme); green / amber / red are the island's own status colours. No violet tint anywhere.
val DarkPillColors = PillColors(
    dark = true,
    background = Color(0xFF000000),
    surface = Color(0xFF0F0F0F),
    surfaceHigh = Color(0xFF1C1C1C),
    border = Color(0x1AFFFFFF),
    text = Color(0xFFFFFFFF),
    textDim = Color(0xFFABABAB),
    accent = Color(0xFFFFFFFF),
    accentSoft = Color(0xFFD9D9D9),
    onAccent = Color(0xFF0B0B0B),
    good = Color(0xFF58DDAB),
    warn = Color(0xFFFFB340),
    bad = Color(0xFFFF453A),
    glow = Color(0xFFFFFFFF),
)

val LightPillColors = PillColors(
    dark = false,
    background = Color(0xFFF6F6F6),
    surface = Color(0xFFE9E9E9),
    surfaceHigh = Color(0xFFDEDEDE),
    border = Color(0x1F000000),
    text = Color(0xFF161616),
    textDim = Color(0xFF555555),
    accent = Color(0xFF161616),
    accentSoft = Color(0xFF4A4A4A),
    onAccent = Color(0xFFFFFFFF),
    good = Color(0xFF11805A),
    warn = Color(0xFFB26A00),
    bad = Color(0xFFD92D20),
    glow = Color(0xFF161616),
)

object PillShapes {
    /** Every card, grouped list and quick-action tile. */
    val Card = 22.dp
    val Sheet = 28.dp
    val Row = 22.dp
}

object PillSpacing {
    val Page = 20.dp
    val Gap = 12.dp
    val Section = 24.dp
}

val LocalPillColors = compositionLocalOf { DarkPillColors }

/** True when the user (or a screenshot test) asked for no looping / decorative motion. */
val LocalReduceMotion = compositionLocalOf { false }

object Pill {
    val colors: PillColors
        @Composable @ReadOnlyComposable get() = LocalPillColors.current
}

/** Inter (bundled, OFL, Latin + Cyrillic): the same face on every phone, with real 400 / 600 / 700 weights. */
private val Sans = FontFamily(
    Font(R.font.inter_regular, FontWeight.Normal),
    Font(R.font.inter_semibold, FontWeight.SemiBold),
    Font(R.font.inter_bold, FontWeight.Bold),
)

/** `breaks`: headings are wrapped balanced (no lone last word), running text with the high-quality paragraph breaker (no short last lines). */
private fun style(size: Int, weight: FontWeight, line: Int, spacing: Float = 0f, breaks: LineBreak = LineBreak.Simple) =
    TextStyle(fontFamily = Sans, fontWeight = weight, fontSize = size.sp, lineHeight = line.sp, letterSpacing = spacing.sp, lineBreak = breaks)

/**
 * One typeface, three weights, a clear ladder: headlines 700 and tight, titles and labels 600, running text 400 (it was all 500, so nothing stood out).
 * Inter wants tighter tracking the bigger it gets and a touch of air when it is small; line heights run about 1.4 for text, 1.2 for headlines.
 */
val PillTypography = Typography(
    displayMedium = style(34, FontWeight.Bold, 40, -0.8f, breaks = LineBreak.Heading),
    headlineLarge = style(30, FontWeight.Bold, 36, -0.7f, breaks = LineBreak.Heading),
    headlineMedium = style(26, FontWeight.Bold, 32, -0.5f, breaks = LineBreak.Heading),
    headlineSmall = style(22, FontWeight.Bold, 28, -0.35f, breaks = LineBreak.Heading),
    titleLarge = style(20, FontWeight.SemiBold, 26, -0.25f, breaks = LineBreak.Heading),
    titleMedium = style(17, FontWeight.SemiBold, 23, -0.15f, breaks = LineBreak.Heading),
    titleSmall = style(15, FontWeight.SemiBold, 21, -0.1f, breaks = LineBreak.Heading),
    bodyLarge = style(16, FontWeight.Normal, 24, -0.05f, breaks = LineBreak.Paragraph),
    bodyMedium = style(14, FontWeight.Normal, 21, breaks = LineBreak.Paragraph),
    bodySmall = style(12, FontWeight.Normal, 17, 0.05f, breaks = LineBreak.Paragraph),
    labelLarge = style(15, FontWeight.SemiBold, 20, -0.05f),
    labelMedium = style(13, FontWeight.SemiBold, 17, 0.05f),
    labelSmall = style(11, FontWeight.SemiBold, 14, 0.2f),
)

private fun PillColors.toMaterial(): ColorScheme =
    if (dark) darkColorScheme(
        primary = accent, onPrimary = onAccent, primaryContainer = surfaceHigh, onPrimaryContainer = text,
        secondary = accentSoft, onSecondary = onAccent, background = background, onBackground = text,
        surface = surface, onSurface = text, surfaceVariant = surfaceHigh, onSurfaceVariant = textDim,
        outline = border, outlineVariant = border, error = bad, onError = onAccent,
        surfaceContainer = surface, surfaceContainerHigh = surfaceHigh, surfaceContainerHighest = surfaceHigh,
    ) else lightColorScheme(
        primary = accent, onPrimary = onAccent, primaryContainer = surfaceHigh, onPrimaryContainer = text,
        secondary = accentSoft, onSecondary = onAccent, background = background, onBackground = text,
        surface = surface, onSurface = text, surfaceVariant = surfaceHigh, onSurfaceVariant = textDim,
        outline = border, outlineVariant = border, error = bad, onError = onAccent,
        surfaceContainer = surface, surfaceContainerHigh = surfaceHigh, surfaceContainerHighest = surfaceHigh,
    )

@Composable
fun PillTheme(dark: Boolean = isSystemInDarkTheme(), reduceMotion: Boolean? = null, content: @Composable () -> Unit) {
    val colors = if (dark) DarkPillColors else LightPillColors
    val ctx = LocalContext.current
    val reduce = reduceMotion ?: remember {
        try {
            Settings.Global.getFloat(ctx.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
        } catch (_: Exception) {
            false
        }
    }
    CompositionLocalProvider(LocalPillColors provides colors, LocalReduceMotion provides reduce) {
        MaterialTheme(colorScheme = colors.toMaterial(), typography = PillTypography, content = content)
    }
}
