package app.pill.phone

import app.pill.phone.net.RoutePolicy
import app.pill.phone.protocol.AuthFailGuard
import app.pill.phone.protocol.Codec
import app.pill.phone.protocol.FrameCrypto
import app.pill.phone.protocol.Msg
import app.pill.phone.protocol.PairRequest
import app.pill.phone.protocol.PairResponse
import app.pill.phone.protocol.RelayBlock
import app.pill.phone.protocol.RelayUrl
import app.pill.phone.protocol.V2
import app.pill.phone.transfer.ChunkFlow
import app.pill.phone.transfer.ChunkReceiver
import app.pill.phone.transfer.SendWindow
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Protocol v2 (docs/phone-relay.md). The fixed-key vector below was produced by an independent implementation
 * (Python `cryptography`: X25519, HKDF, HMAC; XChaCha20-Poly1305 built from HChaCha20 + ChaCha20-Poly1305, whose HChaCha20
 * was checked against the draft-irtf-cfrg-xchacha test vector) from the secrets 01..20 (phone) and 65..84 (PC), hex.
 */
class RelayTest {
    private fun hex(s: String) = s.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    private fun ByteArray.hex() = joinToString("") { "%02x".format(it) }

    private val phoneSk = hex("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20")
    private val pcSk = hex("65666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081828384")
    private val phonePk = "B6N8vBQgk8i3VdwbEOhstCY3StFqqFPtC9_AsrhtHHw"
    private val pcPk = "VxR2nRFr92Q2rnS8eT0sMK0ZA8WaxSc4BcfiaYtBDDY"
    private val code = "AB3D9XKQ"
    private val deviceId = "0b6f2f6e-6c7e-4c53-9d2a-1d0c5b7a9e11"
    private val kP2c = "6f29aa994173633730fd4e75b68768a37fd7a566053ee8b9eb547892d25121af"
    private val kC2p = "e305543817a96cab46c969005eabfeb66aa15dd70bcd93a3f31beb9ea743b726"
    private val room = "2fdc4b4165a4e9db4e0f4cf160d6d9e3"
    private val join = "NsHWn-LIc5AIRoIZMQEdcxf-AmAm9IdoAFk89Q6YKJY"
    private val framePcToPhone = "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b74b5e5c8e21e8ba96a44f519ba3d061edd5f3647fb9b3930235a33f00b36717d055836d45e5ff3a68a141700eea3f1ca489d8a7ee22a0d6a54cd1d2890388aa9196673df1405638ae86dbf8c917786aa1ff6f810006275a3dea6b165d06d0a2ce70417ef0984529d34fa0d3fa5d71a4ff9e4de3309e246fb72b505748aebd21a2"
    private val framePhoneToPc = "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b789056473c7a389b1db80a44d36411304040928e6c57ea0e40e475b3d740962081779a35e543d48cde87dcdde576e3aaa0126dbf10ae6d0b796da57c3f67332161e5f95"

    private fun phoneSide(sid: String = "1111111111111111") = FrameCrypto(room, hex(kP2c), hex(kC2p), "p2c", "c2p", sid)
    private fun pcSide(sid: String = "2222222222222222") = FrameCrypto(room, hex(kC2p), hex(kP2c), "c2p", "p2c", sid)

    // ---- keys ----

    @Test fun publicKeysMatchTheVector() {
        assertEquals(phonePk, V2.b64u(com.google.crypto.tink.subtle.X25519.publicFromPrivate(phoneSk)))
        assertEquals(pcPk, V2.b64u(com.google.crypto.tink.subtle.X25519.publicFromPrivate(pcSk)))
    }

    @Test fun phoneAndPcDeriveTheSameKeysAsTheVector() {
        val phone = V2.derive(phoneSk, V2.unb64u(pcPk), code, deviceId)
        val pc = V2.derive(pcSk, V2.unb64u(phonePk), code, deviceId)
        assertEquals(kP2c, phone.kP2c.hex())
        assertEquals(kC2p, phone.kC2p.hex())
        assertEquals(room, phone.room)
        assertEquals(join, phone.join)
        assertEquals(phone.kP2c.hex(), pc.kP2c.hex())
        assertEquals(phone.join, pc.join)
    }

    @Test fun differentCodeOrDeviceGivesDifferentKeys() {
        val a = V2.derive(phoneSk, V2.unb64u(pcPk), "AB3D9XKR", deviceId)
        val b = V2.derive(phoneSk, V2.unb64u(pcPk), code, "other-device")
        assertTrue(a.kP2c.hex() != kP2c && b.kP2c.hex() != kP2c && a.room != room && b.room != room)
    }

    private fun block(r: String = room, j: String = join, url: String = "wss://pill-relay.example.workers.dev") = RelayBlock(url, r, j)

    @Test fun pairingAcceptsAMatchingBlockAndKeepsTheKeys() {
        val r = V2.completePairing(phoneSk, pcPk, block(), code, deviceId)
        assertTrue(r is V2.Pairing.Ok)
        val cfg = (r as V2.Pairing.Ok).cfg
        assertEquals(room, cfg.room)
        assertEquals(join, cfg.join)
        assertEquals(kP2c, V2.unb64u(cfg.kP2c).hex())
        assertEquals(kC2p, V2.unb64u(cfg.kC2p).hex())
        assertEquals(phoneSk.hex(), V2.unb64u(cfg.secret).hex())
    }

    @Test fun pairingFailsOnAnyMismatch() {
        assertEquals(V2.Pairing.Mismatch, V2.completePairing(phoneSk, pcPk, block(r = "0".repeat(32)), code, deviceId))
        assertEquals(V2.Pairing.Mismatch, V2.completePairing(phoneSk, pcPk, block(j = V2.b64u(ByteArray(32))), code, deviceId))
        assertEquals(V2.Pairing.Mismatch, V2.completePairing(phoneSk, pcPk, block(), "ZZZZZZZZ", deviceId))
        assertEquals(V2.Pairing.Mismatch, V2.completePairing(phoneSk, "not base64 !!", block(), code, deviceId))
        assertEquals(V2.Pairing.Mismatch, V2.completePairing(phoneSk, V2.b64u(ByteArray(31)), block(), code, deviceId))
        // an all-zero public key is a low-order point: no shared secret
        assertEquals(V2.Pairing.Mismatch, V2.completePairing(phoneSk, V2.b64u(ByteArray(32)), block(), code, deviceId))
    }

    @Test fun anOldPcMeansLanOnly() {
        assertEquals(V2.Pairing.NotOffered, V2.completePairing(phoneSk, null, null, code, deviceId))
        assertEquals(V2.Pairing.NotOffered, V2.completePairing(phoneSk, pcPk, null, code, deviceId))   // PC without a relay configured
        assertEquals(V2.Pairing.NotOffered, V2.completePairing(phoneSk, null, block(), code, deviceId))
        assertEquals(V2.Pairing.NotOffered, V2.completePairing(phoneSk, pcPk, block(url = "ws://insecure.example"), code, deviceId))
    }

    @Test fun pairWireFormat() {
        val req = Codec.json.encodeToString(PairRequest.serializer(), PairRequest(code, deviceId, "Pixel", "Pixel", "Android 15", "1.0.0", phonePk))
        assertTrue(req.contains("\"pk\":\"$phonePk\""))
        val old = Codec.json.encodeToString(PairRequest.serializer(), PairRequest(code, deviceId, "Pixel", "Pixel", "Android 15", "1.0.0"))
        assertFalse(old.contains("pk"))
        val resp = Codec.json.decodeFromString(
            PairResponse.serializer(),
            """{"token":"t","pcName":"PC","serverVersion":2,"pk":"$pcPk","relay":{"url":"wss://x.dev","room":"$room","join":"$join"},"extra":1}""",
        )
        assertEquals(pcPk, resp.pk)
        assertEquals(room, resp.relay?.room)
        val v1 = Codec.json.decodeFromString(PairResponse.serializer(), """{"token":"t","pcName":"PC","serverVersion":1}""")
        assertNull(v1.pk)
        assertNull(v1.relay)
    }

    @Test fun relayUrl() {
        assertEquals("wss://w.dev/r/$room?role=phone", RelayUrl.build("wss://w.dev", room))
        assertEquals("wss://w.dev/r/$room?role=phone", RelayUrl.build("https://w.dev/", room))
        assertEquals("wss://w.dev/r/$room?role=phone", RelayUrl.build("wss://w.dev/r/$room", room))
        assertNull(RelayUrl.build("ws://w.dev", room))
        assertNull(RelayUrl.build("", room))
    }

    // ---- frames ----

    @Test fun decryptsTheVectorFrameFromThePc() {
        val r = phoneSide().open(hex(framePcToPhone))
        assertTrue(r is FrameCrypto.Opened.Ok)
        val m = Codec.decode((r as FrameCrypto.Opened.Ok).json)
        assertEquals(Msg.Hello("DESKTOP-X", 1, 1_700_000_000_000), m)
    }

    @Test fun pcSideDecryptsTheVectorFrameFromThePhone() {
        val r = pcSide().open(hex(framePhoneToPc))
        assertEquals("""{"t":"ping"}""", (r as FrameCrypto.Opened.Ok).json)
    }

    @Test fun roundTripBothWaysWithGrowingSeq() {
        val phone = phoneSide()
        val pc = pcSide()
        repeat(3) { i ->
            val f = phone.seal(Codec.encode(Msg.NotifRemoved("k$i")))!!
            assertEquals(Msg.NotifRemoved("k$i"), Codec.decode((pc.open(f) as FrameCrypto.Opened.Ok).json))
        }
        val back = pc.seal(Codec.encode(Msg.Find))!!
        assertEquals(Msg.Find, Codec.decode((phone.open(back) as FrameCrypto.Opened.Ok).json))
        // a frame is nonce(24) || ciphertext || tag(16): fresh random nonce each time
        val a = phone.seal("{\"t\":\"ping\"}")!!
        val b = phone.seal("{\"t\":\"ping\"}")!!
        assertEquals(24 + """{"sid":"1111111111111111","seq":4,"m":{"t":"ping"}}""".length + 16, a.size)
        assertTrue(!a.copyOfRange(0, 24).contentEquals(b.copyOfRange(0, 24)))
    }

    @Test fun envelopeCarriesSidAndSeqFromOne() {
        val pc = pcSide()
        val first = phoneSide("abcdabcdabcdabcd").seal("{\"t\":\"ping\"}")!!
        // open with a PC that sees it, then check the plaintext through a known-key decryptor
        val plain = com.google.crypto.tink.subtle.XChaCha20Poly1305(hex(kP2c)).decrypt(first, "pill-v2${room}p2c".toByteArray())
        assertEquals("""{"sid":"abcdabcdabcdabcd","seq":1,"m":{"t":"ping"}}""", String(plain))
        assertTrue(pc.open(first) is FrameCrypto.Opened.Ok)
    }

    @Test fun replayAndReorderAreRejected() {
        val phone = phoneSide()
        val pc = pcSide()
        val f1 = phone.seal("{\"t\":\"ping\"}")!!
        val f2 = phone.seal("{\"t\":\"ping\"}")!!
        val f3 = phone.seal("{\"t\":\"ping\"}")!!
        assertTrue(pc.open(f1) is FrameCrypto.Opened.Ok)
        assertEquals(FrameCrypto.Opened.Replay, pc.open(f1))      // the same frame again
        assertTrue(pc.open(f3) is FrameCrypto.Opened.Ok)          // a gap is fine (the relay may drop frames)
        assertEquals(FrameCrypto.Opened.Replay, pc.open(f2))      // older than the last accepted
        // a reconnect of the phone is a new sid and starts at seq 1 again
        val again = phoneSide("9999999999999999").seal("{\"t\":\"ping\"}")!!
        assertTrue(pc.open(again) is FrameCrypto.Opened.Ok)
        assertEquals(FrameCrypto.Opened.Replay, pc.open(again))
    }

    @Test fun tamperedOrForeignFramesAreInvalid() {
        val phone = phoneSide()
        val pc = pcSide()
        val f = phone.seal("{\"t\":\"ping\"}")!!
        for (i in listOf(0, 10, 23, 24, 30, f.size - 1)) {
            val bad = f.copyOf().also { it[i] = (it[i].toInt() xor 1).toByte() }
            assertEquals("byte $i", FrameCrypto.Opened.Invalid, pc.open(bad))
        }
        assertEquals(FrameCrypto.Opened.Invalid, pc.open(f.copyOf(f.size - 1)))
        assertEquals(FrameCrypto.Opened.Invalid, pc.open(ByteArray(10)))
        // the direction is part of the AAD: a p2c frame does not open as c2p, nor in another room
        assertEquals(FrameCrypto.Opened.Invalid, phoneSide().open(f))
        assertEquals(FrameCrypto.Opened.Invalid, FrameCrypto("f".repeat(32), hex(kC2p), hex(kP2c), "c2p", "p2c").open(f))
        // wrong key
        assertEquals(FrameCrypto.Opened.Invalid, FrameCrypto(room, hex(kC2p), ByteArray(32), "c2p", "p2c").open(f))
        // the untouched frame still opens: failures did not poison the replay state
        assertTrue(pc.open(f) is FrameCrypto.Opened.Ok)
    }

    @Test fun authenticButMalformedEnvelopeIsInvalid() {
        val aead = com.google.crypto.tink.subtle.XChaCha20Poly1305(hex(kP2c))
        val aad = "pill-v2${room}p2c".toByteArray()
        for (env in listOf("not json", "{}", """{"sid":"a","seq":"1","m":{}}""", """{"sid":"a","seq":1}""", """{"sid":"a","m":{}}""")) {
            assertEquals(env, FrameCrypto.Opened.Invalid, pcSide().open(aead.encrypt(env.toByteArray(), aad)))
        }
    }

    @Test fun oversizeMessageIsRefusedAndDoesNotBurnASeq() {
        val phone = phoneSide()
        assertNull(phone.seal("{\"t\":\"x\",\"d\":\"" + "a".repeat(FrameCrypto.MAX_PLAIN) + "\"}"))
        val f = phone.seal("{\"t\":\"ping\"}")!!
        assertTrue(pcSide().open(f) is FrameCrypto.Opened.Ok)   // seq 1 was still free
        // a file chunk at the protocol's size fits comfortably
        val chunk = Msg.FileChunk("id", 0, ChunkFlow.encode(ByteArray(ChunkFlow.CHUNK), ChunkFlow.CHUNK))
        assertNotNull(phone.seal(Codec.encode(chunk)))
    }

    @Test fun authFailuresCloseTheConnectionAtFiftyInAMinute() {
        val g = AuthFailGuard()
        for (i in 0 until 49) assertFalse(g.record(1_000L + i))
        assertTrue(g.record(1_500))
        val slow = AuthFailGuard()
        // 200 failures, one every 2 s, are 30 per minute: never 50
        for (i in 0 until 200) assertFalse(slow.record(i * 2_000L))
    }

    // ---- route policy, driven by a fake clock ----

    @Test fun lanFirstWithAShortTimeoutOnlyWhenTheRelayIsTheFallback() {
        var usable = true
        val p = RoutePolicy { usable }
        assertEquals(2_000L, p.lanTimeoutMs())
        assertEquals(RoutePolicy.Step.TryRelay, p.afterLanFailed())
        usable = false
        assertEquals(6_000L, p.lanTimeoutMs())
        assertEquals(RoutePolicy.Step.GiveUp, p.afterLanFailed())
    }

    @Test fun onTheRelayTheLanIsLookedAtEverySixtySecondsAndOnRequest() {
        val p = RoutePolicy { true }
        var now = 10_000L
        p.onRelayUp(now)
        assertEquals(60_000L, p.probeInMs(now, false))
        now += 59_999
        assertFalse(p.shouldProbe(now, false, false))
        assertEquals(1L, p.probeInMs(now, false))
        now += 1
        assertTrue(p.shouldProbe(now, false, false))
        p.onProbed(now)                                  // the LAN did not answer
        assertFalse(p.shouldProbe(now + 30_000, false, false))
        assertTrue(p.shouldProbe(now + 30_000, false, true))   // the reconnect button / network back: right now
        assertTrue(p.shouldProbe(now + 60_000, false, false))
    }

    @Test fun aFileInFlightKeepsThePhoneOnTheRelay() {
        val p = RoutePolicy { true }
        p.onRelayUp(0)
        assertFalse(p.shouldProbe(500_000, relayBusy = true, nudged = true))
        assertEquals(RoutePolicy.BUSY_RECHECK_MS, p.probeInMs(500_000, true))
        assertTrue(p.shouldProbe(500_000, relayBusy = false, nudged = false))
    }

    @Test fun backOffRunsFromOneToThirtySeconds() {
        val p = RoutePolicy { true }
        assertEquals(listOf(1_000L, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000), (1..7).map { p.retryDelayMs(it) })
    }

    @Test fun aWholeTripAwayAndBack() {
        // scripted day: leave home, LAN times out, relay serves, a probe finds home Wi-Fi, back on the LAN
        var now = 0L
        val p = RoutePolicy { true }
        val log = mutableListOf<String>()
        log += "lan:${p.lanTimeoutMs()}"; now += p.lanTimeoutMs()
        if (p.afterLanFailed() == RoutePolicy.Step.TryRelay) { log += "relay"; p.onRelayUp(now) }
        val lanBackAt = 130_000L
        while (true) {
            now += p.probeInMs(now, false).coerceAtLeast(1)
            if (p.shouldProbe(now, false, false)) {
                p.onProbed(now)
                log += "probe@$now"
                if (now >= lanBackAt) break
            }
        }
        log += "lan"
        assertEquals(listOf("lan:2000", "relay", "probe@62000", "probe@122000", "probe@182000", "lan"), log)
    }

    // ---- file flow control ----

    @Test fun senderKeepsAtMostEightChunksUnacked() {
        val w = SendWindow()
        var sent = 0L
        var n = 0
        while (w.canSend()) { sent += ChunkFlow.CHUNK; w.onSent(sent); n++ }
        assertEquals(8, n)
        assertEquals(8, w.inFlight)
        w.onAck(ChunkFlow.CHUNK.toLong() * 3)             // cumulative: three chunks are covered
        assertEquals(5, w.inFlight)
        assertTrue(w.canSend())
        w.onAck(ChunkFlow.CHUNK.toLong())                 // stale ack changes nothing
        assertEquals(5, w.inFlight)
        w.onAck(ChunkFlow.CHUNK.toLong() * 3 + 5)         // half a chunk more does not free one
        assertEquals(5, w.inFlight)
        w.onAck(sent)
        assertEquals(0, w.inFlight)
        assertEquals(sent, w.acked)
    }

    @Test fun receiverNeedsContiguousChunksAndTheAnnouncedSize() {
        val rx = ChunkReceiver(250)
        assertEquals(ChunkReceiver.Result.Ok, rx.accept(0, 100))
        assertEquals(ChunkReceiver.Result.BadOffset, rx.accept(200, 50))   // a chunk was lost
        assertEquals(ChunkReceiver.Result.BadOffset, rx.accept(50, 50))    // or repeated
        assertEquals(ChunkReceiver.Result.Ok, rx.accept(100, 100))
        assertEquals(ChunkReceiver.Result.TooMuch, rx.accept(200, 51))
        assertFalse(rx.complete(250))                                      // 50 bytes still missing
        assertEquals(ChunkReceiver.Result.Ok, rx.accept(200, 50))
        assertFalse(rx.complete(249))
        assertTrue(rx.complete(250))
        assertEquals(250L, rx.received)
    }

    @Test fun chunkBase64RoundTripsAndPacingStaysUnderTheRelayLimit() {
        val data = ByteArray(1000) { it.toByte() }
        assertArrayEquals(data, ChunkFlow.decode(ChunkFlow.encode(data, data.size)))
        assertArrayEquals(data.copyOf(10), ChunkFlow.decode(ChunkFlow.encode(data, 10)))
        assertNull(ChunkFlow.decode("***"))
        // one chunk every paceDelay: 4 MiB/s is the relay's ceiling for the whole room
        val perSec = 1000.0 / ChunkFlow.paceDelayMs(ChunkFlow.CHUNK) * ChunkFlow.CHUNK
        assertTrue(perSec < 4 * 1024 * 1024)
        assertTrue(1000.0 / ChunkFlow.paceDelayMs(ChunkFlow.CHUNK) < 60)
    }

    @Test fun fileMessagesRoundTrip() {
        for (m in listOf<Msg>(
            Msg.FileChunk("id1", 98304, "AAEC"), Msg.FileAck("id1", 196608), Msg.FileEnd("id1", 300000),
            Msg.FileAbort("id1", "too_big"), Msg.FileAbort("id1"), Msg.FileAccept("id1"), Msg.FileDone("id1", true),
            Msg.Hello("PC", 2, 5, relayMaxFileMB = 250),
        )) assertEquals(m, Codec.decode(Codec.encode(m)))
        assertEquals("""{"t":"file.chunk","id":"a","off":0,"d":"AA=="}""", Codec.encode(Msg.FileChunk("a", 0, "AA==")))
        assertEquals("""{"t":"file.ack","id":"a","upto":5}""", Codec.encode(Msg.FileAck("a", 5)))
        // an old PC's hello has no cap: null, the app keeps its default
        assertNull((Codec.decode("""{"t":"hello","pcName":"PC","serverVersion":1,"time":1}""") as Msg.Hello).relayMaxFileMB)
    }
}
