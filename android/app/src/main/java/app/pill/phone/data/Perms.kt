package app.pill.phone.data

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.pill.phone.service.PillNotificationListener
import app.pill.phone.ui.PermissionsUi

/** The permission checklist: current state and the one-tap fixes. */
object Perms {
    fun state(ctx: Context): PermissionsUi = PermissionsUi(
        notificationAccess = NotificationManagerCompat.getEnabledListenerPackages(ctx).contains(ctx.packageName),
        postNotifications = Build.VERSION.SDK_INT < 33 || granted(ctx, Manifest.permission.POST_NOTIFICATIONS),
        batteryOptimization = ctx.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(ctx.packageName) == true,
        wifiName = granted(ctx, Manifest.permission.ACCESS_FINE_LOCATION),
        storage = Build.VERSION.SDK_INT >= 29 || granted(ctx, Manifest.permission.WRITE_EXTERNAL_STORAGE),
    )

    private fun granted(ctx: Context, p: String) = ContextCompat.checkSelfPermission(ctx, p) == PackageManager.PERMISSION_GRANTED

    fun notificationAccessIntent(ctx: Context): Intent =
        if (Build.VERSION.SDK_INT >= 30) {
            Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, ComponentName(ctx, PillNotificationListener::class.java).flattenToString())
        } else {
            Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
        }

    fun appDetailsIntent(ctx: Context): Intent =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", ctx.packageName, null))

    @Suppress("BatteryLife") // the whole point of the app is a link that survives the screen being off
    fun batteryIntent(ctx: Context): Intent =
        Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.fromParts("package", ctx.packageName, null))

    fun batteryFallbackIntent(): Intent = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
}
