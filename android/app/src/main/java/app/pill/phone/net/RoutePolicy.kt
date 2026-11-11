package app.pill.phone.net

import app.pill.phone.protocol.Backoff

/** Which way the phone reaches the PC. */
enum class Route { Lan, Relay }

/**
 * The route rules of docs/phone-relay.md, with no I/O so a fake clock can drive them:
 * try the LAN host first (2 s), else the relay; while on the relay look at the LAN again every 60 s (and when asked: a
 * network change restarts the whole attempt, the reconnect button nudges) and switch back when it answers;
 * never switch away while a file moves through the relay; back-off 1..30 s between failed attempts.
 */
class RoutePolicy(private val relayUsable: () -> Boolean) {
    enum class Step { TryRelay, GiveUp }

    /** How long the first LAN attempt may take: short when the relay is the fallback, the old 6 s when nothing else is left. */
    fun lanTimeoutMs(): Long = if (relayUsable()) LAN_FIRST_MS else LAN_ONLY_MS

    fun afterLanFailed(): Step = if (relayUsable()) Step.TryRelay else Step.GiveUp

    private var lastProbe = 0L

    /** The relay session is up as of [now]; the first LAN look is a full interval away. */
    fun onRelayUp(now: Long) { lastProbe = now }

    fun onProbed(now: Long) { lastProbe = now }

    /** Whether to look at the LAN now. [nudged]: the user pressed reconnect or the network came back. */
    fun shouldProbe(now: Long, relayBusy: Boolean, nudged: Boolean): Boolean =
        !relayBusy && (nudged || now - lastProbe >= PROBE_MS)

    /** How long the prober may sleep before it has to ask [shouldProbe] again. */
    fun probeInMs(now: Long, relayBusy: Boolean): Long =
        if (relayBusy) BUSY_RECHECK_MS else (lastProbe + PROBE_MS - now).coerceIn(0, PROBE_MS)

    /** [failures] consecutive failed attempts so far (>= 1). */
    fun retryDelayMs(failures: Int): Long = Backoff.delayMs(failures - 1)

    companion object {
        const val LAN_FIRST_MS = 2_000L
        const val LAN_ONLY_MS = 6_000L
        const val PROBE_MS = 60_000L
        const val BUSY_RECHECK_MS = 5_000L
    }
}
