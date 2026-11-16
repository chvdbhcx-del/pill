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
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.ui.platform.LocalDensity
import app.pill.phone.ui.components.BottomBar
import app.pill.phone.ui.components.PillTab
import app.pill.phone.ui.screens.FilesScreen
import app.pill.phone.ui.theme.Pill
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
import app.pill.phone.ui.screens.ClipboardActions
import app.pill.phone.ui.screens.ClipboardScreen
import app.pill.phone.ui.screens.HomeActions
import app.pill.phone.ui.screens.HomeScreen
import app.pill.phone.ui.screens.OnboardingScreen
import app.pill.phone.ui.screens.PairActions
import app.pill.phone.ui.screens.PairScreen
import app.pill.phone.ui.screens.PcPickerSheet
import app.pill.phone.ui.screens.PermissionActions
import app.pill.phone.ui.screens.SettingsActions
import app.pill.phone.ui.screens.SettingsScreen
import app.pill.phone.ui.screens.UpdateActions
import app.pill.phone.ui.theme.Motion

/** What only the Activity can do (system dialogs, scanner, picker). */
class HostActions(
    val scanQr: () -> Unit,
    val pickFiles: () -> Unit,
    val sendClipboard: () -> Unit,
    /** Android's "install unknown apps" page for this app. */
    val openInstallSettings: () -> Unit,
    val perms: PermissionActions,
)

private fun Route.tab(): PillTab? = when (this) {
    Route.Home -> PillTab.Home
    Route.Files -> PillTab.Files
    Route.Clipboard -> PillTab.Clipboard
    Route.Apps -> PillTab.Apps
    else -> null
}

private fun PillTab.route() = when (this) {
    PillTab.Home -> Route.Home
    PillTab.Files -> Route.Files
    PillTab.Clipboard -> Route.Clipboard
    PillTab.Apps -> Route.Apps
}

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
    val clip by vm.clip.collectAsStateWithLifecycle()
    val pendingClip by vm.pendingClip.collectAsStateWithLifecycle()
    val focusUpdates by vm.focusUpdates.collectAsStateWithLifecycle()
    val updateActions = UpdateActions(
        onCheck = vm::checkUpdates,
        onUpdate = { vm.updateNow(host.openInstallSettings) },
        onAuto = vm::setUpdateAuto,
        onDismiss = vm::dismissUpdate,
        onOpen = vm::openUpdates,
    )
    var askUnpair by remember { mutableStateOf<String?>(null) }
    var pickCount by remember { mutableIntStateOf(0) }
    pendingSend?.let { pickCount = it.sources.size }

    BackHandler(enabled = stack.size > 1) {
        if (stack.last() == Route.Pair) vm.pairBack() else vm.pop()
    }

    val homeActions = HomeActions(
        onSettings = { vm.push(Route.Settings) },
        onScan = host.scanQr,
        onEnterCode = { vm.openPair(manual = true) },
        onSendFiles = host.pickFiles,
        onSendClipboard = host.sendClipboard,
        onApps = { vm.openTab(Route.Apps) },
        onFiles = { vm.openTab(Route.Files) },
        onClipboard = { vm.openTab(Route.Clipboard) },
        onReconnect = vm::reconnect,
        onSelectPc = vm::selectPc,
        onOpenTransfer = vm::openTransfer,
        onAcceptTransfer = vm::acceptTransfer,
        update = updateActions,
        perms = host.perms,
    )

    // The bar shows on a tab that sits right above Home (Apps opened from Settings is deeper and gets a back arrow); the keyboard hides it.
    val imeUp = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    val tab = if (stack.size <= 2 && !imeUp) stack.last().tab() else null
    var lastTab by remember { mutableStateOf(PillTab.Home) }
    if (tab != null) lastTab = tab

    Box(Modifier.fillMaxSize().background(Pill.colors.background)) {
        Column(Modifier.fillMaxSize()) {
            // the bar takes the gesture-bar inset, so the screen above it must not take it again
            Box(Modifier.weight(1f).then(if (tab != null) Modifier.consumeWindowInsets(WindowInsets.navigationBars) else Modifier)) {
                AnimatedContent(
                    targetState = stack,
                    contentKey = { it.last() to it.size },
                    transitionSpec = {
                        val forward = targetState.size > initialState.size
                        val back = targetState.size < initialState.size
                        val from = initialState.last().tab()
                        val to = targetState.last().tab()
                        when {
                            // tab to tab: a short slide the way the tabs lie, with a cross-fade
                            from != null && to != null && initialState.size <= 2 && targetState.size <= 2 -> {
                                val dir = if (to.ordinal > from.ordinal) 1 else -1
                                (slideInHorizontally(Motion.soft()) { dir * it / 8 } + fadeIn(Motion.calm())) togetherWith
                                    (slideOutHorizontally(Motion.calm()) { -dir * it / 8 } + fadeOut(Motion.calm()))
                            }
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
                        Route.Home -> HomeScreen(home, homeActions)
                        Route.Files -> FilesScreen(home, homeActions)
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
                                onBack = if (s.size <= 2) null else ({ vm.pop() }),
                                onQuery = { vm.appsQuery.value = it },
                                onToggle = vm::setAppEnabled,
                                onShowSystem = { vm.showSystem.value = it },
                            ),
                            iconLoader = vm::icon,
                            pcName = appsTitle,
                        )
                        Route.Clipboard -> ClipboardScreen(
                            clip,
                            ClipboardActions(onAddCurrent = vm::addCurrentClip, onSend = vm::sendClip, onDelete = vm::removeClip, onClear = vm::clearClipHistory),
                            notices = vm.clipNotice,
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
                                onClip = vm::setClipEnabled,
                                onClipReceive = vm::setClipReceive,
                                onClipKeep = vm::setClipKeep,
                                onClipClear = vm::clearClipHistory,
                                onUnpair = vm::unpair,
                                onRenamePc = vm::renamePc,
                                onPcMirror = vm::setPcMirror,
                                onPcAutoReceive = vm::setPcAutoReceive,
                                onPcOwnFilter = vm::setPcOwnFilter,
                                onPair = { host.scanQr() },
                                update = updateActions,
                                perms = host.perms,
                            ),
                            askUnpair = askUnpair,
                            onAskUnpair = { askUnpair = it },
                            focusUpdates = focusUpdates,
                            onFocused = vm::updatesFocused,
                        )
                    }
                }
            }
            AnimatedVisibility(tab != null, enter = slideInVertically(Motion.soft()) { it } + fadeIn(Motion.calm()), exit = slideOutVertically(Motion.calm()) { it } + fadeOut(Motion.calm())) {
                BottomBar(lastTab, { vm.openTab(it.route()) })
            }
        }
        PcPickerSheet(
            visible = pendingClip != null, options = pcOptions, fileCount = 0,
            onPick = vm::chooseClipTargets, onClose = vm::dismissClip,
        )
        // "send to which PC?": over whatever screen the user is on, only while files wait for the answer
        PcPickerSheet(
            visible = pendingSend != null, options = pcOptions, fileCount = pickCount,
            onPick = vm::chooseTargets, onClose = vm::dismissSend,
        )
    }
}
