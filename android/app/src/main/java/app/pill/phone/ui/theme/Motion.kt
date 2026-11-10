package app.pill.phone.ui.theme

import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize

/**
 * The "liquid" feel of the desktop island: everything moves on a spring with a soft overshoot.
 * One place, so the whole app has the same physics.
 */
object Motion {
    /** Gentle overshoot: layout, positions, sizes. */
    fun <T> soft(): FiniteAnimationSpec<T> = spring(dampingRatio = 0.68f, stiffness = Spring.StiffnessMediumLow)

    /** Livelier overshoot: presses, toggles, small emphasis. */
    fun <T> bouncy(): FiniteAnimationSpec<T> = spring(dampingRatio = 0.52f, stiffness = 420f)

    /** No overshoot: fades and colours (overshooting alpha looks broken). */
    fun <T> calm(): FiniteAnimationSpec<T> = spring(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = Spring.StiffnessMedium)

    val enterItem: EnterTransition =
        fadeIn(calm()) + scaleIn(soft(), initialScale = 0.92f) + expandVertically(soft())
    val exitItem: ExitTransition =
        fadeOut(calm()) + scaleOut(calm(), targetScale = 0.94f) + shrinkVertically(calm())

    val enterRise: EnterTransition =
        fadeIn(calm()) + slideInVertically(soft<IntOffset>()) { it / 6 } + scaleIn(soft(), initialScale = 0.96f)
    val exitSink: ExitTransition =
        fadeOut(calm()) + slideOutVertically(calm<IntOffset>()) { it / 10 }

    @Suppress("unused")
    val sizeSpec: FiniteAnimationSpec<IntSize> = soft()
}
