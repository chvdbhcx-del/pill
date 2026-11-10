package app.pill.phone

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.pill.phone.data.AppFilter
import app.pill.phone.data.AppsRepo
import app.pill.phone.data.InstalledApp
import app.pill.phone.data.Perms
import app.pill.phone.data.Store
import app.pill.phone.net.Link
import app.pill.phone.net.LinkState
import app.pill.phone.net.PairResult
import app.pill.phone.net.PcApi
import app.pill.phone.protocol.FileNames
import app.pill.phone.protocol.PairLink
import app.pill.phone.protocol.PairTarget
import app.pill.phone.service.LinkService
import app.pill.phone.service.StatusReporter
import app.pill.phone.transfer.Transfer
import app.pill.phone.transfer.Transfers
import app.pill.phone.ui.AppRowUi
import app.pill.phone.ui.HomeUi
import app.pill.phone.ui.LinkUi
import app.pill.phone.ui.PairError
import app.pill.phone.ui.PairUi
import app.pill.phone.ui.SettingsUi
import app.pill.phone.ui.TransferPhase
import app.pill.phone.ui.TransferUi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class Route { Onboarding, Home, Pair, Scan, Apps, Settings }

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

    private val mirrored: StateFlow<Int?> = combine(installed, Store.overrides) { apps, o ->
        if (apps.isEmpty()) null else apps.count { AppFilter.isEnabled(it.pkg, it.system, o) }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    val home: StateFlow<HomeUi> = combine(
        combine(Link.state, Store.paired, StatusReporter.current) { l, p, s -> Triple(l, p, s) },
        combine(Transfers.items, _perms, mirrored) { t, pe, m -> Triple(t, pe, m) },
    ) { (link, paired, status), (transfers, perms, mirroredApps) ->
        HomeUi(
            link = when {
                paired == null -> if (link is LinkState.Revoked) LinkUi.Revoked else LinkUi.Unpaired
                else -> when (link) {
                    is LinkState.Connected -> LinkUi.Connected
                    LinkState.NoNetwork -> LinkUi.NoNetwork
                    LinkState.NotFound -> LinkUi.NotFound
                    LinkState.Revoked -> LinkUi.Revoked
                    LinkState.Connecting, LinkState.Unpaired -> LinkUi.Connecting
                }
            },
            pcName = (link as? LinkState.Connected)?.pcName ?: paired?.pcName,
            address = paired?.address,
            battery = status?.battery,
            charging = status?.charging ?: false,
            network = status?.network ?: "none",
            ssid = status?.ssid,
            perms = perms,
            transfers = transfers.map { it.toUi() },
            mirroredApps = mirroredApps,
            status = status,
        )
    }.stateIn(viewModelScope, SharingStarted.Eagerly, HomeUi())

    private fun Transfer.toUi() = TransferUi(
        id = id,
        name = name,
        sizeText = FileNames.humanSize(size),
        outgoing = outgoing,
        phase = phase,
        progress = progress,
        detail = error?.let { app.getString(it, errorArg) },
        canOpen = !outgoing && phase == TransferPhase.Done && uri != null,
    )

    val settings: StateFlow<SettingsUi> = combine(Store.paired, Store.autoReceive, Store.includeOngoing, _perms) { p, auto, ongoing, perms ->
        SettingsUi(p?.pcName?.ifBlank { null }, p?.address, auto, ongoing, perms, BuildConfig.VERSION_NAME, android.os.Build.MODEL.orEmpty(), android.os.Build.VERSION.RELEASE.orEmpty())
    }.stateIn(viewModelScope, SharingStarted.Eagerly, SettingsUi())

    fun setAutoReceive(on: Boolean) = Store.setAutoReceive(on)
    fun setIncludeOngoing(on: Boolean) = Store.setIncludeOngoing(on)
    fun acceptTransfer(id: String) = Transfers.accept(app, id)
    fun openTransfer(id: String) = Transfers.open(app, id)
    fun reconnect() = Link.nudge()

    // ---- apps filter ----
    val appsQuery = MutableStateFlow("")
    val showSystem = MutableStateFlow(false)

    val appRows: StateFlow<List<AppRowUi>> = combine(installed, Store.overrides, appsQuery, showSystem) { apps, o, q, sys ->
        apps.asSequence()
            .filter { sys || !it.system || it.pkg in AppFilter.KNOWN_ON || o[it.pkg] == true }
            .filter { q.isBlank() || it.label.contains(q.trim(), ignoreCase = true) || it.pkg.contains(q.trim(), ignoreCase = true) }
            .map { AppRowUi(it.pkg, it.label, AppFilter.isEnabled(it.pkg, it.system, o), it.system) }
            // enabled first, then alphabetical (the list is already alphabetical)
            .sortedByDescending { it.enabled }
            .toList()
    }.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    fun setAppEnabled(pkg: String, on: Boolean) = Store.setAppEnabled(pkg, on)
    suspend fun icon(pkg: String) = AppsRepo.icon(app, pkg)

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
                    Store.savePairing(t.host, t.port, r.token, r.pcName)
                    LinkService.start(app)
                    Link.networkChanged()   // a link to a previous PC must not linger
                    _pair.value = PairUi.Success(r.pcName)
                    delay(1500)
                    _stack.value = listOf(Route.Home)
                    pending = null
                }
                PairResult.BadCode -> _pair.value = PairUi.Failed(PairError.BadCode, t.address)
                PairResult.Unreachable -> _pair.value = PairUi.Failed(PairError.Unreachable, t.address)
                is PairResult.Refused -> _pair.value = PairUi.Failed(PairError.Refused, t.address)
            }
        }
    }

    fun unpair() {
        val p = Store.paired.value ?: return
        viewModelScope.launch {
            PcApi.unpair(p)
            Store.clearPairing()
            LinkService.stop(app)
            _stack.value = listOf(Route.Home)
        }
    }
}
