package app.pill.phone.ui.components

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CloudOff
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.net.Route
import app.pill.phone.ui.theme.LocalReduceMotion
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

private enum class Way { Home, Internet, Offline }

/**
 * How the phone reaches the PC right now: "Home network", "Internet" or "Offline". The dot pulses while the link is alive
 * (steady for Reduce motion, hollow while the internet link waits for the PC); switching routes slides the label and
 * re-tints the pill on a spring.
 */
@Composable
fun RouteBadge(route: Route?, modifier: Modifier = Modifier, waitingForPc: Boolean = false, compact: Boolean = false) {
    val c = Pill.colors
    val reduce = LocalReduceMotion.current
    val way = when {
        route == Route.Lan -> Way.Home
        route == Route.Relay || waitingForPc -> Way.Internet
        else -> Way.Offline
    }
    val alive = route != null
    val tint by animateColorAsState(
        when (way) { Way.Home -> c.good; Way.Internet -> c.accentSoft; Way.Offline -> c.textDim },
        Motion.calm(), label = "routeTint",
    )
    val pulse = if (alive && !reduce) {
        rememberInfiniteTransition(label = "routePulse").animateFloat(
            0f, 1f, infiniteRepeatable(tween(1900, easing = FastOutSlowInEasing), RepeatMode.Restart), label = "pulse",
        ).value
    } else 0f

    Row(
        modifier.animateContentSize(Motion.soft()).clip(CircleShape)
            .background(tint.copy(alpha = 0.12f)).border(1.dp, tint.copy(alpha = 0.35f), CircleShape)
            .padding(horizontal = if (compact) 10.dp else 14.dp, vertical = if (compact) 5.dp else 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(if (compact) 12.dp else 16.dp), contentAlignment = Alignment.Center) {
            if (alive) Canvas(Modifier.matchParentSize()) {
                drawCircle(tint.copy(alpha = (1f - pulse) * 0.5f), radius = size.minDimension / 2f * (0.45f + 0.55f * pulse))
            }
            Box(
                Modifier.size(if (compact) 6.dp else 8.dp).clip(CircleShape).then(
                    if (alive || way == Way.Offline) Modifier.background(tint) else Modifier.border(1.5.dp, tint, CircleShape),
                ),
            )
        }
        Spacer(Modifier.width(if (compact) 6.dp else 8.dp))
        AnimatedContent(
            way,
            transitionSpec = {
                (fadeIn(Motion.calm()) + slideInVertically(Motion.soft()) { it / 2 } + scaleIn(Motion.soft(), initialScale = 0.9f)) togetherWith
                    (fadeOut(Motion.calm()) + slideOutVertically(Motion.calm()) { -it / 2 } + scaleOut(Motion.calm(), targetScale = 0.9f))
            },
            label = "routeLabel",
        ) { w ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    when (w) { Way.Home -> Icons.Rounded.Home; Way.Internet -> Icons.Rounded.Public; Way.Offline -> Icons.Rounded.CloudOff },
                    null, Modifier.size(if (compact) 14.dp else 16.dp), tint = tint,
                )
                Spacer(Modifier.width(if (compact) 4.dp else 6.dp))
                Text(
                    stringResource(when (w) { Way.Home -> R.string.route_home; Way.Internet -> R.string.route_internet; Way.Offline -> R.string.route_offline }),
                    style = if (compact) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge, color = c.text, maxLines = 1,
                )
            }
        }
    }
}
