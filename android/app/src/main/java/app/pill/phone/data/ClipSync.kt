package app.pill.phone.data

import app.pill.phone.protocol.Msg
import java.util.UUID

/** The system clipboard, as little as the sync needs. The Android one is in service/ClipBridge; tests use a fake. */
interface ClipboardAccess {
    /** The text on the clipboard, or null when there is none (or the system refuses: no focus, Android 10+). */
    fun read(): String?
    fun write(text: String)
}

/** What happened to one "send clipboard". */
sealed interface ClipSendResult {
    /** At least one PC took it. */
    data class Sent(val pcIds: List<String>) : ClipSendResult
    /** No PC took it (all down). */
    data object Failed : ClipSendResult
    data object Empty : ClipSendResult
    data object TooLarge : ClipSendResult
    data object Off : ClipSendResult
    /** The PC sent this very text a moment ago, so it already has it: nothing goes back (loop guard). */
    data object AlreadyThere : ClipSendResult
}

/**
 * Clipboard sync rules of protocol v1.2, with no Android in them: caps gating, the 256 KiB limit, empty text, and the loop guard
 * (a text just received is not sent back and the other way round; only a hash is kept, never the text itself).
 * [enabled] is the master switch, [receiving] the "Receive from PC" one.
 */
class ClipSync(
    private val board: ClipboardAccess,
    private val enabled: () -> Boolean,
    private val receiving: () -> Boolean,
    private val now: () -> Long = System::currentTimeMillis,
    private val newId: () -> String = { UUID.randomUUID().toString() },
) {
    @Volatile private var lastHash = 0
    @Volatile private var lastAt = Long.MIN_VALUE

    private fun remember(text: String) { lastHash = text.hashCode(); lastAt = now() }

    /** [text] is what we last sent or applied, and that was only moments ago. */
    private fun isEcho(text: String): Boolean = text.hashCode() == lastHash && now() - lastAt in 0..ECHO_WINDOW_MS

    /** The message that announces the feature, or null while it is off (an off phone says nothing). */
    fun caps(): Msg.Caps? = if (enabled()) Msg.Caps(1) else null

    fun readClipboard(): String? = board.read()

    /** The size / emptiness verdict for [text], without sending anything. */
    fun check(text: String?): ClipSendResult? = when {
        !enabled() -> ClipSendResult.Off
        text.isNullOrEmpty() -> ClipSendResult.Empty
        utf8Size(text) > MAX_BYTES -> ClipSendResult.TooLarge
        else -> null
    }

    /** Sends [text] to each of [pcIds] through [sender]; it counts as sent (and is remembered) once any PC took it. */
    fun send(text: String?, pcIds: Collection<String>, sender: (Collection<String>, Msg) -> FanOut): ClipSendResult {
        check(text)?.let { return it }
        text!!
        if (isEcho(text)) return ClipSendResult.AlreadyThere
        val out = sender(pcIds, Msg.Clip(newId(), text, now()))
        if (out.sent.isEmpty()) return ClipSendResult.Failed
        remember(text)
        return ClipSendResult.Sent(out.sent)
    }

    /** A clip from a PC: puts it on the clipboard unless the feature or receiving is off, it is empty or too big, or it is our own echo. True when applied. */
    fun receive(m: Msg.Clip): Boolean {
        if (!enabled() || !receiving()) return false
        if (m.text.isEmpty() || utf8Size(m.text) > MAX_BYTES) return false
        if (isEcho(m.text)) return false
        remember(m.text)
        board.write(m.text)
        return true
    }

    companion object {
        const val MAX_BYTES = 256 * 1024
        /** A text seen again after this long is a new copy, not an echo. */
        const val ECHO_WINDOW_MS = 30_000L

        /** UTF-8 size without building the byte array (a clipboard can be big). */
        fun utf8Size(s: CharSequence): Int {
            var n = 0
            var i = 0
            while (i < s.length) {
                val c = s[i]
                when {
                    c.code < 0x80 -> n += 1
                    c.code < 0x800 -> n += 2
                    Character.isHighSurrogate(c) && i + 1 < s.length && Character.isLowSurrogate(s[i + 1]) -> { n += 4; i++ }
                    else -> n += 3
                }
                i++
            }
            return n
        }
    }
}
