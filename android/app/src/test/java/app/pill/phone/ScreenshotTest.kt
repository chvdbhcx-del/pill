package app.pill.phone

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import app.pill.phone.ui.AppRowUi
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PairError
import app.pill.phone.ui.PairUi
import app.pill.phone.ui.PermissionsUi
import app.pill.phone.ui.SettingsUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import app.pill.phone.ui.screens.AppsActions
import app.pill.phone.ui.screens.AppsScreen
import app.pill.phone.ui.screens.HomeActions
import app.pill.phone.ui.screens.HomeScreen
import app.pill.phone.ui.screens.OnboardingScreen
import app.pill.phone.ui.screens.PairActions
import app.pill.phone.ui.screens.PairScreen
import app.pill.phone.ui.screens.ScanPhase
import app.pill.phone.ui.screens.ScanUi
import app.pill.phone.ui.screens.ScannerActions
import app.pill.phone.ui.screens.ScannerScreen
import app.pill.phone.ui.screens.SendSheet
import app.pill.phone.ui.screens.SendUi
import app.pill.phone.ui.screens.SettingsActions
import app.pill.phone.ui.screens.SettingsScreen
import app.pill.phone.ui.theme.PillTheme
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * Renders the real Compose screens to PNG on the JVM (no device). Run with
 * `./gradlew :app:recordRoborazziDebug --tests "*ScreenshotTest*"`; the images land in docs/screenshots.
 * Motion is reduced so the frame is the settled state, not a mid-animation one.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h780dp-xhdpi", application = android.app.Application::class)
class ScreenshotTest {
    @get:Rule val rule = createComposeRule()

    private fun shoot(name: String, content: @Composable () -> Unit) {
        var dark by mutableStateOf(true)
        rule.setContent { PillTheme(dark = dark, reduceMotion = true) { content() } }
        val dir = File("../docs/screenshots").apply { mkdirs() }
        for (d in listOf(true, false)) {
            dark = d
            rule.waitForIdle()
            rule.onRoot().captureRoboImage(File(dir, "$name-${if (d) "dark" else "light"}.png").path)
        }
    }

    private val perms = PermissionsUi(true, true, true, false, true)
    private val transfers = listOf(
        TransferUi("1", "IMG_20261104_182233.jpg", "4.2 MB", true, TransferPhase.Active, 0.62f),
        TransferUi("2", "Quarterly report.docx", "1.1 MB", false, TransferPhase.Offered),
        TransferUi("3", "holiday.mp4", "182 MB", true, TransferPhase.Done, 1f),
        TransferUi("4", "backup.zip", "2.3 GB", false, TransferPhase.Failed, 0.3f, "Connection lost"),
    )
    private val connected = HomeUi(
        LinkUi.Connected, "DESKTOP-VADIM", "192.168.1.20:41717", 87, false, "wifi", "Home", perms, transfers, 12,
        app.pill.phone.protocol.Msg.Status(
            87, false, "wifi", "Home", false, 0L, charge = "none", tempC = 31.5, health = "good",
            storage = app.pill.phone.protocol.Msg.Status.Space(52_000_000_000, 128_000_000_000), ram = app.pill.phone.protocol.Msg.Status.Space(3072, 8000),
            net = app.pill.phone.protocol.Msg.Status.NetInfo("wifi", mbps = 866, bars = 3), ringer = "vibrate", screen = true,
        ),
    )

    @Test fun onboarding1() = shoot("onboarding-1") { OnboardingScreen({}, 0) }
    @Test fun onboarding2() = shoot("onboarding-2") { OnboardingScreen({}, 1) }
    @Test fun onboarding3() = shoot("onboarding-3") { OnboardingScreen({}, 2) }

    private val connectedQuiet = connected.copy(transfers = emptyList(), perms = PermissionsUi(true, true, true, true))
    private val busyPhone = app.pill.phone.protocol.Msg.Status(
        77, true, "wifi", "Home", true, 0L, charge = "charging", plug = "usb", tempC = 30.0, health = "good", fullInMin = 42,
        storage = app.pill.phone.protocol.Msg.Status.Space(52_000_000_000, 128_000_000_000), ram = app.pill.phone.protocol.Msg.Status.Space(3072, 8000),
        net = app.pill.phone.protocol.Msg.Status.NetInfo("wifi", mbps = 702, bars = 4), ringer = "silent", screen = true, airplane = false,
    )

    // 1080x2400 is about 360x800 dp: this is the first screen, nothing scrolled
    @Config(qualifiers = "w360dp-h800dp-xhdpi")
    @Test fun homeConnected() = shoot("home-connected") { HomeScreen(connectedQuiet.copy(route = app.pill.phone.net.Route.Lan), HomeActions()) }

    @Config(qualifiers = "w360dp-h800dp-xhdpi")
    @Test fun homeConnectedExpanded() = shoot("home-connected-expanded") {
        HomeScreen(connectedQuiet.copy(route = app.pill.phone.net.Route.Lan, status = busyPhone, phoneExpanded = true), HomeActions())
    }

    @Config(qualifiers = "ru-w360dp-h800dp-xhdpi")
    @Test fun homeConnectedRu() = shoot("home-connected-ru") {
        HomeScreen(connectedQuiet.copy(route = app.pill.phone.net.Route.Relay, status = busyPhone, phoneExpanded = true, mirroredApps = 128), HomeActions())
    }

    @Config(qualifiers = "ru-w360dp-h800dp-xhdpi")
    @Test fun homeNotFoundRu() = shoot("home-notfound-ru") { HomeScreen(connectedQuiet.copy(link = LinkUi.NotFound), HomeActions()) }

    @Config(qualifiers = "w360dp-h900dp-xhdpi")
    @Test fun homeConnectedFiles() = shoot("home-connected-files") { HomeScreen(connectedQuiet.copy(transfers = transfers, route = app.pill.phone.net.Route.Lan), HomeActions()) }

    @Config(qualifiers = "w360dp-h1000dp-xhdpi")
    @Test fun homeConnectedEmptyAndPerms() = shoot("home-connected-setup") {
        HomeScreen(connected.copy(transfers = emptyList(), perms = PermissionsUi(false, false, false, false, true), route = app.pill.phone.net.Route.Lan), HomeActions())
    }

    @Config(qualifiers = "w360dp-h1000dp-xhdpi")
    @Test fun homeUnpaired() = shoot("home-unpaired") { HomeScreen(HomeUi(perms = PermissionsUi(true, true, true, true, true)), HomeActions()) }

    @Config(qualifiers = "w360dp-h800dp-xhdpi")
    @Test fun homeConnecting() = shoot("home-connecting") { HomeScreen(connected.copy(link = LinkUi.Connecting, transfers = emptyList(), perms = PermissionsUi(true, true, true, true)), HomeActions()) }
    @Test fun homeNotFound() = shoot("home-notfound") { HomeScreen(connected.copy(link = LinkUi.NotFound, transfers = emptyList(), perms = PermissionsUi(true, true, true, true)), HomeActions()) }
    @Test fun homeNoNetwork() = shoot("home-nonetwork") { HomeScreen(connected.copy(link = LinkUi.NoNetwork, transfers = emptyList(), perms = PermissionsUi(true, true, true, true)), HomeActions()) }
    @Test fun homeRevoked() = shoot("home-revoked") { HomeScreen(connected.copy(link = LinkUi.Revoked, transfers = emptyList(), perms = PermissionsUi(true, true, true, true)), HomeActions()) }

    @Test fun pairChoose() = shoot("pair-choose") { PairScreen(PairUi.Entry(), PairActions()) }
    @Test fun pairManual() = shoot("pair-manual") { PairScreen(PairUi.Entry(true, "192.168.1.20:41717", "AB3D9X", PairError.BadCode), PairActions()) }
    @Test fun pairConfirm() = shoot("pair-confirm") { PairScreen(PairUi.Confirm("192.168.1.20:41717", "DESKTOP-VADIM"), PairActions()) }
    @Test fun pairWorking() = shoot("pair-working") { PairScreen(PairUi.Working("192.168.1.20:41717"), PairActions()) }
    @Test fun pairSuccess() = shoot("pair-success") { PairScreen(PairUi.Success("DESKTOP-VADIM"), PairActions()) }
    @Test fun pairFailed() = shoot("pair-failed") { PairScreen(PairUi.Failed(PairError.Unreachable, "192.168.1.20:41717"), PairActions()) }

    private val apps = listOf(
        AppRowUi("org.telegram.messenger", "Telegram", true, false),
        AppRowUi("com.whatsapp", "WhatsApp", true, false),
        AppRowUi("com.google.android.gm", "Gmail", true, false),
        AppRowUi("com.instagram.android", "Instagram", false, false),
        AppRowUi("ru.yandex.taxi", "Yandex Go", false, false),
        AppRowUi("com.android.vending", "Play Store", false, true),
    )

    @Config(qualifiers = "w360dp-h900dp-xhdpi")
    @Test fun appsList() = shoot("apps") { AppsScreen(apps, "", true, 3, AppsActions()) }
    @Test fun appsEmpty() = shoot("apps-empty") { AppsScreen(emptyList(), "zzz", false, 3, AppsActions()) }

    @Config(qualifiers = "w360dp-h1250dp-xhdpi")
    @Test fun settingsPaired() = shoot("settings") {
        SettingsScreen(SettingsUi("DESKTOP-VADIM", "192.168.1.20:41717", true, false, PermissionsUi(true, true, false, false, true), "1.0.0"), SettingsActions())
    }

    @Test fun settingsUnpaired() = shoot("settings-unpaired") {
        SettingsScreen(SettingsUi(null, null, false, false, PermissionsUi(true, true, true, true, true), "1.0.0"), SettingsActions())
    }

    // ---- internet relay (protocol v2) ----
    private val setupDone = PermissionsUi(true, true, true, true)
    private val sNet = app.pill.phone.net.RelayState.values()

    @Test fun homeOnHomeNetwork() = shoot("home-route-lan") {
        HomeScreen(connected.copy(transfers = emptyList(), perms = setupDone, route = app.pill.phone.net.Route.Lan), HomeActions())
    }
    @Test fun homeOnInternet() = shoot("home-route-internet") {
        HomeScreen(connected.copy(transfers = emptyList(), perms = setupDone, route = app.pill.phone.net.Route.Relay, relay = app.pill.phone.net.RelayState.Active), HomeActions())
    }
    @Test fun homePcOffline() = shoot("home-route-pcoffline") {
        HomeScreen(connected.copy(link = LinkUi.NotFound, transfers = emptyList(), perms = setupDone, relay = app.pill.phone.net.RelayState.WaitingForPc), HomeActions())
    }

    @Config(qualifiers = "w360dp-h1450dp-xhdpi")
    @Test fun settingsRelayActive() = shoot("settings-relay") {
        SettingsScreen(
            SettingsUi("DESKTOP-VADIM", "192.168.1.20:41717", true, false, PermissionsUi(true, true, true, false, true), "1.0.0", "Pixel 8", "15",
                relayEnabled = true, relay = app.pill.phone.net.RelayState.Active, route = app.pill.phone.net.Route.Relay, relayMaxMb = 100),
            SettingsActions(),
        )
    }
    @Config(qualifiers = "w360dp-h1450dp-xhdpi")
    @Test fun settingsRelayUnavailable() = shoot("settings-relay-old-pc") {
        SettingsScreen(
            SettingsUi("DESKTOP-VADIM", "192.168.1.20:41717", true, false, PermissionsUi(true, true, true, false, true), "1.0.0", "Pixel 8", "15",
                relayEnabled = true, relay = app.pill.phone.net.RelayState.Unavailable),
            SettingsActions(),
        )
    }

    @Test fun sendSheetSending() = shoot("send-sending") { SendSheet(SendUi.Sending(2), {}, {}) }
    @Test fun sendSheetNotConnected() = shoot("send-notconnected") { SendSheet(SendUi.NotConnected, {}, {}) }

    // A stand-in for the camera picture: the real preview is a CameraX view that needs a device.
    private val fakeCamera: @Composable (Modifier) -> Unit = { m ->
        Box(m.background(Brush.linearGradient(listOf(Color(0xFF8A7F6A), Color(0xFF3B4A3F), Color(0xFF1E2A33)))))
    }

    @Test fun scanScanning() = shoot("scan-scanning") { ScannerScreen(ScanUi.Camera(), ScannerActions(), 0, fakeCamera) }
    @Test fun scanTorch() = shoot("scan-torch") { ScannerScreen(ScanUi.Camera(torch = true), ScannerActions(), 0, fakeCamera) }
    @Test fun scanRejected() = shoot("scan-rejected") { ScannerScreen(ScanUi.Camera(ScanPhase.Rejected), ScannerActions(), 1, fakeCamera) }
    @Test fun scanSuccess() = shoot("scan-success") { ScannerScreen(ScanUi.Camera(ScanPhase.Success), ScannerActions(), 0, fakeCamera) }
    @Test fun scanRationale() = shoot("scan-rationale") { ScannerScreen(ScanUi.Rationale, ScannerActions()) {} }
    @Test fun scanDenied() = shoot("scan-denied") { ScannerScreen(ScanUi.Denied, ScannerActions()) {} }
    @Test fun scanDeniedForever() = shoot("scan-denied-forever") { ScannerScreen(ScanUi.DeniedForever, ScannerActions()) {} }
}
