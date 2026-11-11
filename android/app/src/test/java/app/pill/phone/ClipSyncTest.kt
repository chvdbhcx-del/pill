package app.pill.phone

import app.pill.phone.data.ClipSendResult
import app.pill.phone.data.ClipSync
import app.pill.phone.data.ClipboardAccess
import app.pill.phone.data.FanOut
import app.pill.phone.data.Routing
import app.pill.phone.protocol.Codec
import app.pill.phone.protocol.Msg
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Protocol v1.2 clipboard sync: the wire format, the limits, caps gating, the loop guard, and receiving through an abstract clipboard. */
class ClipSyncTest {
    private class FakeBoard(var text: String? = null) : ClipboardAccess {
        val writes = ArrayList<String>()
        override fun read() = text
        override fun write(text: String) { writes += text; this.text = text }
    }

    private var clock = 1_000_000L
    private var on = true
    private var receiving = true
    private val board = FakeBoard()
    private var ids = 0
    private val sync = ClipSync(board, { on }, { receiving }, { clock }, { "id-${ids++}" })

    /** A sender that records what went where; [up] are the PCs that take it. */
    private class Outbox(private val up: Set<String>) {
        val sent = ArrayList<Pair<String, Msg>>()
        fun send(ids: Collection<String>, msg: Msg): FanOut = Routing.fanOut(ids, msg) { id, m ->
            (id in up).also { if (it) sent += id to m }
        }
    }

    // ---- wire format ----

    @Test fun `caps is exactly the spec`() {
        assertEquals("""{"t":"caps","clip":1}""", Codec.encode(Msg.Caps(1)))
        assertEquals(Msg.Caps(1), Codec.decode("""{"t":"caps","clip":1}"""))
    }

    @Test fun `clip is exactly the spec and round-trips text with newlines quotes and emoji`() {
        val m = Msg.Clip("u-1", "line1\nline \"2\" \\ 😀 Привет", 1700000000123)
        val wire = Codec.encode(m)
        assertTrue(wire, wire.startsWith("""{"t":"clip","id":"u-1","text":"""))
        assertTrue(wire, wire.endsWith(""","ts":1700000000123}"""))
        assertEquals(m, Codec.decode(wire))
    }

    @Test fun `a clip from the PC decodes, extra keys are ignored`() {
        assertEquals(Msg.Clip("x", "hi", 5), Codec.decode("""{"t":"clip","id":"x","text":"hi","ts":5,"future":true}"""))
    }

    @Test fun `old peers' view - an unknown message type is simply ignored`() {
        assertNull(Codec.decode("""{"t":"clip2","id":"x"}"""))
    }

    // ---- caps gating ----

    @Test fun `no caps while the feature is off, caps once it is on`() {
        on = false
        assertNull(sync.caps())
        on = true
        assertEquals(Msg.Caps(1), sync.caps())
    }

    // ---- size limits ----

    @Test fun `utf8 size counts bytes not chars`() {
        assertEquals(3, ClipSync.utf8Size("abc"))
        assertEquals(4, ClipSync.utf8Size("жж"))      // 2 bytes each
        assertEquals(3, ClipSync.utf8Size("€"))            // euro sign
        assertEquals(4, ClipSync.utf8Size("😀"))      // a surrogate pair is one 4-byte code point
        assertEquals("aж€😀".toByteArray().size, ClipSync.utf8Size("aж€😀"))
    }

    @Test fun `exactly 256 KiB goes, one byte more does not`() {
        val max = "a".repeat(ClipSync.MAX_BYTES)
        assertNull(sync.check(max))
        assertEquals(ClipSendResult.TooLarge, sync.check(max + "a"))
        // 128 Ki two-byte chars is exactly the limit, one more is over
        assertNull(sync.check("ж".repeat(ClipSync.MAX_BYTES / 2)))
        assertEquals(ClipSendResult.TooLarge, sync.check("ж".repeat(ClipSync.MAX_BYTES / 2 + 1)))
    }

    @Test fun `empty text is not sent and nothing leaves`() {
        val out = Outbox(setOf("a"))
        assertEquals(ClipSendResult.Empty, sync.send("", listOf("a"), out::send))
        assertEquals(ClipSendResult.Empty, sync.send(null, listOf("a"), out::send))
        assertTrue(out.sent.isEmpty())
    }

    @Test fun `too big text is not sent`() {
        val out = Outbox(setOf("a"))
        assertEquals(ClipSendResult.TooLarge, sync.send("a".repeat(ClipSync.MAX_BYTES + 1), listOf("a"), out::send))
        assertTrue(out.sent.isEmpty())
    }

    @Test fun `nothing is sent while the feature is off`() {
        on = false
        val out = Outbox(setOf("a"))
        assertEquals(ClipSendResult.Off, sync.send("hello", listOf("a"), out::send))
        assertTrue(out.sent.isEmpty())
    }

    // ---- sending ----

    @Test fun `sending builds the spec message and fans out to every chosen PC`() {
        val out = Outbox(setOf("a", "b"))
        val r = sync.send("hello", listOf("a", "b"), out::send)
        assertEquals(ClipSendResult.Sent(listOf("a", "b")), r)
        val msgs = out.sent.map { it.second as Msg.Clip }
        assertEquals(listOf("hello", "hello"), msgs.map { it.text })
        assertEquals(clock, msgs[0].ts)
        assertNotNull(msgs[0].id)
        assertEquals(listOf("a", "b"), out.sent.map { it.first })
    }

    @Test fun `an offline PC does not stop the others and the result lists who got it`() {
        val out = Outbox(setOf("b"))
        assertEquals(ClipSendResult.Sent(listOf("b")), sync.send("x", listOf("a", "b"), out::send))
    }

    @Test fun `all PCs down is a failure and is not remembered, so a retry goes out`() {
        assertEquals(ClipSendResult.Failed, sync.send("x", listOf("a"), Outbox(emptySet())::send))
        val out = Outbox(setOf("a"))
        assertEquals(ClipSendResult.Sent(listOf("a")), sync.send("x", listOf("a"), out::send))
    }

    // ---- receiving ----

    @Test fun `a clip from the PC is put on the clipboard`() {
        assertTrue(sync.receive(Msg.Clip("1", "from pc", clock)))
        assertEquals(listOf("from pc"), board.writes)
    }

    @Test fun `nothing is applied while the feature or receiving is off`() {
        on = false
        assertFalse(sync.receive(Msg.Clip("1", "a", clock)))
        on = true
        receiving = false
        assertFalse(sync.receive(Msg.Clip("2", "b", clock)))
        assertTrue(board.writes.isEmpty())
    }

    @Test fun `empty and oversized incoming clips are dropped`() {
        assertFalse(sync.receive(Msg.Clip("1", "", clock)))
        assertFalse(sync.receive(Msg.Clip("2", "a".repeat(ClipSync.MAX_BYTES + 1), clock)))
        assertTrue(sync.receive(Msg.Clip("3", "a".repeat(ClipSync.MAX_BYTES), clock)))
        assertEquals(1, board.writes.size)
    }

    // ---- loop prevention ----

    @Test fun `a text just received is not sent back`() {
        sync.receive(Msg.Clip("1", "from pc", clock))
        val out = Outbox(setOf("a"))
        clock += 2_000
        assertEquals(ClipSendResult.AlreadyThere, sync.send("from pc", listOf("a"), out::send))
        assertTrue(out.sent.isEmpty())
    }

    @Test fun `the PC echoing our text right back is not applied`() {
        sync.send("mine", listOf("a"), Outbox(setOf("a"))::send)
        clock += 1_000
        assertFalse(sync.receive(Msg.Clip("echo", "mine", clock)))
        assertTrue(board.writes.isEmpty())
    }

    @Test fun `the same text a while later is a new copy, not an echo`() {
        sync.receive(Msg.Clip("1", "again", clock))
        clock += ClipSync.ECHO_WINDOW_MS + 1
        val out = Outbox(setOf("a"))
        assertEquals(ClipSendResult.Sent(listOf("a")), sync.send("again", listOf("a"), out::send))
    }

    @Test fun `a different text is never blocked by the guard`() {
        sync.receive(Msg.Clip("1", "one", clock))
        assertEquals(ClipSendResult.Sent(listOf("a")), sync.send("two", listOf("a"), Outbox(setOf("a"))::send))
        assertTrue(sync.receive(Msg.Clip("3", "three", clock)))
    }

    @Test fun `the clipboard is read through the abstraction`() {
        board.text = "copied"
        assertEquals("copied", sync.readClipboard())
        board.text = null
        assertNull(sync.readClipboard())
    }
}
