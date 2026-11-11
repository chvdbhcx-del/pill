package app.pill.phone.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import app.pill.phone.data.Paired
import app.pill.phone.data.Store
import app.pill.phone.protocol.AuthFailGuard
import app.pill.phone.protocol.Codec
import app.pill.phone.protocol.FrameCrypto
import app.pill.phone.protocol.Msg
import app.pill.phone.protocol.Proto
import app.pill.phone.protocol.RelayCfg
import app.pill.phone.protocol.RelayUrl
import app.pill.phone.protocol.V2
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.net.URLEncoder
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

sealed interface LinkState {
    data object Unpaired : LinkState
    data object Connecting : LinkState
    /** [route] is the way the PC is reached right now: its home network or the internet relay. */
    data class Connected(val pcName: String, val route: Route = Route.Lan) : LinkState
    data object NoNetwork : LinkState
    /** Network is up but the PC does not answer: wrong network, PC off, Pill closed. */
    data object NotFound : LinkState
    /** The relay is reachable and the phone is in its room, but the PC is not: it is off or Pill is closed. The phone reconnects the moment it starts. */
    data object PcOffline : LinkState
    data object Revoked : LinkState
}

/** The internet link as the settings and the detail rows describe it. */
enum class RelayState {
    /** Paired with a PC that cannot do it (old Pill, or no relay configured on it): LAN only. */
    Unavailable,
    /** The user switched "Use internet relay" off. */
    Disabled,
    /** Ready, and not needed: the phone is on the home network (or between attempts). */
    Standby,
    Connecting,
    /** Joined the room, the PC is not there. */
    WaitingForPc,
    /** The traffic goes through the relay right now. */
    Active,
    /** The relay rejected our join secret: pair again. */
    Refused;

    companion object {
        /** The dynamic state folded with whether this pairing has keys and the setting is on. */
        fun effective(p: Paired?, enabled: Boolean, dyn: RelayState): RelayState = when {
            p?.relay == null -> Unavailable
            !enabled -> Disabled
            else -> dyn
        }
    }
}

/** A frame from one PC, tagged with where it came from so replies and results go back to the same PC. */
data class Incoming(val pcId: String, val msg: Msg)

/**
 * The connection to ONE paired PC (protocol "Realtime channel"), over the LAN or, away from home, over the relay
 * (docs/phone-relay.md). [LinkManager] owns one of these per PC; each makes its own route decision and keeps its own
 * back-off, so one PC can be on the home network while another is on the internet. Everything else only reads [state] and
 * calls [send], and does not care which route carries it.
 */
class PcLink(
    val pcId: String,
    private val app: Context,
    private val emit: (Incoming) -> Unit,
    private val onRevoked: (Paired) -> Unit,
) {
    private val _state = MutableStateFlow<LinkState>(LinkState.Connecting)
    val state: StateFlow<LinkState> = _state.asStateFlow()

    private val _relayDyn = MutableStateFlow(RelayState.Standby)
    /** The relay's dynamic state, before it is folded with the pairing's keys and the setting. */
    val relayDyn: StateFlow<RelayState> = _relayDyn.asStateFlow()

    private val _relayMaxMb = MutableStateFlow(DEFAULT_RELAY_MAX_MB)
    /** Size cap of a file through the relay, from the PC's hello (default 100 MB). */
    val relayMaxFileMB: StateFlow<Int> = _relayMaxMb.asStateFlow()

    private val nudges = Channel<Unit>(Channel.CONFLATED)
    @Volatile private var out: Conn? = null
    private val relayHolds = AtomicInteger()
    private val policy = RoutePolicy { relayConfig(Store.pc(pcId)) != null }

    /** The current pairing of this PC; null once it is gone. */
    private val paired: Paired? get() = Store.pc(pcId)

    /** Try again now instead of waiting out the back-off (button, network back); on the relay it also looks at the LAN. */
    fun nudge() { nudges.trySend(Unit) }

    /** The default network changed (or the credentials did): the open socket is probably dead, do not wait for the watchdog. The next attempt starts at the LAN again. */
    fun networkChanged() {
        out?.ws?.cancel()
        nudge()
    }

    fun send(msg: Msg): Boolean {
        val c = out ?: return false
        if (_state.value !is LinkState.Connected) return false
        return c.sendText(Codec.encode(msg))
    }

    val isConnected: Boolean get() = _state.value is LinkState.Connected

    /** The route carrying the traffic, null while not connected. */
    val route: Route? get() = (_state.value as? LinkState.Connected)?.route

    /** While [block] runs the phone stays on the relay even if the home network turns up (a file is moving). */
    suspend fun <T> holdRelay(block: suspend () -> T): T {
        relayHolds.incrementAndGet()
        try { return block() } finally { relayHolds.decrementAndGet() }
    }

    private fun relayConfig(p: Paired?): RelayCfg? = p?.relay?.takeIf { Store.relayEnabled.value && RelayUrl.build(it.url, it.room) != null }

    private fun now() = System.currentTimeMillis()

    /** Runs until cancelled (the PC is unpaired or the service stops). */
    suspend fun run() = coroutineScope {
        // "Use internet relay" switched while running: leave the relay at once, or try it now.
        val watcher = launch {
            Store.relayEnabled.drop(1).collect { on ->
                if (!on) out?.takeIf { it.route == Route.Relay }?.ws?.cancel()
                nudge()
            }
        }
        try {
            loop()
        } finally {
            watcher.cancel()
            out?.ws?.cancel()
            out = null
        }
    }

    private suspend fun loop() {
        var failures = 0
        while (currentCoroutineContext().isActive) {
            val p = paired ?: return   // unpaired: the manager is about to cancel us anyway
            if (!Net.isUp(app)) {
                _state.value = LinkState.NoNetwork
                withTimeoutOrNull(30_000) { nudges.receive() }
                continue
            }
            _state.value = if (failures >= NOT_FOUND_AFTER) LinkState.NotFound else LinkState.Connecting
            _relayDyn.value = RelayState.Standby
            nudges.tryReceive()
            when (session(p)) {
                End.Revoked -> {
                    _state.value = LinkState.Revoked   // before the pairing goes, so observers see why
                    onRevoked(p)
                    return
                }
                End.ToLan -> { failures = 0; continue }   // the home network answers again: straight back, no waiting
                End.Dropped -> failures = 0          // it worked for a while: start the schedule over
                End.Failed -> failures++
            }
            if (paired == null) return
            if (failures < NOT_FOUND_AFTER) _state.value = LinkState.Connecting
            withTimeoutOrNull(policy.retryDelayMs(failures.coerceAtLeast(1))) { nudges.receive() }
        }
    }

    private enum class End { Revoked, Dropped, Failed, ToLan }

    /** One connection attempt over one route. [send] is the only way out; frames in go to [handleText]. */
    private class Conn(val route: Route) {
        lateinit var ws: WebSocket
        /** Completes when the connection is usable (LAN: socket open; relay: joined), false when it died first. */
        val ready = CompletableDeferred<Boolean>()
        val done = CompletableDeferred<End>()
        val lastRx = AtomicLong(System.currentTimeMillis())
        @Volatile var opened = false
        @Volatile var refused = false
        @Volatile var peerUp = route == Route.Lan
        var crypto: FrameCrypto? = null
        val auth = AuthFailGuard()

        fun sendText(text: String): Boolean {
            if (route == Route.Lan) return ws.send(text)
            // seal and send under one lock: the receiver drops a frame whose seq is not newer than the last one
            synchronized(this) {
                val frame = crypto?.seal(text) ?: return false
                return ws.send(frame.toByteString())
            }
        }

        fun finish(e: End) {
            ready.complete(false)
            done.complete(e)
        }
    }

    private suspend fun session(p: Paired): End {
        val relay = relayConfig(p)
        val lan = openLan(p, policy.lanTimeoutMs())
        if (lan.done.isCompleted && lan.done.getCompleted() == End.Revoked) return End.Revoked
        if (lan.opened) return serve(p, lan)
        lan.ws.cancel()
        if (policy.afterLanFailed() == RoutePolicy.Step.GiveUp || relay == null) return End.Failed
        _relayDyn.value = RelayState.Connecting
        val rl = openRelay(p, relay)
        if (rl.opened) return serve(p, rl)
        rl.ws.cancel()
        if (rl.refused) _relayDyn.value = RelayState.Refused
        return End.Failed
    }

    private suspend fun openLan(p: Paired, timeoutMs: Long): Conn {
        val conn = Conn(Route.Lan)
        val request = Request.Builder().url("ws://${p.host}:${p.port}/ws?token=${URLEncoder.encode(p.token, "UTF-8")}").build()
        conn.ws = PcApi.socket.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                conn.opened = true
                conn.lastRx.set(now())
                _state.value = LinkState.Connected(p.pcName, Route.Lan)
                _relayDyn.value = RelayState.Standby
                Store.touchSeen(pcId)
                conn.ready.complete(true)
            }

            override fun onMessage(webSocket: WebSocket, text: String) = handleText(conn, p, text)

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                if (code == 4001) conn.finish(End.Revoked) else { webSocket.close(1000, null); conn.finish(if (conn.opened) End.Dropped else End.Failed) }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                conn.finish(if (code == 4001) End.Revoked else if (conn.opened) End.Dropped else End.Failed)
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                conn.finish(if (response?.code == 401) End.Revoked else if (conn.opened) End.Dropped else End.Failed)
            }
        })
        withTimeoutOrNull(timeoutMs) { conn.ready.await() }
        return conn
    }

    private suspend fun openRelay(p: Paired, cfg: RelayCfg): Conn {
        val conn = Conn(Route.Relay)
        conn.crypto = FrameCrypto(cfg.room, V2.unb64u(cfg.kP2c), V2.unb64u(cfg.kC2p))
        val url = RelayUrl.build(cfg.url, cfg.room)!!   // relayConfig() already checked it
        val join = "{\"t\":\"join\",\"join\":\"${cfg.join}\",\"v\":2}"
        conn.ws = PcApi.relay.newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { webSocket.send(join) }

            override fun onMessage(webSocket: WebSocket, text: String) {
                conn.lastRx.set(now())
                val o = try { Codec.json.parseToJsonElement(text) as? JsonObject } catch (_: Exception) { null } ?: return
                val peer = (o["peer"] as? JsonPrimitive)?.content == "true" || (o["up"] as? JsonPrimitive)?.content == "true"
                when ((o["t"] as? JsonPrimitive)?.content) {
                    "joined" -> { conn.opened = true; conn.peerUp = peer; conn.ready.complete(true); onPeer(conn, peer) }
                    "peer" -> { conn.peerUp = peer; onPeer(conn, peer) }
                }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (!conn.opened) return
                when (val r = conn.crypto!!.open(bytes.toByteArray())) {
                    is FrameCrypto.Opened.Ok -> handleText(conn, p, r.json)
                    FrameCrypto.Opened.Replay -> Unit
                    FrameCrypto.Opened.Invalid -> if (conn.auth.record(now())) webSocket.close(1008, "too many bad frames")
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                if (code == 4401) conn.refused = true   // the relay does not know this join secret
                webSocket.close(1000, null)
                conn.finish(if (conn.opened) End.Dropped else End.Failed)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                if (code == 4401) conn.refused = true
                conn.finish(if (conn.opened) End.Dropped else End.Failed)
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                conn.finish(if (conn.opened) End.Dropped else End.Failed)
            }
        })
        withTimeoutOrNull(JOIN_WAIT_MS) { conn.ready.await() }
        return conn
    }

    /** The relay says the PC came or went. */
    private fun onPeer(conn: Conn, up: Boolean) {
        if (up) {
            conn.lastRx.set(now())
            if (_state.value !is LinkState.Connected) { _state.value = LinkState.Connecting; _relayDyn.value = RelayState.Connecting }
        } else {
            _state.value = LinkState.PcOffline
            _relayDyn.value = RelayState.WaitingForPc
        }
    }

    /** A decoded v1 message from the PC, whichever way it came. */
    private fun handleText(conn: Conn, p: Paired, text: String) {
        conn.lastRx.set(now())
        when (val m = Codec.decode(text)) {
            null -> Unit
            Msg.Pong -> Unit
            Msg.Ping -> conn.sendText(Codec.encode(Msg.Pong))
            is Msg.Hello -> {
                val name = m.pcName.ifBlank { p.pcName }
                m.relayMaxFileMB?.takeIf { it > 0 }?.let { _relayMaxMb.value = it }
                _state.value = LinkState.Connected(name, conn.route)
                if (conn.route == Route.Relay) _relayDyn.value = RelayState.Active
                Store.touchSeen(pcId)
                if (name != p.pcName) Store.setPcName(pcId, name)
                emit(Incoming(pcId, m))
            }
            else -> {
                // A PC that talks without a hello first is still there.
                if (conn.route == Route.Relay && _state.value !is LinkState.Connected) {
                    _state.value = LinkState.Connected(p.pcName, Route.Relay)
                    _relayDyn.value = RelayState.Active
                }
                emit(Incoming(pcId, m))
            }
        }
    }

    /** Keep an open connection alive and watched; on the relay also look at the home network now and then. */
    private suspend fun serve(p: Paired, conn: Conn): End = coroutineScope {
        out = conn
        if (conn.route == Route.Relay) policy.onRelayUp(now())
        val pinger = launch {
            while (true) {
                // on the relay, a PC that joined but said nothing yet gets a quick nudge instead of a 20 s wait
                delay(if (conn.route == Route.Relay && _state.value !is LinkState.Connected) HELLO_WAIT_MS else Proto.PING_MS)
                if (conn.route == Route.Lan || conn.peerUp) {
                    conn.sendText(Codec.encode(Msg.Ping))
                    if (now() - conn.lastRx.get() > Proto.DEAD_AFTER_MS) conn.ws.cancel()
                }
            }
        }
        val prober = if (conn.route == Route.Relay) launch { probeLan(p, conn) } else null
        try {
            conn.done.await()
        } finally {
            pinger.cancel()
            prober?.cancel()
            conn.ws.close(1000, null)
            conn.ws.cancel()
            if (out === conn) out = null
            if (conn.opened) Store.touchSeen(pcId)
        }
    }

    /** While on the relay: every 60 s, and when nudged, ask whether the PC answers on the LAN; if so end the relay session. */
    private suspend fun probeLan(p: Paired, conn: Conn) {
        while (true) {
            val busy = relayHolds.get() > 0
            val nudged = withTimeoutOrNull(policy.probeInMs(now(), busy)) { nudges.receive(); true } ?: false
            if (policy.shouldProbe(now(), relayHolds.get() > 0, nudged)) {
                policy.onProbed(now())
                if (PcApi.lanReachable(paired ?: p)) { conn.done.complete(End.ToLan); return }
            }
        }
    }

    companion object {
        /** Consecutive failed attempts before we call it "PC not found" instead of "connecting". */
        private const val NOT_FOUND_AFTER = 3
        private const val JOIN_WAIT_MS = 8_000L
        private const val HELLO_WAIT_MS = 3_000L
        const val DEFAULT_RELAY_MAX_MB = 100
    }
}

object Net {
    private fun caps(ctx: Context): NetworkCapabilities? {
        val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return null
        return cm.getNetworkCapabilities(cm.activeNetwork ?: return null)
    }

    fun isUp(ctx: Context): Boolean = caps(ctx) != null

    /** "wifi", "mobile" or "none", as the protocol's status message wants it. */
    fun kind(ctx: Context): String {
        val c = caps(ctx) ?: return "none"
        return when {
            c.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || c.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> "wifi"
            c.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "mobile"
            else -> "wifi" // VPN or similar over something else: treat as a LAN-ish link
        }
    }
}
