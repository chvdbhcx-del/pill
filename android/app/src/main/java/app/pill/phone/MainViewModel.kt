package app.pill.phone

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.pill.phone.data.AppFilter
import app.pill.phone.data.AppsRepo
import app.pill.phone.data.FileTarget
import app.pill.phone.data.InstalledApp
import app.pill.phone.data.Paired
import app.pill.phone.data.PcOption
import app.pill.phone.data.Perms
import app.pill.phone.data.Routing
import app.pill.phone.data.Store
import app.pill.phone.net.LinkManager
import app.pill.phone.net.LinkState
import app.pill.phone.net.PairResult
import app.pill.phone.net.PcApi
import app.pill.phone.net.PcView
import app.pill.phone.net.RelayState
import app.pill.phone.protocol.FileNames
import app.pill.phone.protocol.Msg
import app.pill.phone.protocol.PairLink
import app.pill.phone.protocol.PairTarget
import app.pill.phone.service.LinkService
import app.pill.phone.service.StatusReporter
import app.pill.phone.transfer.Source
import app.pill.phone.transfer.Transfer
import app.pill.phone.transfer.Transfers
import app.pill.phone.ui.AppRowUi
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PairError
import app.pill.phone.ui.PairUi
import app.pill.phone.ui.PcSettingsUi
import app.pill.phone.ui.PcUi
import app.pill.phone.ui.PermissionsUi
import app.pill.phone.ui.SettingsUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class Route { Onboarding, Home, Pair, Scan, Apps, Settings }

/** Files the user picked, waiting for the answer to "send to which PC?". */
class PendingSend(val sources: List<Source>)

/** The link state as the user sees it. */
fun LinkState.toUi(): LinkUi = when (this) {
    is LinkState.Connected -> LinkUi.Connected
    LinkState.NoNetwork -> LinkUi.NoNetwork
    LinkState.NotFound, LinkState.PcOffline -> LinkUi.NotFound
    LinkState.Revoked -> LinkUi.Revoked
    LinkState.Connecting, LinkState.Unpaired -> LinkUi.Connecting
}

/** Screen state for the whole app: navigation, the pairing flow and the data the screens show. */
class MainViewModel(private val app: Application) : AndroidViewModel(app) {

    // ---- navigation ----
    private val _stack = MutableStateFlow(listOf(if (Store.onboarded.value) Route.Home else Route.Onboarding))
    val stack: StateFlow<List<Route>> = _stack

    fun push(r: Route) = _stack.update { if (it.last() == r) it else it + r }
    fun pop(): Boolean {
        if (_stack.value.size <= 1) return false
        _stack.update { it.dropLast(1) }
        return true
    }
    fun finishOnboarding() { Store.setOnboarded(); _stack.value = listOf(Route.Home) }

    // ---- permissions ----
    private val _perms = MutableStateFlow(Perms.state(app))
    fun refreshPerms() { _perms.value = Perms.state(app) }

    // ---- home ----
    private val installed = MutableStateFlow<List<InstalledApp>>(emptyList())

    init {
        viewModelScope.launch { installed.value = AppsRepo.load(app) }
    }

    /** The PC the hero and the "This phone" row describe. */
    private val selected = combine(Store.pcs, Store.selectedPc, LinkManager.views) { pcs, saved, views ->
        Routing.selected(pcs, saved, views.filterValues { it.connected }.keys)
    }

    private val mirrored: StateFlow<Int?> = combine(installed, Store.overrides, selected) { apps, global, pc ->
        val o = pc?.overrides(global) ?: global
        if (apps.isEmpty()) null else apps.count { AppFilter.isEnabled(it.pkg, it.system, o) }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    private data class NetBits(val pcs: List<Paired>, val views: Map<String, PcView>, val sel: Paired?, val revoked: Boolean)

    private fun view(views: Map<String, PcView>, pc: Paired?) = views[pc?.pcId] ?: PcView()

    val home: StateFlow<HomeUi> = combine(
        combine(Store.pcs, LinkManager.views, selected, LinkManager.revoked) { pcs, v, sel, rev -> NetBits(pcs, v, sel, rev.isNotEmpty()) },
        combine(Store.relayEnabled, Store.clipEnabled, StatusReporter.current) { on, clip, s -> Triple(on, clip, s) },
        combine(Transfers.items, _perms, mirrored, Store.phoneExpanded) { t, pe, m, x -> HomeBits(t, pe, m, x) },
    ) { net, (relayOn, clipOn, status), (transfers, perms, mirroredApps, phoneExpanded) ->
        val sel = net.sel
        val v = view(net.views, sel)
        val state = v.state
        val two = net.pcs.size >= 2
        HomeUi(
            link = when {
                sel == null -> if (net.revoked) LinkUi.Revoked else LinkUi.Unpaired
                else -> state.toUi()
            },
            pcName = sel?.label,
            address = sel?.address,
            battery = status?.battery,
            charging = status?.charging ?: false,
            network = status?.network ?: "none",
            ssid = status?.ssid,
            perms = perms,
            transfers = transfers.map { it.toUi(net.pcs.takeIf { two }) },
            mirroredApps = mirroredApps,
            status = status,
            route = v.route,
            relay = RelayState.effective(sel, relayOn, v.relayDyn),
            phoneExpanded = phoneExpanded,
            clipEnabled = clipOn,
            pcs = if (two) net.pcs.map { PcUi(it.pcId, it.label, view(net.views, it).state.toUi(), view(net.views, it).route) } else emptyList(),
            selectedPcId = sel?.pcId,
        )
    }.stateIn(viewModelScope, SharingStarted.Eagerly, HomeUi())

    private data class HomeBits(val transfers: List<Transfer>, val perms: PermissionsUi, val mirrored: Int?, val phoneExpanded: Boolean)

    private fun Transfer.toUi(pcs: List<Paired>?) = TransferUi(
        id = id,
        name = name,
        sizeText = FileNames.humanSize(size),
        outgoing = outgoing,
        phase = phase,
        progress = progress,
        detail = error?.let { app.getString(it, errorArg) },
        canOpen = !outgoing && phase == TransferPhase.Done && uri != null,
        pc = pcs?.firstOrNull { it.pcId == pcId }?.label,
    )

    private val settingsBase = combine(Store.pcs, Store.autoReceive, Store.includeOngoing, _perms, selected) { pcs, auto, ongoing, perms, sel ->
        val first = sel ?: pcs.firstOrNull()
        SettingsUi(first?.label?.ifBlank { null }, first?.address, auto, ongoing, perms, BuildConfig.VERSION_NAME, android.os.Build.MODEL.orEmpty(), android.os.Build.VERSION.RELEASE.orEmpty())
    }

    val settings: StateFlow<SettingsUi> = combine(
        settingsBase, Store.relayEnabled, LinkManager.views, combine(Store.pcs, Store.overrides, selected) { p, o, s -> Triple(p, o, s) },
        combine(Store.clipEnabled, Store.clipReceive) { e, r -> e to r },
    ) { s, on, views, (pcs, global, sel), (clipOn, clipRecv) ->
        val v = view(views, sel)
        s.copy(
            clipEnabled = clipOn, clipReceive = clipRecv,
            relayEnabled = on, relay = RelayState.effective(sel, on, v.relayDyn), route = v.route, relayMaxMb = v.relayMaxMb,
            pcs = pcs.map { p ->
                val pv = view(views, p)
                PcSettingsUi(
                    pcId = p.pcId, name = p.label, pcName = p.pcName, address = p.address, link = pv.state.toUi(), route = pv.route,
                    lastSeen = p.lastSeen, mirror = p.mirror, autoReceive = p.autoReceives(Store.autoReceive.value),
                    ownFilter = p.filter != null, filterCount = p.filter?.count { it.value },
                )
            },
        )
    }.stateIn(viewModelScope, SharingStarted.Eagerly, SettingsUi())

    fun setRelayEnabled(on: Boolean) = Store.setRelayEnabled(on)
    fun setAutoReceive(on: Boolean) = Store.setAutoReceive(on)
    fun setClipEnabled(on: Boolean) = Store.setClipEnabled(on)
    fun setClipReceive(on: Boolean) = Store.setClipReceive(on)
    fun setIncludeOngoing(on: Boolean) = Store.setIncludeOngoing(on)
    fun acceptTransfer(id: String) = Transfers.accept(app, id)
    fun openTransfer(id: String) = Transfers.open(app, id)
    fun reconnect() = (Store.selectedPc.value?.let { LinkManager.nudge(it) } ?: LinkManager.nudge())
    fun togglePhone() = Store.setPhoneExpanded(!Store.phoneExpanded.value)
    fun selectPc(pcId: String) = Store.setSelectedPc(pcId)

    // ---- per-PC settings ----
    fun renamePc(pcId: String, name: String) = Store.rename(pcId, name)
    fun setPcMirror(pcId: String, on: Boolean) = Store.setMirror(pcId, on)
    fun setPcAutoReceive(pcId: String, on: Boolean) = Store.setAutoReceiveFor(pcId, on)
    fun setPcOwnFilter(pcId: String, own: Boolean) = Store.setOwnFilter(pcId, own)

    // ---- apps filter ----
    val appsQuery = MutableStateFlow("")
    val showSystem = MutableStateFlow(false)

    /** Whose filter the Apps screen edits: one PC's own, or (null) the global one. */
    private val appsPcId = MutableStateFlow<String?>(null)

    /** The name shown on the Apps screen when it edits one PC's filter. */
    val appsTitle: StateFlow<String?> = combine(appsPcId, Store.pcs) { id, pcs -> pcs.firstOrNull { it.pcId == id }?.label }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    fun openApps(pcId: String? = null) {
        if (pcId != null) Store.setOwnFilter(pcId, true)
        appsPcId.value = pcId
        push(Route.Apps)
    }

    val appRows: StateFlow<List<AppRowUi>> = combine(
        combine(installed, Store.overrides, Store.pcs) { apps, g, pcs -> Triple(apps, g, pcs) }, appsPcId, appsQuery, showSystem,
    ) { (apps, global, pcs), pcId, q, sys ->
        val o = pcs.firstOrNull { it.pcId == pcId }?.overrides(global) ?: global
        apps.asSequence()
            .filter { sys || !it.system || it.pkg in AppFilter.KNOWN_ON || o[it.pkg] == true }
            .filter { q.isBlank() || it.label.contains(q.trim(), ignoreCase = true) || it.pkg.contains(q.trim(), ignoreCase = true) }
            .map { AppRowUi(it.pkg, it.label, AppFilter.isEnabled(it.pkg, it.system, o), it.system) }
            // enabled first, then alphabetical (the list is already alphabetical)
            .sortedByDescending { it.enabled }
            .toList()
    }.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    fun setAppEnabled(pkg: String, on: Boolean) {
        val pc = appsPcId.value
        if (pc != null) Store.setAppEnabledFor(pc, pkg, on) else Store.setAppEnabled(pkg, on)
    }
    suspend fun icon(pkg: String) = AppsRepo.icon(app, pkg)

    // ---- sending files: ask which PC when there is a choice ----
    private val _pending = MutableStateFlow<PendingSend?>(null)
    /** Set while the "send to which PC?" sheet is up. */
    val pendingSend: StateFlow<PendingSend?> = _pending

    /** The PCs for that sheet, live: a PC that goes offline while it is open turns dim. */
    val pcOptions: StateFlow<List<PcOption>> = LinkManager.options.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    /** Files were picked in the app. One PC: send. Several: ask. */
    fun onFilesPicked(sources: List<Source>) {
        if (sources.isEmpty()) return
        when (val t = Routing.fileTarget(Store.pcs.value, LinkManager.connectedIds())) {
            FileTarget.None -> sources.forEach { it.release() }
            is FileTarget.Single -> Transfers.send(app, sources, listOf(t.pcId))
            is FileTarget.Ask -> { _pending.value?.sources?.forEach { it.release() }; _pending.value = PendingSend(sources) }
        }
    }

    fun chooseTargets(pcIds: List<String>) {
        val p = _pending.getAndUpdate { null } ?: return
        Transfers.send(app, p.sources, pcIds)
    }

    fun dismissSend() {
        _pending.getAndUpdate { null }?.sources?.forEach { it.release() }
    }

    // ---- pairing ----
    private val _pair = MutableStateFlow<PairUi>(PairUi.Entry())
    val pair: StateFlow<PairUi> = _pair
    private var pending: PairTarget? = null

    fun openPair(manual: Boolean = false) {
        _pair.value = PairUi.Entry(manual = manual)
        push(Route.Pair)
    }

    /** A pillphone:// link or a scanned QR code. */
    fun onPairLink(raw: String?) {
        val t = PairLink.parse(raw)
        push(Route.Pair)
        if (t == null) {
            _pair.value = PairUi.Failed(PairError.InvalidLink, null)
        } else {
            pending = t
            _pair.value = PairUi.Confirm(t.address, t.pcName)
        }
    }

    fun openScanner() = push(Route.Scan)

    /** A recognised pairing QR: the scanner makes way for the confirm screen. */
    fun onScanned(raw: String) {
        _stack.update { st -> st.filter { it != Route.Scan }.ifEmpty { listOf(Route.Home) } }
        onPairLink(raw)
    }

    fun enterCodeInstead() {
        _stack.update { st -> st.filter { it != Route.Scan }.ifEmpty { listOf(Route.Home) } }
        openPair(manual = true)
    }

    fun setManual(on: Boolean) { _pair.value = PairUi.Entry(manual = on) }
    fun setAddress(v: String) = _pair.update { if (it is PairUi.Entry) it.copy(address = v, error = null) else it }
    fun setCode(v: String) = _pair.update {
        if (it is PairUi.Entry) it.copy(code = v.uppercase().filter { c -> c.isLetterOrDigit() }.take(8), error = null) else it
    }

    fun submitManual() {
        val e = _pair.value as? PairUi.Entry ?: return
        val t = PairLink.parseManual(e.address, e.code)
        if (t == null) _pair.value = e.copy(error = PairError.InvalidLink) else runPair(t)
    }

    fun confirmPair() { pending?.let(::runPair) }
    fun retryPair() { pending?.let(::runPair) ?: _pair.update { PairUi.Entry() } }

    /** Back inside the pairing screen: one step up the flow, then out. */
    fun pairBack() {
        when (val s = _pair.value) {
            is PairUi.Entry -> if (s.manual) _pair.value = PairUi.Entry() else pop()
            is PairUi.Working -> Unit
            else -> pop()
        }
    }

    private fun runPair(t: PairTarget) {
        pending = t
        _pair.value = PairUi.Working(t.address)
        viewModelScope.launch {
            when (val r = PcApi.pair(t, app, Store.deviceId)) {
                is PairResult.Ok -> {
                    // adds a PC, or replaces the entry of the same PC (same address or name); the other PCs are left alone
                    val written = Store.addOrReplace(t.host, t.port, r.token, r.pcName, r.relay)
                    LinkManager.clearRevoked()
                    LinkService.start(app)
                    Store.setSelectedPc(written.pcId)
                    _pair.value = PairUi.Success(r.pcName)
                    delay(1500)
                    _stack.value = listOf(Route.Home)
                    pending = null
                }
                PairResult.BadCode -> _pair.value = PairUi.Failed(PairError.BadCode, t.address)
                PairResult.Unreachable -> _pair.value = PairUi.Failed(PairError.Unreachable, t.address)
                is PairResult.Refused -> _pair.value = PairUi.Failed(PairError.Refused, t.address)
                PairResult.RelayMismatch -> _pair.value = PairUi.Failed(PairError.RelayMismatch, t.address)
            }
        }
    }

    /** Unpair one PC; the others keep running. */
    fun unpair(pcId: String) {
        val p = Store.pc(pcId) ?: return
        viewModelScope.launch {
            PcApi.unpair(p)
            Store.remove(pcId)
            if (Store.pcs.value.isEmpty()) {
                LinkService.stop(app)
                _stack.value = listOf(Route.Home)
            }
        }
    }
}
