package app.pill.phone.service

import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import app.pill.phone.PillApp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/** The system hands us every notification here; [NotifBridge] decides what goes to the PC. */
class PillNotificationListener : NotificationListenerService() {

    override fun onListenerConnected() {
        instance = this
        LinkService.start(this)
        // The link may already be up (service started first): send what is currently shown.
        PillApp.scope.launch(Dispatchers.Default) { NotifBridge.replayAll(applicationContext) }
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        PillApp.scope.launch(Dispatchers.Default) { NotifBridge.posted(applicationContext, sbn, replay = false) }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        NotifBridge.removed(sbn.key)
    }

    fun cancelNotificationSafe(key: String) {
        try { cancelNotification(key) } catch (_: Exception) { }
    }

    companion object {
        @Volatile var instance: PillNotificationListener? = null
    }
}
