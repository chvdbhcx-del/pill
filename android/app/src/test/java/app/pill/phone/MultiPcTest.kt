package app.pill.phone

import app.pill.phone.data.FileTarget
import app.pill.phone.data.PairingBook
import app.pill.phone.data.Paired
import app.pill.phone.data.Routing
import app.pill.phone.net.Route
import app.pill.phone.protocol.Msg
import app.pill.phone.protocol.RelayCfg
import app.pill.phone.service.NotifBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The rules of several paired PCs: the list and its migration, who gets which notification, where replies go, who is asked about a file. */
class MultiPcTest {
    private val relay = RelayCfg("wss://r.example", "room", "join", "k1", "k2", "sk", "pk")

    private fun pc(id: String, host: String = "10.0.0.${id.hashCode() and 0x7F}", name: String = "PC-$id", relay: RelayCfg? = null) =
        Paired(id, host, 41717, "tok-$id", name, "dev", relay)

    // ---- the list and the migration ----

    @Test fun `an existing single pairing becomes the first entry with its keys and no re-pairing`() {
        val list = PairingBook.migrate("192.168.1.20", 41717, "tok", "AENEAS", "dev-1", relay, pcId = "p1")
        assertEquals(1, list.size)
        val p = list.single()
        assertEquals("p1", p.pcId)
        assertEquals("192.168.1.20:41717", p.address)
        assertEquals("tok", p.token)
        assertEquals("dev-1", p.deviceId)
        assertEquals(relay, p.relay)
        // settings of the new model start as "follow the global ones"
        assertTrue(p.mirror)
        assertNull(p.autoReceive)
        assertNull(p.filter)
    }

    @Test fun `nothing to migrate when the old store held no pairing`() {
        assertTrue(PairingBook.migrate(null, 0, null, null, "dev", null).isEmpty())
        assertTrue(PairingBook.migrate("1.2.3.4", 5, "", "x", "dev", null).isEmpty())
    }

    @Test fun `the list survives encoding with relay keys and per-PC settings`() {
        val a = pc("a", relay = relay).copy(alias = "Desk", mirror = false, autoReceive = true, filter = mapOf("com.x" to false), lastSeen = 99)
        val b = pc("b")
        assertEquals(listOf(a, b), PairingBook.decode(PairingBook.encode(listOf(a, b))))
        assertTrue(PairingBook.decode("garbage").isEmpty())
        assertTrue(PairingBook.decode(null).isEmpty())
    }

    @Test fun `pairing a new PC adds it, the first one stays`() {
        val a = pc("a", host = "10.0.0.1", name = "AENEAS")
        val (list, written) = PairingBook.upsert(listOf(a), pc("b", host = "10.0.0.2", name = "LAPTOP"))
        assertEquals(listOf("a", "b"), list.map { it.pcId })
        assertEquals("b", written.pcId)
    }

    @Test fun `pairing a PC that is already listed replaces its entry and keeps id and settings`() {
        val a = pc("a", host = "10.0.0.1", name = "AENEAS").copy(alias = "Desk", mirror = false, autoReceive = true, filter = mapOf("p" to true))
        val b = pc("b", host = "10.0.0.2", name = "LAPTOP")
        // same address, new token and keys
        val (byHost, w1) = PairingBook.upsert(listOf(a, b), pc("n1", host = "10.0.0.1", name = "AENEAS").copy(token = "fresh", relay = relay))
        assertEquals(2, byHost.size)
        assertEquals("a", w1.pcId)
        assertEquals("fresh", byHost[0].token)
        assertEquals(relay, byHost[0].relay)
        assertEquals("Desk", byHost[0].alias)
        assertFalse(byHost[0].mirror)
        assertEquals(b, byHost[1])
        // same name on a new address (the PC got another IP)
        val (byName, w2) = PairingBook.upsert(listOf(a, b), pc("n2", host = "10.0.0.9", name = "aeneas"))
        assertEquals(2, byName.size)
        assertEquals("a", w2.pcId)
        assertEquals("10.0.0.9", byName[0].host)
    }

    @Test fun `unpairing one PC leaves the other untouched`() {
        val a = pc("a")
        val b = pc("b")
        assertEquals(listOf(b), PairingBook.remove(listOf(a, b), "a"))
        assertTrue(PairingBook.remove(listOf(a), "a").isEmpty())
        assertEquals(listOf(a, b), PairingBook.remove(listOf(a, b), "zzz"))
    }

    @Test fun `rename changes only the label, not the name the PC reports`() {
        val a = pc("a", name = "AENEAS")
        val renamed = PairingBook.update(listOf(a), "a") { it.copy(alias = "Desk") }.single()
        assertEquals("Desk", renamed.label)
        assertEquals("AENEAS", renamed.pcName)
        assertEquals("AENEAS", renamed.copy(alias = " ").label)
    }

    @Test fun `a changed token or address counts as new credentials, a new name does not`() {
        val a = pc("a")
        assertEquals(a.credentials, a.copy(pcName = "other", alias = "x", mirror = false, lastSeen = 5).credentials)
        assertNotEquals(a.credentials, a.copy(token = "new").credentials)
        assertNotEquals(a.credentials, a.copy(host = "10.9.9.9").credentials)
    }

    // ---- notifications: per-PC switch and filter ----

    private val tg = "org.telegram.messenger"
    private val game = "com.some.game"

    @Test fun `a notification goes to every connected PC that wants it`() {
        val a = pc("a")
        val b = pc("b")
        val t = Routing.mirrorTargets(listOf(a, b), setOf("a", "b"), tg, system = false, global = emptyMap())
        assertEquals(listOf("a", "b"), t.map { it.pcId })
    }

    @Test fun `a PC that is not connected gets nothing`() {
        val t = Routing.mirrorTargets(listOf(pc("a"), pc("b")), setOf("b"), tg, false, emptyMap())
        assertEquals(listOf("b"), t.map { it.pcId })
    }

    @Test fun `the per-PC mirror switch turns one PC off and leaves the other`() {
        val t = Routing.mirrorTargets(listOf(pc("a").copy(mirror = false), pc("b")), setOf("a", "b"), tg, false, emptyMap())
        assertEquals(listOf("b"), t.map { it.pcId })
    }

    @Test fun `a PC without its own filter inherits the global one`() {
        val global = mapOf(game to false)
        val t = Routing.mirrorTargets(listOf(pc("a"), pc("b")), setOf("a", "b"), game, false, global)
        assertTrue(t.isEmpty())
    }

    @Test fun `a PC with its own filter overrides the global one in both directions`() {
        val global = mapOf(game to false, tg to true)
        val a = pc("a").copy(filter = mapOf(game to true))             // allows the game the global filter blocks
        val b = pc("b").copy(filter = mapOf(tg to false, game to false)) // blocks telegram the global filter allows
        val c = pc("c")                                                // inherits
        assertEquals(listOf("a"), Routing.mirrorTargets(listOf(a, b, c), setOf("a", "b", "c"), game, false, global).map { it.pcId })
        assertEquals(listOf("a", "c"), Routing.mirrorTargets(listOf(a, b, c), setOf("a", "b", "c"), tg, false, global).map { it.pcId })
    }

    @Test fun `system apps stay off by default for every PC`() {
        assertTrue(Routing.mirrorTargets(listOf(pc("a")), setOf("a"), "com.android.systemui", true, emptyMap()).isEmpty())
    }

    // ---- sending to several PCs ----

    @Test fun `one PC failing does not keep the others from getting a message`() {
        val got = ArrayList<String>()
        val r = Routing.fanOut(listOf("a", "b", "c")) { id ->
            if (id == "a") throw IllegalStateException("socket closed")
            if (id == "b") false else { got += id; true }
        }
        assertEquals(listOf("c"), got)
        assertEquals(listOf("c"), r.sent)
        assertEquals(listOf("a", "b"), r.failed)
    }

    @Test fun `a protocol message goes through the sender of each PC`() {
        val log = ArrayList<Pair<String, Msg>>()
        Routing.fanOut(listOf("a", "b"), Msg.NotifRemoved("k")) { id, m -> log += id to m; true }
        assertEquals(listOf("a" to Msg.NotifRemoved("k"), "b" to Msg.NotifRemoved("k")), log)
    }

    // ---- replies and requests go back to the PC that sent them ----

    private class FakePhone : NotifBridge.Phone {
        val dismissed = ArrayList<String>()
        val read = ArrayList<String>()
        val replayed = ArrayList<String>()
        var replyError: String? = null
        val replies = ArrayList<Pair<String, String>>()
        override fun dismiss(key: String) { dismissed += key }
        override fun read(key: String) { read += key }
        override fun reply(key: String, text: String): String? { replies += key to text; return replyError }
        override fun replay(pcId: String) { replayed += pcId }
    }

    @Test fun `a reply typed on one PC answers only that PC`() {
        val phone = FakePhone()
        val sent = ArrayList<Pair<String, Msg>>()
        NotifBridge.dispatch("B", Msg.NotifReply("r1", "key1", "ok"), phone) { id, m -> sent += id to m; true }
        assertEquals(listOf("key1" to "ok"), phone.replies)
        assertEquals(listOf<Pair<String, Msg>>("B" to Msg.ReplyResult("r1", true, null)), sent)
    }

    @Test fun `a failed reply reports the error to the asking PC only`() {
        val phone = FakePhone().apply { replyError = "expired" }
        val sent = ArrayList<Pair<String, Msg>>()
        NotifBridge.dispatch("A", Msg.NotifReply("r2", "gone", "hi"), phone) { id, m -> sent += id to m; true }
        assertEquals(listOf<Pair<String, Msg>>("A" to Msg.ReplyResult("r2", false, "expired")), sent)
    }

    @Test fun `a dismiss or mark-read from any PC acts on the phone once and sends nothing back`() {
        val phone = FakePhone()
        val sent = ArrayList<Pair<String, Msg>>()
        NotifBridge.dispatch("A", Msg.NotifDismiss("k1"), phone) { id, m -> sent += id to m; true }
        NotifBridge.dispatch("B", Msg.NotifRead("k2"), phone) { id, m -> sent += id to m; true }
        assertEquals(listOf("k1"), phone.dismissed)
        assertEquals(listOf("k2"), phone.read)
        // the removal is announced by the phone's own listener (to every PC that had it), not by the handler
        assertTrue(sent.isEmpty())
    }

    @Test fun `a hello replays notifications to the PC that connected, not to the others`() {
        val phone = FakePhone()
        NotifBridge.dispatch("B", Msg.Hello(pcName = "LAPTOP"), phone) { _, _ -> true }
        assertEquals(listOf("B"), phone.replayed)
    }

    // ---- which PC the file goes to ----

    @Test fun `no PC paired means nothing to send to`() {
        assertEquals(FileTarget.None, Routing.fileTarget(emptyList(), emptySet()))
    }

    @Test fun `exactly one PC is never asked about, even when it is offline`() {
        val a = pc("a")
        assertEquals(FileTarget.Single("a"), Routing.fileTarget(listOf(a), setOf("a")))
        assertEquals(FileTarget.Single("a"), Routing.fileTarget(listOf(a), emptySet()))
    }

    @Test fun `two PCs always ask, listing each with its state and route`() {
        val a = pc("a", name = "AENEAS")
        val b = pc("b", name = "LAPTOP")
        val t = Routing.fileTarget(listOf(a, b), setOf("a", "b"), mapOf("a" to Route.Lan, "b" to Route.Relay)) as FileTarget.Ask
        assertEquals(listOf("AENEAS", "LAPTOP"), t.options.map { it.name })
        assertEquals(listOf(Route.Lan, Route.Relay), t.options.map { it.route })
        assertTrue(t.options.all { it.online })
    }

    @Test fun `an offline PC is listed but marked offline, and all-offline is detectable`() {
        val a = pc("a")
        val b = pc("b")
        val t = Routing.fileTarget(listOf(a, b), setOf("b")) as FileTarget.Ask
        assertEquals(listOf(false, true), t.options.map { it.online })
        assertTrue(t.anyOnline)
        assertFalse((Routing.fileTarget(listOf(a, b), emptySet()) as FileTarget.Ask).anyOnline)
    }

    // ---- the hero and the foreground notification ----

    @Test fun `the hero follows the picked PC, else the first one that is up, else the first`() {
        val a = pc("a")
        val b = pc("b")
        assertEquals("b", Routing.selected(listOf(a, b), "b", emptySet())?.pcId)
        assertEquals("b", Routing.selected(listOf(a, b), null, setOf("b"))?.pcId)
        assertEquals("b", Routing.selected(listOf(a, b), "gone", setOf("b"))?.pcId)
        assertEquals("a", Routing.selected(listOf(a, b), null, emptySet())?.pcId)
        assertNull(Routing.selected(emptyList(), "a", emptySet()))
    }

    @Test fun `the headline counts the reachable PCs`() {
        assertEquals(Routing.Summary.All(2), Routing.summary(2, 2))
        assertEquals(Routing.Summary.Some(1, 2), Routing.summary(1, 2))
        assertEquals(Routing.Summary.None, Routing.summary(0, 2))
    }
}
