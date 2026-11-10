package app.pill.phone.net

import android.annotation.SuppressLint
import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import app.pill.phone.data.Paired
import app.pill.phone.data.Store
import app.pill.phone.protocol.Backoff
import app.pill.phone.protocol.Codec
import app.pill.phone.protocol.Msg
import app.pill.phone.protocol.Proto
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.net.URLEncoder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

sealed interface LinkState {
    data object Unpaired : LinkState
    data object Connecting : LinkState
    data class Connected(val pcName: String) : LinkState
    data object NoNetwork : LinkState
    /** Network is up but the PC does not answer: wrong network, PC off, Pill closed. */
    data object NotFound : LinkState
    data object Revoked : LinkState
}

/**
 * The one WebSocket to the PC (protocol "Realtime channel"). Owned by [app.pill.phone.service.LinkService];
 * everything else only reads [state] / [incoming] and calls [send].
 */
@SuppressLint("StaticFieldLeak") // holds only the application context, which lives as long as the process
object Link {
    private lateinit var app: Context

    private val _state = MutableStateFlow<LinkState>(LinkState.Unpaired)
    val state: StateFlow<LinkState> = _state.asStateFlow()

    private val _incoming = MutableSharedFlow<Msg>(extraBufferCapacity = 256, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    /** Every frame from the PC except ping/pong. [Msg.Hello] marks a fresh session. */
    val incoming: SharedFlow<Msg> = _incoming.asSharedFlow()

    private val nudges = Channel<Unit>(Channel.CONFLATED)
    private var job: Job? = null
    @Volatile private var socket: WebSocket? = null

    /** Consecutive failed attempts before we call it "PC not found" instead of "connecting". */
    private const val NOT_FOUND_AFTER = 3

    fun init(ctx: Context) {
        app = ctx.applicationContext
        if (Store.paired.value == null) _state.value = LinkState.Unpaired
    }

    @Synchronized
    fun start(scope: CoroutineScope) {
        if (job?.isActive == true) return
        job = scope.launch { loop() }
    }

    @Synchronized
    fun stop() {
        job?.cancel()
        job = null
        socket?.cancel()
        socket = null
        if (_state.value !is LinkState.Revoked) {
            _state.value = if (Store.paired.value == null) LinkState.Unpaired else LinkState.Connecting
        }
    }

    /** Try again now instead of waiting out the back-off (button, network back). */
    fun nudge() { nudges.trySend(Unit) }

    /** The default network changed: the open socket is probably dead, do not wait for the watchdog. */
    fun networkChanged() {
        socket?.cancel()
        nudge()
    }

    fun send(msg: Msg): Boolean {
        val ws = socket ?: return false
        if (_state.value !is LinkState.Connected) return false
        return ws.send(Codec.encode(msg))
    }

    val isConnected: Boolean get() = _state.value is LinkState.Connected

    private suspend fun loop() {
        var failures = 0
        while (currentCoroutineContext().isActive) {
            val p = Store.paired.value
            if (p == null) {
                _state.value = LinkState.Unpaired
                Store.paired.first { it != null }
                failures = 0
                continue
            }
            if (!Net.isUp(app)) {
                _state.value = LinkState.NoNetwork
                withTimeoutOrNull(30_000) { nudges.receive() }
                continue
            }
            _state.value = if (failures >= NOT_FOUND_AFTER) LinkState.NotFound else LinkState.Connecting
            nudges.tryReceive()
            when (session(p)) {
                End.Revoked -> {
                    _state.value = LinkState.Revoked   // before the pairing goes, so observers see why
                    Store.clearPairing()
                    Store.paired.first { it != null }
                    failures = 0
                    continue
                }
                End.Dropped -> failures = 0          // it worked for a while: start the schedule over
                End.Failed -> failures++
            }
            if (Store.paired.value == null) continue
            if (failures < NOT_FOUND_AFTER) _state.value = LinkState.Connecting
            withTimeoutOrNull(Backoff.delayMs(failures - 1)) { nudges.receive() }
        }
    }

    private enum class End { Revoked, Dropped, Failed }

    private suspend fun session(p: Paired): End = coroutineScope {
        val done = CompletableDeferred<End>()
        val lastRx = AtomicLong(System.currentTimeMillis())
        val opened = AtomicBoolean(false)
        val request = Request.Builder().url("ws://${p.host}:${p.port}/ws?token=${URLEncoder.encode(p.token, "UTF-8")}").build()

        val ws = PcApi.socket.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened.set(true)
                lastRx.set(System.currentTimeMillis())
                _state.value = LinkState.Connected(p.pcName)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                lastRx.set(System.currentTimeMillis())
                when (val m = Codec.decode(text)) {
                    null, Msg.Pong -> Unit
                    Msg.Ping -> webSocket.send(Codec.encode(Msg.Pong))
                    is Msg.Hello -> {
                        val name = m.pcName.ifBlank { p.pcName }
                        _state.value = LinkState.Connected(name)
                        if (name != p.pcName) Store.savePairing(p.host, p.port, p.token, name)
                        _incoming.tryEmit(m)
                    }
                    else -> _incoming.tryEmit(m)
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                if (code == 4001) done.complete(End.Revoked) else { webSocket.close(1000, null); done.complete(if (opened.get()) End.Dropped else End.Failed) }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                done.complete(if (code == 4001) End.Revoked else if (opened.get()) End.Dropped else End.Failed)
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                done.complete(if (response?.code == 401) End.Revoked else if (opened.get()) End.Dropped else End.Failed)
            }
        })
        socket = ws

        val pinger = launch {
            while (true) {
                delay(Proto.PING_MS)
                ws.send(Codec.encode(Msg.Ping))
                if (System.currentTimeMillis() - lastRx.get() > Proto.DEAD_AFTER_MS) ws.cancel()
            }
        }
        try {
            done.await()
        } finally {
            pinger.cancel()
            ws.close(1000, null)
            ws.cancel()
            if (socket === ws) socket = null
        }
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
