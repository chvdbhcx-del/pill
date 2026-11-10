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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

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

val DarkPillColors = PillColors(
    dark = true,
    background = Color(0xFF0B0B0F),
    surface = Color(0xFF15151C),
    surfaceHigh = Color(0xFF1D1D27),
    border = Color(0x14FFFFFF),
    text = Color(0xFFF2F2F7),
    textDim = Color(0xFF9A9AA8),
    accent = Color(0xFF8E7BFF),
    accentSoft = Color(0xFFB9AEFF),
    onAccent = Color(0xFF110C33),
    good = Color(0xFF5BE3A4),
    warn = Color(0xFFFFC66B),
    bad = Color(0xFFFF7A8A),
    glow = Color(0xFF8E7BFF),
)

val LightPillColors = PillColors(
    dark = false,
    background = Color(0xFFF4F3FA),
    surface = Color(0xFFFFFFFF),
    surfaceHigh = Color(0xFFEDEBF8),
    border = Color(0x140B0B1F),
    text = Color(0xFF14141C),
    textDim = Color(0xFF6A6A7C),
    accent = Color(0xFF6B57F0),
    accentSoft = Color(0xFF8E7BFF),
    onAccent = Color(0xFFFFFFFF),
    good = Color(0xFF12945F),
    warn = Color(0xFFB9710A),
    bad = Color(0xFFD3394C),
    glow = Color(0xFF8E7BFF),
)

object PillShapes {
    val Card = 24.dp
    val Sheet = 28.dp
    val Row = 20.dp
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

private val Sans = FontFamily.Default

private fun style(size: Int, weight: FontWeight, line: Int, spacing: Float = 0f) =
    TextStyle(fontFamily = Sans, fontWeight = weight, fontSize = size.sp, lineHeight = line.sp, letterSpacing = spacing.sp)

/** One typeface (the system sans), three weights: 500 / 600 / 700. */
val PillTypography = Typography(
    displayMedium = style(34, FontWeight.Bold, 40, -0.4f),
    headlineLarge = style(30, FontWeight.Bold, 36, -0.3f),
    headlineMedium = style(26, FontWeight.Bold, 32, -0.2f),
    headlineSmall = style(22, FontWeight.Bold, 28),
    titleLarge = style(20, FontWeight.SemiBold, 26),
    titleMedium = style(17, FontWeight.SemiBold, 22),
    titleSmall = style(15, FontWeight.SemiBold, 20),
    bodyLarge = style(16, FontWeight.Medium, 24),
    bodyMedium = style(14, FontWeight.Medium, 20),
    bodySmall = style(12, FontWeight.Medium, 16),
    labelLarge = style(15, FontWeight.SemiBold, 20, 0.1f),
    labelMedium = style(13, FontWeight.SemiBold, 16, 0.2f),
    labelSmall = style(11, FontWeight.SemiBold, 14, 0.4f),
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
