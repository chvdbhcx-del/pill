package app.pill.phone

import app.pill.phone.protocol.Backoff
import app.pill.phone.protocol.Codec
import app.pill.phone.protocol.FileNames
import app.pill.phone.protocol.Msg
import app.pill.phone.protocol.PairLink
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtocolTest {

    private fun roundTrip(m: Msg) = assertEquals(m, Codec.decode(Codec.encode(m)))
    private fun obj(m: Msg) = Codec.json.parseToJsonElement(Codec.encode(m)) as JsonObject

    @Test fun everyMessageTypeRoundTrips() {
        val posted = Msg.NotifPosted(
            key = "0|org.telegram.messenger|1|null|10", pkg = "org.telegram.messenger", app = "Telegram",
            title = "Вадим", text = "привет, как дела? \"ok\"\nline2", sub = "", whenMs = 1_700_000_000_000,
            category = "msg", ongoing = false, progress = null, icon = "AAAA", avatar = null, canReply = true,
            actions = listOf(Msg.NotifPosted.Action("a1", "Mark as read")), group = "g",
        )
        listOf(
            Msg.Ping, Msg.Pong, Msg.Find,
            Msg.Hello("DESKTOP-X", 1, 123L),
            posted, posted.copy(replay = true, progress = 40, ongoing = true),
            Msg.NotifRemoved("k"), Msg.NotifDismiss("k"), Msg.NotifRead("k"),
            Msg.NotifReply("r1", "k", "хорошо"),
            Msg.Status(87, false, "wifi", "Home", true, 5L), Msg.Status(5, true, "none", null, false, 6L),
            Msg.ReplyResult("r1", true), Msg.ReplyResult("r1", false, "expired"),
            Msg.FileOffer("id", "photo.jpg", 123456, "image/jpeg"),
            Msg.FileAccept("id"), Msg.FileReject("id", "too_big"), Msg.FileDone("id", true),
        ).forEach(::roundTrip)
    }

    @Test fun wireFormatMatchesTheContract() {
        val o = obj(Msg.NotifPosted("k", "p", "A", "T", "x", "", 7L, "msg", false, null, null, null, false, emptyList(), null))
        assertEquals("notif.posted", o["t"]!!.jsonPrimitive.content)
        assertEquals("7", o["when"]!!.jsonPrimitive.content)
        // nulls are explicit, replay is absent unless true
        assertEquals(JsonNull, o["progress"]); assertEquals(JsonNull, o["icon"]); assertEquals(JsonNull, o["group"])
        assertFalse(o.containsKey("replay"))
        assertTrue(obj(Msg.NotifPosted("k", "p", "A", "T", "x", "", 7L, "msg", false, null, null, null, false, emptyList(), null, replay = true))["replay"]!!.jsonPrimitive.boolean)
        assertEquals("""{"t":"ping"}""", Codec.encode(Msg.Ping))
        assertEquals("""{"t":"reply.result","req":"r","ok":true}""", Codec.encode(Msg.ReplyResult("r", true)))
        assertEquals("""{"t":"file.done","id":"i","ok":true}""", Codec.encode(Msg.FileDone("i", true)))
    }

    private val rich = Msg.Status(
        87, true, "wifi", "Home", true, 5L,
        charge = "charging", plug = "usb", tempC = 31.5, health = "good", fullInMin = 42,
        storage = Msg.Status.Space(52_000_000_000, 128_000_000_000), ram = Msg.Status.Space(3100, 8000),
        net = Msg.Status.NetInfo("wifi", mbps = 866, bars = 3), airplane = false, ringer = "vibrate", screen = true,
        tz = "Europe/Moscow", utcMin = 180,
    )

    @Test fun theRichStatusRoundTripsAndKeepsTheOldWireFormat() {
        roundTrip(rich)
        roundTrip(rich.copy(net = Msg.Status.NetInfo("mobile", carrier = "Beeline", gen = "5G", bars = 2), plug = null, fullInMin = null))
        // a phone that knows nothing extra sends exactly what protocol v1 says: no new keys at all
        assertEquals(
            """{"t":"status","battery":87,"charging":false,"network":"wifi","ssid":"Home","dnd":false,"time":5}""",
            Codec.encode(Msg.Status(87, false, "wifi", "Home", false, 5L)),
        )
        val o = obj(rich)
        assertEquals("charging", o["charge"]!!.jsonPrimitive.content)
        assertEquals("31.5", o["tempC"]!!.jsonPrimitive.content)
        assertEquals("""{"free":52000000000,"total":128000000000}""", o["storage"].toString())
        assertEquals("""{"type":"wifi","mbps":866,"bars":3}""", o["net"].toString())
        assertFalse(obj(rich.copy(plug = null)).containsKey("plug"))
        // ...and an old or a future status still decodes
        val old = Codec.decode("""{"t":"status","battery":5,"charging":true,"network":"none","ssid":null,"dnd":false,"time":1}""") as Msg.Status
        assertNull(old.charge); assertNull(old.storage); assertNull(old.net)
        val future = Codec.decode("""{"t":"status","battery":5,"charging":true,"network":"none","ssid":null,"dnd":false,"time":1,"charge":"full","hologram":{"x":1}}""") as Msg.Status
        assertEquals("full", future.charge)
    }

    @Test fun decodesWhatThePcSends() {
        assertEquals(Msg.Hello("PC", 1, 99), Codec.decode("""{"t":"hello","pcName":"PC","serverVersion":1,"time":99}"""))
        assertEquals(Msg.NotifReply("7", "k", "ok"), Codec.decode("""{"t":"notif.reply","req":"7","key":"k","text":"ok"}"""))
        assertEquals(Msg.FileOffer("u", "Report.docx", 10, "application/x"), Codec.decode("""{"t":"file.offer","id":"u","name":"Report.docx","size":10,"mime":"application/x"}"""))
        assertEquals(Msg.Pong, Codec.decode("""{"t":"pong"}"""))
        // unknown extra keys are ignored
        assertEquals(Msg.NotifDismiss("k"), Codec.decode("""{"t":"notif.dismiss","key":"k","future":1}"""))
    }

    @Test fun garbageIsIgnoredNotFatal() {
        assertNull(Codec.decode("not json"))
        assertNull(Codec.decode("""{"t":"from.the.future"}"""))
        assertNull(Codec.decode("""{"nope":1}"""))
        assertNull(Codec.decode("[]"))
    }

    @Test fun pairingLinkParsing() {
        val a = PairLink.parse("pillphone://pair?h=192.168.1.20&p=41717&c=AB3D9XKQ&n=DESKTOP-X")!!
        assertEquals("192.168.1.20", a.host); assertEquals(41717, a.port); assertEquals("AB3D9XKQ", a.code); assertEquals("DESKTOP-X", a.pcName)

        val enc = PairLink.parse("pillphone://pair?h=10.0.0.5&p=5&c=ab3d9xkq&n=My%20PC%20%D0%94")!!
        assertEquals("My PC Д", enc.pcName); assertEquals("AB3D9XKQ", enc.code)

        val intent = PairLink.parse("intent://pair?h=172.20.1.2&p=41718&c=AB3D9XKQ&n=PC#Intent;scheme=pillphone;package=app.pill.phone;end")!!
        assertEquals("172.20.1.2", intent.host); assertEquals(41718, intent.port)

        val qr = PairLink.parse("http://192.168.0.7:41717/p?c=AB3D9XKQ")!!
        assertEquals("192.168.0.7:41717", qr.address); assertNull(qr.pcName)
    }

    @Test fun pairingLinkRejectsBadInput() {
        assertNull(PairLink.parse(null))
        assertNull(PairLink.parse(""))
        assertNull(PairLink.parse("pillphone://pair?h=8.8.8.8&p=41717&c=AB3D9XKQ"))      // public address
        assertNull(PairLink.parse("pillphone://pair?h=192.168.1.20&p=0&c=AB3D9XKQ"))     // port
        assertNull(PairLink.parse("pillphone://pair?h=192.168.1.20&p=70000&c=AB3D9XKQ"))
        assertNull(PairLink.parse("pillphone://pair?h=192.168.1.20&p=41717&c=SHORT"))    // code
        assertNull(PairLink.parse("pillphone://pair?h=192.168.1.20&p=41717&c=AB3D9XK0")) // 0/1/O are not in the alphabet
        assertNull(PairLink.parse("pillphone://pair?p=41717&c=AB3D9XKQ"))                // no host
        assertNull(PairLink.parse("http://192.168.0.7:41717/other?c=AB3D9XKQ"))
        assertNull(PairLink.parse("https://example.com/p?c=AB3D9XKQ"))
        assertNull(PairLink.parse("ftp://192.168.0.7:1/p?c=AB3D9XKQ"))
    }

    @Test fun lanHostRules() {
        listOf("10.1.2.3", "172.16.0.1", "172.31.255.1", "192.168.0.1", "169.254.3.4", "127.0.0.1", "mypc", "mypc.local").forEach { assertTrue(it, PairLink.isLanHost(it)) }
        listOf("172.32.0.1", "172.15.0.1", "8.8.8.8", "300.1.1.1", "example.com", "bad host", "").forEach { assertFalse(it, PairLink.isLanHost(it)) }
    }

    @Test fun manualEntry() {
        assertEquals("192.168.1.5:41717", PairLink.parseManual("192.168.1.5", "ab3d 9xkq")!!.address)
        assertEquals("192.168.1.5:4000", PairLink.parseManual("http://192.168.1.5:4000/", "AB3D-9XKQ")!!.address)
        assertNull(PairLink.parseManual("192.168.1.5:abc", "AB3D9XKQ"))
        assertNull(PairLink.parseManual("192.168.1.5", "123"))
        assertNull(PairLink.parseManual("", "AB3D9XKQ"))
        val pasted = PairLink.parseManual("http://192.168.0.7:41717/p?c=AB3D9XKQ", "")
        assertNotNull(pasted); assertEquals("AB3D9XKQ", pasted!!.code)
    }

    @Test fun backoffSchedule() {
        assertEquals(listOf(1000L, 1000L, 2000L, 4000L, 8000L, 16000L, 30000L, 30000L, 30000L), (-1..6).map { Backoff.delayMs(it) } + Backoff.delayMs(100))
        var prev = 0L
        for (i in 0..50) { val d = Backoff.delayMs(i); assertTrue(d >= prev && d in 1000..30000); prev = d }
    }

    @Test fun fileNameHandling() {
        assertEquals("passwd", FileNames.sanitize("../../etc/passwd"))
        assertEquals("evil.exe", FileNames.sanitize("C:\\Windows\\evil.exe"))
        assertEquals("a_b_c.txt", FileNames.sanitize("a:b*c.txt"))
        assertEquals("hidden", FileNames.sanitize(".hidden"))
        assertEquals("file", FileNames.sanitize("..."))
        assertEquals("file", FileNames.sanitize(null))
        assertEquals("_CON.txt", FileNames.sanitize("CON.txt"))
        assertEquals("Отчёт 2026.docx", FileNames.sanitize("Отчёт 2026.docx"))
        val long = FileNames.sanitize("x".repeat(300) + ".jpg")
        assertTrue(long.length <= 120 && long.endsWith(".jpg"))
        assertEquals("photo (2).jpg", FileNames.withSuffix("photo.jpg", 2))
        assertEquals("photo.jpg", FileNames.withSuffix("photo.jpg", 0))
        assertEquals("README (1)", FileNames.withSuffix("README", 1))
        assertEquals("1.5 MB", FileNames.humanSize(1_572_864)); assertEquals("512 B", FileNames.humanSize(512))
        assertEquals("4.0 GB", FileNames.humanSize(4L shl 30))
    }
}
