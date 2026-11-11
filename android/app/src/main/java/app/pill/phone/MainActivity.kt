package app.pill.phone

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.lifecycle.lifecycleScope
import app.pill.phone.data.Perms
import app.pill.phone.service.ClipBridge
import app.pill.phone.service.LinkService
import app.pill.phone.transfer.Transfers
import app.pill.phone.ui.HostActions
import app.pill.phone.ui.PillRoot
import app.pill.phone.ui.screens.PermissionActions
import app.pill.phone.ui.theme.PillTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val vm: MainViewModel by viewModels()

    private val postNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) { vm.refreshPerms() }
    private val location = registerForActivityResult(ActivityResultContracts.RequestPermission()) { vm.refreshPerms() }
    private val storage = registerForActivityResult(ActivityResultContracts.RequestPermission()) { vm.refreshPerms() }
    private val picker = registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) lifecycleScope.launch {
            // Open the files while the picker's grant is valid, then let the view model send them (asking which PC when there are several).
            vm.onFilesPicked(Transfers.prepare(this@MainActivity, uris))
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handle(intent)
        setContent {
            PillTheme {
                PillRoot(
                    vm,
                    HostActions(
                        scanQr = vm::openScanner,
                        pickFiles = { picker.launch(arrayOf("*/*")) },
                        sendClipboard = { startActivity(Intent(this, ClipSendActivity::class.java)) },
                        perms = PermissionActions(
                            onNotificationAccess = { open(Perms.notificationAccessIntent(this)) },
                            onPostNotifications = {
                                if (Build.VERSION.SDK_INT >= 33) postNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                            },
                            onBattery = { open(Perms.batteryIntent(this), Perms.batteryFallbackIntent()) },
                            onWifiName = { location.launch(Manifest.permission.ACCESS_FINE_LOCATION) },
                            onStorage = { storage.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE) },
                        ),
                    ),
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handle(intent)
    }

    override fun onStart() {
        super.onStart()
        ClipBridge.foreground = true
    }

    override fun onStop() {
        ClipBridge.foreground = false
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        vm.refreshPerms()
        LinkService.start(this)   // no-op when unpaired; revives the link after a force-stop or a killed service
    }

    /** `pillphone://pair?...` links and the notification's "Scan again" action. */
    private fun handle(i: Intent?) {
        i ?: return
        val data = i.data
        when {
            i.action == Intent.ACTION_VIEW && data?.scheme == "pillphone" -> vm.onPairLink(data.toString())
            i.action == ACTION_SCAN -> vm.openScanner()
        }
    }

    private fun open(vararg intents: Intent) {
        for (i in intents) {
            try {
                startActivity(i)
                return
            } catch (_: Exception) {
            }
        }
        try { startActivity(Perms.appDetailsIntent(this)) } catch (_: Exception) { }
    }

    companion object {
        const val ACTION_SCAN = "app.pill.phone.SCAN"
    }
}
