package app.pill.phone

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import app.pill.phone.data.UpdateFail
import app.pill.phone.data.UpdatePhase
import app.pill.phone.net.Route
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PermissionsUi
import app.pill.phone.ui.SettingsUi
import app.pill.phone.ui.UpdateUi
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.screens.HomeActions
import app.pill.phone.ui.screens.HomeScreen
import app.pill.phone.ui.screens.SettingsActions
import app.pill.phone.ui.screens.SettingsScreen
import app.pill.phone.ui.screens.UpdateActions
import app.pill.phone.ui.screens.UpdateCard
import app.pill.phone.ui.screens.UpdatesCard
import app.pill.phone.ui.theme.PillTheme
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/** The update card on Home and the Settings "Updates" section in every state, dark and light (images land in docs/screenshots). */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h560dp-xhdpi", application = android.app.Application::class)
class UpdateScreenshotTest {
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

    private val notes = "Pill Phone checks for updates and installs them from inside the app. The pairing screen opens faster."
    private val ago = System.currentTimeMillis() - 5 * 60_000L
    private val available = UpdateUi(UpdatePhase.Available, "1.1.0", notes, "4.2 MB", lastCheck = ago)
    private val downloading = available.copy(phase = UpdatePhase.Downloading, progress = 0.42f)
    private val verifying = available.copy(phase = UpdatePhase.Verifying, progress = 1f)
    private val failed = available.copy(phase = UpdatePhase.Failed, fail = UpdateFail.Mismatch)
    private val needsPermission = available.copy(phase = UpdatePhase.NeedsPermission)
    private val upToDate = UpdateUi(UpdatePhase.UpToDate, lastCheck = ago)
    private val checking = UpdateUi(UpdatePhase.Checking, lastCheck = ago)
    private val checkFailed = UpdateUi(UpdatePhase.Failed, fail = UpdateFail.Check, lastCheck = ago)

    private val connected = HomeUi(
        LinkUi.Connected, "DESKTOP-VADIM", "192.168.1.20:41717", 87, false, "wifi", "Home",
        PermissionsUi(true, true, true, true), emptyList(), 12, route = Route.Lan,
    )

    @Composable private fun home(u: UpdateUi) = HomeScreen(connected.copy(update = u), HomeActions())

    // the card on Home: the first screen, nothing scrolled
    @Config(qualifiers = "w360dp-h800dp-xhdpi") @Test fun homeAvailable() = shoot("update-home-available") { home(available) }
    @Config(qualifiers = "w360dp-h800dp-xhdpi") @Test fun homeDownloading() = shoot("update-home-downloading") { home(downloading) }
    @Config(qualifiers = "w360dp-h800dp-xhdpi") @Test fun homeFailed() = shoot("update-home-failed") { home(failed) }
    @Config(qualifiers = "ru-w360dp-h800dp-xhdpi") @Test fun homeAvailableRu() = shoot("update-home-available-ru") { home(available) }

    // the cards alone, big enough to read
    @Composable private fun alone(u: UpdateUi, version: String = "1.0.0") {
        PillBackground {
            Column(Modifier.fillMaxSize().statusBarsPadding().padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                UpdateCard(u, UpdateActions())
                UpdatesCard(u, version, UpdateActions())
            }
        }
    }

    @Config(qualifiers = "w360dp-h900dp-xhdpi") @Test fun available() = shoot("update-available") { alone(available) }
    @Config(qualifiers = "w360dp-h900dp-xhdpi") @Test fun downloading() = shoot("update-downloading") { alone(downloading) }
    @Config(qualifiers = "w360dp-h900dp-xhdpi") @Test fun verifying() = shoot("update-verifying") { alone(verifying) }
    @Config(qualifiers = "w360dp-h900dp-xhdpi") @Test fun failed() = shoot("update-failed") { alone(failed) }
    @Config(qualifiers = "w360dp-h900dp-xhdpi") @Test fun permission() = shoot("update-permission") { alone(needsPermission) }
    @Config(qualifiers = "ru-w360dp-h900dp-xhdpi") @Test fun availableRu() = shoot("update-available-ru") { alone(available.copy(notes = "Pill Phone сам проверяет обновления и ставит их из приложения.", sizeText = "4,2 МБ")) }

    @Config(qualifiers = "w360dp-h400dp-xhdpi")
    @Test fun upToDate() = shoot("update-uptodate") { PillBackground { UpdatesCard(upToDate, "1.1.0", UpdateActions(), Modifier.padding(20.dp).statusBarsPadding()) } }

    @Config(qualifiers = "w360dp-h400dp-xhdpi")
    @Test fun checking() = shoot("update-checking") { PillBackground { UpdatesCard(checking, "1.0.0", UpdateActions(), Modifier.padding(20.dp).statusBarsPadding()) } }

    @Config(qualifiers = "w360dp-h400dp-xhdpi")
    @Test fun checkFailed() = shoot("update-checkfailed") { PillBackground { UpdatesCard(checkFailed, "1.0.0", UpdateActions(), Modifier.padding(20.dp).statusBarsPadding()) } }

    // the whole Settings page, to judge the section next to its neighbours
    @Config(qualifiers = "w360dp-h3000dp-xhdpi")
    @Test fun settingsWithUpdate() = shoot("update-settings") {
        SettingsScreen(
            SettingsUi("DESKTOP-VADIM", "192.168.1.20:41717", true, false, PermissionsUi(true, true, true, false, true), "1.0.0", "Pixel 8", "15", update = available),
            SettingsActions(),
        )
    }
}
