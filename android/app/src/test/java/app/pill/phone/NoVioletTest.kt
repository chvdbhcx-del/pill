package app.pill.phone

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/** The owner asked for a neutral black-and-white look: no colour literal in the app sources may be violet / indigo / periwinkle again. */
class NoVioletTest {
    private val hex = Regex("0x[0-9A-Fa-f]{2}([0-9A-Fa-f]{6})|#(?:[0-9A-Fa-f]{2})?([0-9A-Fa-f]{6})")

    /** Hue 225..330 degrees with saturation above 25 % (Google's own scanner blue, hue 217, stays outside). */
    private fun violet(rgb: String): Boolean {
        val (r, g, b) = (0..2).map { rgb.substring(it * 2, it * 2 + 2).toInt(16) / 255.0 }
        val max = maxOf(r, g, b); val d = max - minOf(r, g, b)
        if (max == 0.0 || d / max <= 0.25) return false
        val h = 60 * when (max) { r -> ((g - b) / d + 6) % 6; g -> (b - r) / d + 2; else -> (r - g) / d + 4 }
        return h in 225.0..330.0
    }

    @Test fun detectorSeesViolet() = assertTrue(violet("8F96FF") && violet("5B63E6") && !violet("4285F4") && !violet("FFFFFF") && !violet("58DDAB"))

    @Test fun noVioletColourLiteralsInMain() {
        val files = File("src/main").walkTopDown().filter { it.extension == "kt" || it.extension == "xml" }.toList()
        assertTrue("the sources were not found from ${File(".").absolutePath}", files.any { it.name == "Theme.kt" })
        val bad = files.asSequence().flatMap { f ->
            f.readLines().flatMapIndexed { i, l ->
                hex.findAll(l).map { m -> m.groupValues[1].ifEmpty { m.groupValues[2] } to "${f.name}:${i + 1}" }.toList()
            }
        }.filter { violet(it.first) }.map { "${it.second} #${it.first}" }.toList()
        assertTrue("violet colours: $bad", bad.isEmpty())
    }
}
