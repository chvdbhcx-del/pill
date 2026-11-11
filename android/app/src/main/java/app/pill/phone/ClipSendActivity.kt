package app.pill.phone

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.lifecycleScope
import app.pill.phone.data.ClipSendResult
import app.pill.phone.data.FileTarget
import app.pill.phone.data.Routing
import app.pill.phone.data.Store
import app.pill.phone.net.LinkManager
import app.pill.phone.service.ClipBridge
import app.pill.phone.ui.screens.SendSheet
import app.pill.phone.ui.screens.SendUi
import app.pill.phone.ui.theme.PillTheme
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * "Send clipboard": the Home button, the quick settings tile and the notification action all open this see-through activity.
 * Android 10+ lets only the window that has focus read the clipboard, so the read happens in [onWindowFocusChanged]; then it
 * sends (asking which PC when there are several), shows a short confirmation and finishes.
 */
class ClipSendActivity : ComponentActivity() {
    private var ui by mutableStateOf<SendUi>(SendUi.Preparing)
    private var choice: CompletableDeferred<List<String>?>? = null
    private var started = false

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            val options by LinkManager.options.collectAsState(emptyList())
            PillTheme {
                // nothing at all until there is something to say: a normal send is over in a blink
                if (ui != SendUi.Preparing) {
                    SendSheet(
                        ui,
                        onOpenApp = {
                            startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                            finish()
                        },
                        onClose = { if (choice?.complete(null) != true) finish() },
                        options = options,
                        onPick = { ids -> choice?.complete(ids) },
                    )
                }
            }
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && !started) {
            started = true
            lifecycleScope.launch { run() }
        }
    }

    private suspend fun run() {
        val sync = ClipBridge.sync(this)
        // the read must happen now, while this window has focus
        val text = sync.readClipboard()
        val verdict = sync.check(text)
        if (verdict != null) {
            ui = when (verdict) {
                ClipSendResult.Off -> SendUi.ClipOff
                ClipSendResult.TooLarge -> SendUi.ClipTooLarge
                else -> SendUi.ClipEmpty
            }
            if (verdict != ClipSendResult.Off) { delay(2200); finish() }
            return
        }
        val target = Routing.fileTarget(Store.pcs.value, LinkManager.connectedIds())
        if (target is FileTarget.None || (target is FileTarget.Single && !LinkManager.isConnected(target.pcId)) || (target is FileTarget.Ask && !target.anyOnline)) {
            ui = SendUi.NotConnected
            return
        }
        val targets: List<String> = when (target) {
            is FileTarget.Single -> listOf(target.pcId)
            else -> {
                ui = SendUi.Choose(0)
                val d = CompletableDeferred<List<String>?>().also { choice = it }
                val picked = d.await()
                choice = null
                if (picked.isNullOrEmpty()) { finish(); return }
                picked
            }
        }
        ui = when (sync.send(text, targets, LinkManager::sendTo)) {
            is ClipSendResult.Sent, ClipSendResult.AlreadyThere -> SendUi.ClipSent
            else -> SendUi.NotConnected
        }
        if (ui == SendUi.NotConnected) return
        delay(1400)
        finish()
    }
}
