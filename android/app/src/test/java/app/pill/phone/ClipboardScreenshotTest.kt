package app.pill.phone

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import app.pill.phone.data.ClipItem
import app.pill.phone.net.Route
import app.pill.phone.ui.ClipUi
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PermissionsUi
import app.pill.phone.ui.SettingsUi
import app.pill.phone.ui.screens.HomeActions
import app.pill.phone.ui.screens.HomeScreen
import app.pill.phone.ui.screens.SettingsActions
import app.pill.phone.ui.screens.SettingsScreen
import app.pill.phone.ui.screens.ClipboardActions
import app.pill.phone.ui.screens.ClipboardScreen
import app.pill.phone.ui.theme.PillTheme
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/** The Clipboard screen in its states, dark and light (images land in docs/screenshots). */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h780dp-xhdpi", application = android.app.Application::class)
class ClipboardScreenshotTest {
    @get:Rule val rule = createComposeRule()

    private val now = 1_700_000_000_000L
    private val five = listOf(
        ClipItem("https://example.com/invoices/2026-11?utm=mail", now - 2 * 60_000, "DESKTOP-VADIM"),
        ClipItem("Meeting moved to 15:30, room 4.\nBring the printed contract and the signed annex.", now - 40 * 60_000),
        ClipItem("ssh -p 2222 deploy@build.example.org", now - 3 * 3_600_000, "DESKTOP-VADIM"),
        ClipItem("Привет! Отправляю файлы завтра утром.", now - 26 * 3_600_000),
        ClipItem("4242 4242 4242 4242", now - 3 * 86_400_000),
    )
    private val twelve = five + (1..7).map { ClipItem("Note number $it about the pill project", now - (4 + it) * 86_400_000L) }

    private fun shoot(name: String, ui: ClipUi, actions: ClipboardActions = ClipboardActions(), steps: () -> Unit = {}) =
        shoot(name, steps) { ClipboardScreen(ui, actions, nowMs = now) }

    private fun shoot(name: String, steps: () -> Unit = {}, content: @androidx.compose.runtime.Composable () -> Unit) {
        var dark by mutableStateOf(true)
        rule.mainClock.autoAdvance = true
        rule.setContent { PillTheme(dark = dark, reduceMotion = true) { content() } }
        rule.waitForIdle()
        rule.mainClock.autoAdvance = false
        steps()
        rule.mainClock.advanceTimeBy(1000)
        val dir = File("../docs/screenshots").apply { mkdirs() }
        // a dialog window renders blank on its first capture: take one that is thrown away
        if (name.endsWith("clear")) File.createTempFile("warm", ".png").also { rule.onNode(isDialog()).captureRoboImage(it.path) }.delete()
        for (d in listOf(true, false)) {
            dark = d
            androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications()
            rule.mainClock.advanceTimeBy(300)
            val node = if (name.endsWith("clear")) rule.onNode(isDialog()) else rule.onRoot()
            node.captureRoboImage(File(dir, "$name-${if (d) "dark" else "light"}.png").path)
        }
    }

    @Test fun empty() = shoot("clipboard-empty", ClipUi())
    @Test fun off() = shoot("clipboard-off", ClipUi(keep = false))
    @Test fun five() = shoot("clipboard-5", ClipUi(five, canSend = true))
    @Test fun fiveNoSend() = shoot("clipboard-5-nosend", ClipUi(five, canSend = false))

    @Test fun search() = shoot("clipboard-search", ClipUi(twelve, canSend = true)) {
        rule.onNode(hasSetTextAction()).performTextInput("pill")
    }

    @Test fun copiedConfirmation() = shoot("clipboard-copied", ClipUi(five, canSend = true)) {
        rule.onNodeWithText("ssh -p 2222 deploy@build.example.org").performClick()
    }

    @Test fun clearDialog() = shoot("clipboard-clear", ClipUi(five, canSend = true)) {
        rule.onNodeWithContentDescription("Clear all").performClick()
    }

    @Test fun homeWithClipboardSync() = shoot("clipboard-home-sync") {
        HomeScreen(
            HomeUi(LinkUi.Connected, "DESKTOP-VADIM", "192.168.1.20:41717", 87, false, "wifi", "Home", PermissionsUi(true, true, true, true), emptyList(), 12, route = Route.Lan),
            HomeActions(),
        )
    }

    @Config(qualifiers = "w360dp-h1500dp-xhdpi")
    @Test fun settingsClipboard() = shoot("clipboard-settings") {
        SettingsScreen(SettingsUi("DESKTOP-VADIM", "192.168.1.20:41717", true, false, PermissionsUi(true, true, true, true, true), "1.2.0", clipEnabled = true), SettingsActions())
    }

    @Test fun deleteAndSendAreReported() {
        val log = ArrayList<String>()
        rule.setContent {
            PillTheme(dark = true, reduceMotion = true) {
                ClipboardScreen(ClipUi(five.take(2), canSend = true), ClipboardActions(onSend = { log += "send:$it" }, onDelete = { log += "del:${it.text.take(5)}" }), nowMs = now)
            }
        }
        rule.onAllNodesWithContentDescription("Send to PC")[0].performClick()
        rule.onAllNodesWithContentDescription("Delete")[1].performClick()
        assertEquals(listOf("send:https://example.com/invoices/2026-11?utm=mail", "del:Meeti"), log)
    }
}
