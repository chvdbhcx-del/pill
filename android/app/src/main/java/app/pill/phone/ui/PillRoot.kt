package app.pill.phone.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.pill.phone.MainViewModel
import app.pill.phone.Route
import app.pill.phone.ui.screens.AppsActions
import app.pill.phone.ui.screens.AppsScreen
import app.pill.phone.ui.screens.HomeActions
import app.pill.phone.ui.screens.HomeScreen
import app.pill.phone.ui.screens.OnboardingScreen
import app.pill.phone.ui.screens.PairActions
import app.pill.phone.ui.screens.PairScreen
import app.pill.phone.ui.screens.PcPickerSheet
import app.pill.phone.ui.screens.PermissionActions
import app.pill.phone.ui.screens.SettingsActions
import app.pill.phone.ui.screens.SettingsScreen
import app.pill.phone.ui.theme.Motion

/** What only the Activity can do (system dialogs, scanner, picker). */
class HostActions(
    val scanQr: () -> Unit,
    val pickFiles: () -> Unit,
    val perms: PermissionActions,
)

@Composable
fun PillRoot(vm: MainViewModel, host: HostActions) {
    val stack by vm.stack.collectAsStateWithLifecycle()
    val home by vm.home.collectAsStateWithLifecycle()
    val settings by vm.settings.collectAsStateWithLifecycle()
    val pair by vm.pair.collectAsStateWithLifecycle()
    val rows by vm.appRows.collectAsStateWithLifecycle()
    val query by vm.appsQuery.collectAsStateWithLifecycle()
    val showSystem by vm.showSystem.collectAsStateWithLifecycle()
    val appsTitle by vm.appsTitle.collectAsStateWithLifecycle()
    val pendingSend by vm.pendingSend.collectAsStateWithLifecycle()
    val pcOptions by vm.pcOptions.collectAsStateWithLifecycle()
    var askUnpair by remember { mutableStateOf<String?>(null) }
    var pickCount by remember { mutableIntStateOf(0) }
    pendingSend?.let { pickCount = it.sources.size }

    BackHandler(enabled = stack.size > 1) {
        if (stack.last() == Route.Pair) vm.pairBack() else vm.pop()
    }

    Box(Modifier.fillMaxSize()) {
    AnimatedContent(
        targetState = stack,
        contentKey = { it.last() to it.size },
        transitionSpec = {
            val forward = targetState.size > initialState.size
            val back = targetState.size < initialState.size
            when {
                forward -> (slideInHorizontally(Motion.soft()) { it / 3 } + fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.96f)) togetherWith
                    (slideOutHorizontally(Motion.soft()) { -it / 5 } + fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.96f))
                back -> (slideInHorizontally(Motion.soft()) { -it / 5 } + fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.96f)) togetherWith
                    (slideOutHorizontally(Motion.soft()) { it / 3 } + fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.96f))
                else -> (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.94f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 1.04f))
            }
        },
        label = "route",
    ) { s ->
        when (s.last()) {
            Route.Onboarding -> OnboardingScreen(onFinish = vm::finishOnboarding)
            Route.Home -> HomeScreen(
                home,
                HomeActions(
                    onSettings = { vm.push(Route.Settings) },
                    onScan = host.scanQr,
                    onEnterCode = { vm.openPair(manual = true) },
                    onSendFiles = host.pickFiles,
                    onApps = { vm.openApps(null) },
                    onReconnect = vm::reconnect,
                    onSelectPc = vm::selectPc,
                    onTogglePhone = vm::togglePhone,
                    onOpenTransfer = vm::openTransfer,
                    onAcceptTransfer = vm::acceptTransfer,
                    perms = host.perms,
                ),
            )
            Route.Pair -> PairScreen(
                pair,
                PairActions(
                    onBack = vm::pairBack,
                    onScan = host.scanQr,
                    onManual = vm::setManual,
                    onAddress = vm::setAddress,
                    onCode = vm::setCode,
                    onSubmit = vm::submitManual,
                    onConfirm = vm::confirmPair,
                    onRetry = vm::retryPair,
                ),
            )
            Route.Scan -> ScannerRoute(vm, onClose = { vm.pop() })
            Route.Apps -> AppsScreen(
                rows, query, showSystem, rows.count { it.enabled },
                AppsActions(
                    onBack = { vm.pop() },
                    onQuery = { vm.appsQuery.value = it },
                    onToggle = vm::setAppEnabled,
                    onShowSystem = { vm.showSystem.value = it },
                ),
                iconLoader = vm::icon,
                pcName = appsTitle,
            )
            Route.Settings -> SettingsScreen(
                settings,
                SettingsActions(
                    onBack = { vm.pop() },
                    onApps = { vm.openApps(null) },
                    onPcApps = vm::openApps,
                    onAutoReceive = vm::setAutoReceive,
                    onOngoing = vm::setIncludeOngoing,
                    onRelay = vm::setRelayEnabled,
                    onUnpair = vm::unpair,
                    onRenamePc = vm::renamePc,
                    onPcMirror = vm::setPcMirror,
                    onPcAutoReceive = vm::setPcAutoReceive,
                    onPcOwnFilter = vm::setPcOwnFilter,
                    onPair = { host.scanQr() },
                    perms = host.perms,
                ),
                askUnpair = askUnpair,
                onAskUnpair = { askUnpair = it },
            )
        }
    }
    // "send to which PC?": over whatever screen the user is on, only while files wait for the answer
    PcPickerSheet(
        visible = pendingSend != null, options = pcOptions, fileCount = pickCount,
        onPick = vm::chooseTargets, onClose = vm::dismissSend,
    )
    }
}
