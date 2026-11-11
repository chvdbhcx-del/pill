package app.pill.phone

import android.app.Application
import app.pill.phone.data.Store
import app.pill.phone.net.LinkManager
import app.pill.phone.protocol.Msg
import app.pill.phone.service.NotifBridge
import app.pill.phone.service.Notifs
import app.pill.phone.service.StatusReporter
import app.pill.phone.transfer.Transfers
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class PillApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Store.init(this)
        LinkManager.init(this)
        Notifs.createChannels(this)
        scope.launch {
            // One place routes every frame from the PC to whoever handles it.
            LinkManager.incoming.collect { (pcId, m) ->
                if (m is Msg.Hello) StatusReporter.onHello(pcId)
                NotifBridge.handle(this@PillApp, pcId, m)
                Transfers.handle(this@PillApp, pcId, m)
            }
        }
    }

    companion object {
        /** Process-wide scope for work that must outlive any screen (transfers, message routing). */
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    }
}
