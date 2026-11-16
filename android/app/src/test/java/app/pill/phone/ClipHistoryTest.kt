package app.pill.phone

import app.pill.phone.data.ClipHistory
import app.pill.phone.data.ClipItem
import app.pill.phone.data.ClipSync
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipHistoryTest {
    private val saved = ArrayList<String>()
    private fun history(stored: String? = null, keep: Boolean = true) = ClipHistory(stored, keep) { saved += it }
    private fun ClipHistory.texts() = items.value.map { it.text }

    @Test fun `newest first`() {
        val h = history()
        h.add("a", 1); h.add("b", 2); h.add("c", 3)
        assertEquals(listOf("c", "b", "a"), h.texts())
    }

    @Test fun `the same text as the newest only refreshes its time and keeps its source`() {
        val h = history()
        h.add("a", 1, "DESKTOP"); h.add("a", 5)
        assertEquals(listOf(ClipItem("a", 5, "DESKTOP")), h.items.value)
    }

    @Test fun `an older identical text moves to the top instead of being added twice`() {
        val h = history()
        h.add("a", 1); h.add("b", 2); h.add("a", 3, "PC")
        assertEquals(listOf(ClipItem("a", 3, "PC"), ClipItem("b", 2)), h.items.value)
    }

    @Test fun `empty and oversized texts are ignored, the protocol cap itself fits`() {
        val h = history()
        h.add("", 1); h.add("x".repeat(ClipSync.MAX_BYTES + 1), 2)
        assertTrue(h.items.value.isEmpty())
        h.add("x".repeat(ClipSync.MAX_BYTES), 3)
        assertEquals(1, h.items.value.size)
    }

    @Test fun `at most 100 items, the oldest fall off`() {
        val h = history()
        repeat(120) { h.add("t$it", it.toLong()) }
        assertEquals(100, h.items.value.size)
        assertEquals("t119", h.texts().first())
        assertEquals("t20", h.texts().last())
    }

    @Test fun `at most about 1 MiB of text in all, counted in UTF-8 bytes`() {
        val h = history()
        repeat(6) { h.add("$it" + "я".repeat(100_000), it.toLong()) } // ~200 KB each
        assertTrue(h.items.value.sumOf { ClipSync.utf8Size(it.text) } <= ClipHistory.MAX_BYTES)
        assertEquals(5, h.items.value.size)
        assertEquals("5", h.texts().first().take(1))
    }

    @Test fun `switching off clears and stops recording, switching on records again`() {
        val h = history()
        h.add("a", 1)
        h.setKeep(false)
        assertTrue(h.items.value.isEmpty())
        h.add("b", 2)
        assertTrue(h.items.value.isEmpty())
        assertEquals("[]", saved.last())
        h.setKeep(true); h.add("c", 3)
        assertEquals(listOf("c"), h.texts())
    }

    @Test fun `a history that was kept off never loads old text`() {
        assertTrue(history(ClipHistory.encode(listOf(ClipItem("a", 1))), keep = false).items.value.isEmpty())
    }

    @Test fun `remove and clear`() {
        val h = history()
        h.add("a", 1); h.add("b", 2)
        h.remove(ClipItem("a", 1))
        assertEquals(listOf("b"), h.texts())
        h.clear()
        assertTrue(h.items.value.isEmpty())
    }

    @Test fun `every change is saved, and the saved text loads back the same`() {
        val h = history()
        h.add("line1\nline \"2\" 😀 Привет", 7, "DESKTOP"); h.add("b", 9)
        assertEquals(h.items.value, history(saved.last()).items.value)
    }

    @Test fun `bad, empty and foreign data loads as an empty list`() {
        for (s in listOf(null, "", "   ", "not json", "{}", "[1,2]", "{\"a\":1}")) assertTrue(s, history(s).items.value.isEmpty())
    }

    @Test fun `damaged items are skipped, unknown keys ignored, old data re-capped and re-ordered`() {
        val h = history("""[{"text":"old","at":1},{"at":2},{"text":5,"at":3},{"text":"new","at":9,"from":"PC","future":true},{"text":"","at":4},null]""")
        assertEquals(listOf(ClipItem("new", 9, "PC"), ClipItem("old", 1)), h.items.value)
        val many = ClipHistory.encode((1..150).map { ClipItem("t$it", it.toLong()) })
        assertEquals(100, history(many).items.value.size)
    }
}
