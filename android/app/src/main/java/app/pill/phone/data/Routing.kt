package app.pill.phone.data

import androidx.compose.runtime.Immutable
import app.pill.phone.net.Route
import app.pill.phone.protocol.Msg

/** A PC as the "send to which PC" sheet lists it. Offline ones are shown but cannot be picked. */
@Immutable
data class PcOption(val pcId: String, val name: String, val online: Boolean, val route: Route? = null)

/** What to do when the user sends files from the phone. */
sealed interface FileTarget {
    /** Nothing is paired. */
    data object None : FileTarget
    /** Exactly one PC: no question, whatever its state (a transfer to an offline PC fails visibly, as before). */
    data class Single(val pcId: String) : FileTarget
    /** Two or more: always ask. */
    data class Ask(val options: List<PcOption>) : FileTarget {
        val anyOnline: Boolean get() = options.any { it.online }
    }
}

/** The result of sending one message to several PCs: who took it and who did not. */
data class FanOut(val sent: List<String>, val failed: List<String>)

/** Pure routing rules: which PC gets what. No Android, no sockets. */
object Routing {
    fun options(pcs: List<Paired>, connected: Set<String>, routes: Map<String, Route?>): List<PcOption> =
        pcs.map { PcOption(it.pcId, it.label, it.pcId in connected, routes[it.pcId]) }

    /** The PC the Home hero describes: the one the user picked if it is still paired, else the first one that is up, else the first. */
    fun selected(pcs: List<Paired>, saved: String?, connected: Set<String>): Paired? =
        pcs.firstOrNull { it.pcId == saved } ?: pcs.firstOrNull { it.pcId in connected } ?: pcs.firstOrNull()

    fun fileTarget(pcs: List<Paired>, connected: Set<String>, routes: Map<String, Route?> = emptyMap()): FileTarget = when (pcs.size) {
        0 -> FileTarget.None
        1 -> FileTarget.Single(pcs[0].pcId)
        else -> FileTarget.Ask(options(pcs, connected, routes))
    }

    /** The connected PCs that should get a mirrored notification of [pkg]: mirror switch on and this PC's own app filter (or the global one) allowing it. */
    fun mirrorTargets(pcs: List<Paired>, connected: Set<String>, pkg: String, system: Boolean, global: Map<String, Boolean>): List<Paired> =
        pcs.filter { it.pcId in connected && it.mirror && AppFilter.isEnabled(pkg, system, it.overrides(global)) }

    /** Calls [send] for every target; one that throws or refuses never keeps the others from getting it. */
    fun fanOut(targets: Collection<String>, send: (String) -> Boolean): FanOut {
        val ok = ArrayList<String>()
        val bad = ArrayList<String>()
        for (t in targets) {
            val sent = try { send(t) } catch (_: Exception) { false }
            (if (sent) ok else bad) += t
        }
        return FanOut(ok, bad)
    }

    /** The same, for a protocol message through a sender keyed by PC id. */
    fun fanOut(targets: Collection<String>, msg: Msg, send: (String, Msg) -> Boolean): FanOut = fanOut(targets) { send(it, msg) }

    /** The foreground notification's headline for several PCs. [connected] of [total] are up;. */
    sealed interface Summary {
        data class All(val count: Int) : Summary
        data class Some(val connected: Int, val total: Int) : Summary
        data object None : Summary
    }

    fun summary(connected: Int, total: Int): Summary = when {
        connected <= 0 -> Summary.None
        connected >= total -> Summary.All(total)
        else -> Summary.Some(connected, total)
    }
}
