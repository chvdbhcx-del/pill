package app.pill.phone.service

import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import app.pill.phone.ClipSendActivity
import app.pill.phone.data.Store

/**
 * Quick settings tile "Send clipboard". The clipboard can only be read by a window that has focus, so a tap opens the
 * see-through [ClipSendActivity] (which reads, sends and shows the confirmation) and collapses the panel.
 */
class ClipTileService : TileService() {
    override fun onStartListening() {
        val tile = qsTile ?: return
        tile.state = if (Store.clipEnabled.value) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.updateTile()
    }

    override fun onClick() {
        if (isLocked) unlockAndRun { open() } else open()
    }

    @Suppress("DEPRECATION")
    private fun open() {
        val i = Intent(this, ClipSendActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
        } else {
            startActivityAndCollapse(i)
        }
    }
}
