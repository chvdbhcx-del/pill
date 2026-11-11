package app.pill.phone.net

import android.annotation.SuppressLint
import android.content.Context
import app.pill.phone.data.FanOut
import app.pill.phone.data.Paired
import app.pill.phone.data.PcOption
import app.pill.phone.data.Routing
import app.pill.phone.data.Store
import app.pill.phone.protocol.Msg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap

/** What one PC's connection looks like right now. */
data class PcView(
    val state: LinkState = LinkState.Connecting,
    /** The relay's dynamic state (before it is folded with the pairing's keys and the setting). */
    val relayDyn: RelayState = RelayState.Standby,
    val relayMaxMb: Int = PcLink.DEFAULT_RELAY_MAX_MB,
) {
    val connected: Boolean get() = state is LinkState.Connected
    val route: Route? get() = (state as? LinkState.Connected)?.route
}

/**
 * Owns one [PcLink] per paired PC and keeps them in step with [Store.pcs]: a new pairing starts a link, an unpaired PC stops
 * its link, a changed token or address restarts it. Each link decides its own route (home network or relay) and backs off on
 * its own, so a PC that cannot be reached never slows down another. Started and stopped by the foreground service.
 */
@SuppressLint("StaticFieldLeak") // holds only the application context, which lives as long as the process
object LinkManager {
    private lateinit var app: Context

    private class Entry(val link: PcLink, val job: Job, val credentials: List<Any?>)

    private val entries = ConcurrentHashMap<String, Entry>()

    private val _views = MutableStateFlow<Map<String, PcView>>(emptyMap())
    /** The connection state of every PC that has a running link; a PC without an entry is still starting. */
    val views: StateFlow<Map<String, PcView>> = _views.asStateFlow()

    private val _incoming = MutableSharedFlow<Incoming>(extraBufferCapacity = 256, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    /** Every frame from any PC except ping/pong, tagged with its PC. [Msg.Hello] marks a fresh session of that PC. */
    val incoming: SharedFlow<Incoming> = _incoming.asSharedFlow()

    private val _revoked = MutableStateFlow<List<String>>(emptyList())
    /** Names of the PCs that removed this phone since the user last looked; cleared on the next pairing or by [clearRevoked]. */
    val revoked: StateFlow<List<String>> = _revoked.asStateFlow()

    private var syncJob: Job? = null

    fun init(ctx: Context) {
        app = ctx.applicationContext
    }

    @Synchronized
    fun start(scope: CoroutineScope) {
        if (syncJob?.isActive == true) return
        syncJob = scope.launch {
            Store.pcs.collect { sync(this, it) }
        }
    }

    @Synchronized
    fun stop() {
        syncJob?.cancel()
        syncJob = null
        for (e in entries.values) e.job.cancel()
        entries.clear()
        _views.value = emptyMap()
    }

    private fun sync(scope: CoroutineScope, pcs: List<Paired>) {
        val ids = pcs.mapTo(HashSet()) { it.pcId }
        for ((id, e) in entries.entries.toList()) {
            if (id !in ids) {
                e.job.cancel()
                entries.remove(id)
                _views.update { it - id }
            }
        }
        for (p in pcs) {
            val e = entries[p.pcId]
            if (e == null) {
                val link = PcLink(p.pcId, app, { _incoming.tryEmit(it) }, ::pcRevoked)
                val job = scope.launch {
                    coroutineScope {
                        launch { link.run() }
                        launch {
                            combine(link.state, link.relayDyn, link.relayMaxFileMB) { s, r, mb -> PcView(s, r, mb) }
                                .collect { v -> _views.update { it + (p.pcId to v) } }
                        }
                    }
                }
                entries[p.pcId] = Entry(link, job, p.credentials)
            } else if (e.credentials != p.credentials) {
                // re-paired, or the PC moved: the open session belongs to the old token / address
                entries[p.pcId] = Entry(e.link, e.job, p.credentials)
                e.link.networkChanged()
            }
        }
    }

    /** The PC dropped our token: forget just that pairing, remember its name for the notice. */
    private fun pcRevoked(p: Paired) {
        _revoked.update { it + p.label }
        Store.remove(p.pcId)
    }

    fun clearRevoked() { _revoked.value = emptyList() }

    fun link(pcId: String): PcLink? = entries[pcId]?.link

    fun isConnected(pcId: String): Boolean = entries[pcId]?.link?.isConnected == true

    /** Ids of the PCs that are up right now. */
    fun connectedIds(): Set<String> = entries.filterValues { it.link.isConnected }.keys.toSet()

    fun route(pcId: String): Route? = entries[pcId]?.link?.route

    fun send(pcId: String, msg: Msg): Boolean = entries[pcId]?.link?.send(msg) ?: false

    /** Send to each of [pcIds] on its own link; a PC that is down or fails does not stop the others. */
    fun sendTo(pcIds: Collection<String>, msg: Msg): FanOut = Routing.fanOut(pcIds, msg, ::send)

    /** Try again now instead of waiting out the back-off (button, network back). */
    fun nudge() { for (e in entries.values) e.link.nudge() }

    fun nudge(pcId: String) { entries[pcId]?.link?.nudge() }

    /** The default network changed: every open socket is probably dead. */
    fun networkChanged() { for (e in entries.values) e.link.networkChanged() }

    /** The PCs as the "send to which PC" sheet lists them, kept current as links come and go. */
    val options: Flow<List<PcOption>> = combine(Store.pcs, views) { pcs, v ->
        Routing.options(pcs, v.filterValues { it.connected }.keys, v.mapValues { it.value.route })
    }
}
