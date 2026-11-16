package app.pill.phone

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import app.pill.phone.data.ClipItem
import app.pill.phone.net.RelayState
import app.pill.phone.net.Route
import app.pill.phone.protocol.Msg
import app.pill.phone.ui.AppRowUi
import app.pill.phone.ui.ClipUi
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PcUi
import app.pill.phone.ui.PermissionsUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import app.pill.phone.ui.UpdateUi
import app.pill.phone.ui.components.BottomBar
import app.pill.phone.ui.components.PillTab
import app.pill.phone.ui.screens.AppsActions
import app.pill.phone.ui.screens.AppsScreen
import app.pill.phone.ui.screens.ClipboardActions
import app.pill.phone.ui.screens.ClipboardScreen
import app.pill.phone.ui.screens.FilesScreen
import app.pill.phone.ui.screens.HomeActions
import app.pill.phone.ui.screens.HomeScreen
import app.pill.phone.ui.theme.PillTheme
import app.pill.phone.data.UpdatePhase
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/** The 1.3.0 shell (bottom bar + the four tabs) at 1080x2400 (360x800 dp at 3x), dark and light; images land in docs/screenshots. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xxhdpi", application = android.app.Application::class)
class HomeScreenshotTest {
    @get:Rule val rule = createComposeRule()

    private val now = 1_700_000_000_000L

    private fun shoot(name: String, tab: PillTab? = PillTab.Home, content: @Composable () -> Unit) {
        var dark by mutableStateOf(true)
        rule.setContent {
            PillTheme(dark = dark, reduceMotion = true) {
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f)) { content() }
                    if (tab != null) BottomBar(tab, {})
                }
            }
        }
        val dir = File("../docs/screenshots").apply { mkdirs() }
        for (d in listOf(true, false)) {
            dark = d
            rule.waitForIdle()
            rule.onRoot().captureRoboImage(File(dir, "$name-${if (d) "dark" else "light"}.png").path)
        }
    }

    private val status = Msg.Status(
        93, false, "wifi", "Home", false, 0L, charge = "none", tempC = 32.0, health = "good",
        storage = Msg.Status.Space(36_500_000_000, 118_000_000_000), ram = Msg.Status.Space(2300, 7400),
        net = Msg.Status.NetInfo("wifi", mbps = 866, bars = 3), ringer = "vibrate", screen = true,
    )
    private val file = TransferUi("1", "IMG_2041.jpg", "3.1 MB", false, TransferPhase.Done, 1f, canOpen = true)
    private val perms = PermissionsUi(true, true, true, true)
    private val home = HomeUi(
        LinkUi.Connected, "HARD-VIDEO", "192.168.1.20:41717", 93, false, "wifi", "Home", perms, listOf(file), 124,
        status, Route.Lan, RelayState.Standby, lastClip = ClipItem("npm run build -- --release", now - 2 * 60_000, "HARD-VIDEO"),
    )

    private fun screen(ui: HomeUi) = @Composable { HomeScreen(ui, HomeActions(), nowMs = now) }

    @Test fun homeConnected() = shoot("c-home", content = screen(home))
    @Test fun homeEmpty() = shoot("c-home-empty", content = screen(home.copy(lastClip = null, transfers = emptyList(), mirroredApps = 0)))
    @Test fun homeBadgeOne() = shoot("c-home-badge-1", content = screen(home.copy(mirroredApps = 1)))
    @Test fun homeBadgeThousands() = shoot("c-home-badge-1000", content = screen(home.copy(mirroredApps = 1234)))
    @Test fun homeConnecting() = shoot("c-home-connecting", content = screen(home.copy(link = LinkUi.Connecting, route = null)))
    @Test fun homeNotFound() = shoot("c-home-offline", content = screen(home.copy(link = LinkUi.NotFound, route = null)))
    @Test fun homeNoNetwork() = shoot("c-home-nonetwork", content = screen(home.copy(link = LinkUi.NoNetwork, route = null)))
    @Test fun homeRevoked() = shoot("c-home-revoked", content = screen(HomeUi(link = LinkUi.Revoked, perms = perms)))
    @Test fun homeUnpaired() = shoot("c-home-unpaired", content = screen(HomeUi(perms = perms)))
    @Test fun homeInternet() = shoot("c-home-internet", content = screen(home.copy(route = Route.Relay, relay = RelayState.Active)))

    @Test fun homeLongName() = shoot("c-home-longname", content = screen(home.copy(
        pcName = "DESKTOP-WITH-AN-EXTREMELY-LONG-NAME-OF-THE-WORKSTATION-2026",
        lastClip = ClipItem("https://example.com/a/very/long/link/that/does/not/fit/on/one/single/line/of/the/phone?utm=mail", now - 3_600_000),
        transfers = listOf(file.copy(name = "Quarterly-financial-report-final-FINAL-v7-signed-by-everyone.docx"))),
    ))

    @Test fun homeBusy() = shoot("c-home-busy", content = screen(home.copy(
        transfers = listOf(TransferUi("2", "holiday.mp4", "182 MB", false, TransferPhase.Offered)),
        perms = PermissionsUi(false, false, false, false, true),
        update = UpdateUi(latest = "1.4.0", sizeText = "9.1 MB", phase = UpdatePhase.Available),
    )))

    @Config(qualifiers = "w360dp-h1200dp-xxhdpi")
    @Test fun homeTwoPcs() = shoot("c-home-2pcs", content = screen(home.copy(
        pcs = listOf(PcUi("a", "HARD-VIDEO", LinkUi.Connected, Route.Lan), PcUi("b", "Work laptop", LinkUi.NotFound)), selectedPcId = "a",
    )))

    @Config(qualifiers = "ru-w360dp-h800dp-xxhdpi")
    @Test fun homeRu() = shoot("c-home-ru", content = screen(home))

    @Config(qualifiers = "ru-w360dp-h800dp-xxhdpi")
    @Test fun homeRuOffline() = shoot("c-home-ru-offline", content = screen(home.copy(link = LinkUi.NotFound, route = null)))

    @Test fun files() = shoot("c-files", PillTab.Files) {
        FilesScreen(home.copy(transfers = listOf(
            TransferUi("1", "IMG_20261104_182233.jpg", "4.2 MB", true, TransferPhase.Active, 0.62f),
            TransferUi("2", "Quarterly report.docx", "1.1 MB", false, TransferPhase.Offered),
            file.copy(id = "3"), TransferUi("4", "backup.zip", "2.3 GB", false, TransferPhase.Failed, 0.3f, "Connection lost"),
        )), HomeActions())
    }
    @Test fun filesEmpty() = shoot("c-files-empty", PillTab.Files) { FilesScreen(home.copy(transfers = emptyList()), HomeActions()) }

    @Test fun clipboard() = shoot("c-clipboard", PillTab.Clipboard) {
        ClipboardScreen(
            ClipUi(listOf(
                ClipItem("npm run build -- --release", now - 2 * 60_000, "HARD-VIDEO"),
                ClipItem("Meeting moved to 15:30, room 4.\nBring the printed contract.", now - 40 * 60_000),
                ClipItem("ssh -p 2222 deploy@build.example.org", now - 3 * 3_600_000, "HARD-VIDEO"),
            ), canSend = true),
            ClipboardActions(), nowMs = now,
        )
    }

    @Test fun apps() = shoot("c-apps", PillTab.Apps) {
        AppsScreen(
            listOf(
                AppRowUi("org.telegram.messenger", "Telegram", true, false), AppRowUi("com.whatsapp", "WhatsApp", true, false),
                AppRowUi("com.instagram.android", "Instagram", false, false), AppRowUi("com.android.vending", "Play Store", false, true),
            ),
            "", false, 2, AppsActions(),
        )
    }
}
