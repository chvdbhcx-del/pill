package app.pill.phone

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.ParcelFileDescriptor
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.collectAsState
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import app.pill.phone.data.FileTarget
import app.pill.phone.data.Routing
import app.pill.phone.data.Store
import app.pill.phone.net.LinkManager
import app.pill.phone.transfer.Source
import app.pill.phone.transfer.Transfers
import app.pill.phone.ui.screens.SendSheet
import app.pill.phone.ui.screens.SendUi
import app.pill.phone.ui.theme.PillTheme
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Share-sheet target "Send to PC": ACTION_SEND / ACTION_SEND_MULTIPLE with any mime type (and shared text as a .txt).
 * With one PC paired it just sends; with two or more it always asks which one (or all of them).
 */
class SendActivity : ComponentActivity() {
    private var ui by mutableStateOf<SendUi>(SendUi.Preparing)
    /** The answer to "which PC?": the ids chosen, or null when the user backed out. */
    private var choice: CompletableDeferred<List<String>?>? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            val options by LinkManager.options.collectAsState(emptyList())
            PillTheme {
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
        lifecycleScope.launch { run(intent) }
    }

    private suspend fun run(i: Intent) {
        val pcs = Store.pcs.value
        val target = Routing.fileTarget(pcs, LinkManager.connectedIds())
        if (target is FileTarget.None || (target is FileTarget.Single && !LinkManager.isConnected(target.pcId)) || (target is FileTarget.Ask && !target.anyOnline)) {
            ui = SendUi.NotConnected
            return
        }
        val uris: List<Uri> = when (i.action) {
            Intent.ACTION_SEND_MULTIPLE -> IntentCompat.getParcelableArrayListExtra(i, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> listOfNotNull(IntentCompat.getParcelableExtra(i, Intent.EXTRA_STREAM, Uri::class.java))
        }
        // The share grant ends with this activity, so every file is opened here before the transfer is queued.
        val sources = ArrayList(Transfers.prepare(this, uris))
        if (uris.isEmpty()) i.getCharSequenceExtra(Intent.EXTRA_TEXT)?.let { text -> textSource(text.toString())?.let(sources::add) }
        if (sources.isEmpty()) {
            ui = SendUi.Nothing
            return
        }
        val targets: List<String> = when (target) {
            is FileTarget.Single -> listOf(target.pcId)
            else -> {
                // two or more PCs: always ask
                ui = SendUi.Choose(sources.size)
                val d = CompletableDeferred<List<String>?>().also { choice = it }
                val picked = d.await()
                choice = null
                if (picked.isNullOrEmpty()) {
                    sources.forEach { it.release() }
                    finish()
                    return
                }
                picked
            }
        }
        Transfers.send(this, sources, targets)
        ui = SendUi.Sending(sources.size)
        delay(1600)
        finish()
    }

    private suspend fun textSource(text: String): Source? = withContext(Dispatchers.IO) {
        try {
            val f = File.createTempFile("text", ".txt", cacheDir)
            f.writeText(text)
            val pfd = ParcelFileDescriptor.open(f, ParcelFileDescriptor.MODE_READ_ONLY)
            val size = f.length()
            f.delete()
            Source(getString(R.string.send_text_name), size, "text/plain", pfd)
        } catch (_: Exception) {
            null
        }
    }
}
