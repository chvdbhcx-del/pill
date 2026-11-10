package app.pill.phone.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Restores the link after a reboot or an app update. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            LinkService.start(context)
        }
    }
}
