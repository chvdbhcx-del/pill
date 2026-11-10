package app.pill.phone.ui

import android.Manifest
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.pill.phone.MainViewModel
import app.pill.phone.protocol.PairLink
import app.pill.phone.ui.screens.ScanPhase
import app.pill.phone.ui.screens.ScanUi
import app.pill.phone.ui.screens.ScannerActions
import app.pill.phone.ui.screens.ScannerScreen
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private fun Context.activity(): ComponentActivity? {
    var c: Context? = this
    while (c is ContextWrapper) { if (c is ComponentActivity) return c; c = c.baseContext }
    return null
}

/** The scanner with its camera permission flow: rationale first, then the system dialog, then the denied states. */
@Composable
fun ScannerRoute(vm: MainViewModel, onClose: () -> Unit) {
    val ctx = LocalContext.current
    val haptic = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()

    fun granted() = ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
    var hasCamera by remember { mutableStateOf(granted()) }
    var denial by remember { mutableStateOf<ScanUi>(ScanUi.Rationale) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { hasCamera = granted() }   // e.g. back from the app settings

    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        hasCamera = ok
        if (!ok) {
            val act = ctx.activity()
            denial = if (act != null && ActivityCompat.shouldShowRequestPermissionRationale(act, Manifest.permission.CAMERA)) ScanUi.Denied else ScanUi.DeniedForever
        }
    }

    var phase by remember { mutableStateOf(ScanPhase.Scanning) }
    var torch by remember { mutableStateOf(false) }
    var hasTorch by remember { mutableStateOf(true) }
    var rejects by remember { mutableIntStateOf(0) }
    var lastReject by remember { mutableStateOf(0L) }

    fun onQr(raw: String) {
        scope.launch {
            if (phase == ScanPhase.Success) return@launch
            if (PairLink.parse(raw) != null) {
                phase = ScanPhase.Success
                haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                delay(900)   // let the success animation play before the pairing screen takes over
                vm.onScanned(raw)
            } else {
                val now = System.currentTimeMillis()
                if (now - lastReject < 2000) return@launch
                lastReject = now
                rejects++
                phase = ScanPhase.Rejected
                delay(2600)
                if (phase == ScanPhase.Rejected) phase = ScanPhase.Scanning
            }
        }
    }

    val ui: ScanUi = if (hasCamera) ScanUi.Camera(phase, torch, hasTorch) else denial
    ScannerScreen(
        ui,
        ScannerActions(
            onClose = onClose,
            onTorch = { torch = !torch },
            onAllow = { ask.launch(Manifest.permission.CAMERA) },
            onOpenSettings = {
                try {
                    ctx.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", ctx.packageName, null)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                } catch (_: Exception) {
                }
            },
            onEnterCode = vm::enterCodeInstead,
        ),
        rejectCount = rejects,
    ) { m -> CameraView(m, torch, onHasTorch = { hasTorch = it }, onQr = ::onQr) }
}
