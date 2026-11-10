package app.pill.phone.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.pill.phone.transfer.Transfers

/** The "Receive" button / tap on the "File from PC" notification. */
class TransferActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == ACTION_RECEIVE) intent.getStringExtra(EXTRA_ID)?.let { Transfers.accept(context, it) }
    }

    companion object {
        const val ACTION_RECEIVE = "app.pill.phone.RECEIVE"
        const val EXTRA_ID = "id"
    }
}
