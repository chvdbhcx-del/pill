package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatterySaver
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.SdStorage
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.pill.phone.R
import app.pill.phone.ui.PermissionsUi
import app.pill.phone.ui.components.Chip
import app.pill.phone.ui.components.Divider
import app.pill.phone.ui.components.DoneTick
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.IconBadge
import app.pill.phone.ui.components.SmallButton
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

class PermissionActions(
    val onNotificationAccess: () -> Unit = {},
    val onPostNotifications: () -> Unit = {},
    val onBattery: () -> Unit = {},
    val onWifiName: () -> Unit = {},
    val onStorage: () -> Unit = {},
)

/** The setup checklist: every item is either a green tick or a one-tap "Allow". */
@Composable
fun PermissionsCard(perms: PermissionsUi, actions: PermissionActions, modifier: Modifier = Modifier) {
    val c = Pill.colors
    GlassCard(modifier.animateContentSize(Motion.soft()), accentBorder = !perms.required) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    stringResource(if (perms.required) R.string.perms_ready_title else R.string.perms_title),
                    style = MaterialTheme.typography.titleMedium, color = c.text,
                )
                Spacer(Modifier.height(2.dp))
                Text(
                    stringResource(if (perms.required) R.string.perms_ready_body else R.string.perms_body),
                    style = MaterialTheme.typography.bodyMedium, color = c.textDim,
                )
            }
            if (!perms.required) {
                Spacer(Modifier.width(10.dp))
                Chip(pluralStringResource(R.plurals.perms_left, perms.missingCount, perms.missingCount), dot = c.warn)
            }
        }
        Spacer(Modifier.height(10.dp))
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            PermRow(Icons.Rounded.NotificationsActive, R.string.perm_access_title, R.string.perm_access_why, perms.notificationAccess, actions.onNotificationAccess, hint = R.string.perm_access_hint)
            Divider(Modifier.padding(start = 58.dp))
            PermRow(Icons.Rounded.Notifications, R.string.perm_post_title, R.string.perm_post_why, perms.postNotifications, actions.onPostNotifications)
            Divider(Modifier.padding(start = 58.dp))
            PermRow(Icons.Rounded.BatterySaver, R.string.perm_battery_title, R.string.perm_battery_why, perms.batteryOptimization, actions.onBattery)
            Divider(Modifier.padding(start = 58.dp))
            PermRow(Icons.Rounded.Wifi, R.string.perm_wifi_title, R.string.perm_wifi_why, perms.wifiName, actions.onWifiName)
            if (!perms.storage) {
                Divider(Modifier.padding(start = 58.dp))
                PermRow(Icons.Rounded.SdStorage, R.string.perm_storage_title, R.string.perm_storage_why, false, actions.onStorage)
            }
        }
    }
}

@Composable
private fun PermRow(icon: ImageVector, title: Int, why: Int, granted: Boolean, onFix: () -> Unit, hint: Int? = null) {
    val c = Pill.colors
    Column(Modifier.fillMaxWidth().padding(vertical = 10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBadge(icon, tint = if (granted) c.good else c.accentSoft)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(stringResource(title), style = MaterialTheme.typography.titleSmall, color = c.text)
                Spacer(Modifier.height(1.dp))
                Text(stringResource(why), style = MaterialTheme.typography.bodySmall, color = c.textDim)
            }
            Spacer(Modifier.width(10.dp))
            AnimatedContent(
                granted,
                transitionSpec = { (fadeIn(Motion.calm()) + scaleIn(Motion.bouncy(), initialScale = 0.4f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.6f)) },
                label = "perm",
            ) { ok ->
                if (ok) DoneTick() else SmallButton(stringResource(R.string.btn_allow), onFix)
            }
        }
        if (hint != null && !granted) {
            Spacer(Modifier.height(8.dp))
            Text(stringResource(hint), Modifier.padding(start = 58.dp), style = MaterialTheme.typography.bodySmall, color = c.textDim)
        }
    }
}
