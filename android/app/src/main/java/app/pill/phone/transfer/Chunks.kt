package app.pill.phone.transfer

import java.util.Base64

/** Constants of "Files over the relay" in docs/phone-relay.md. */
object ChunkFlow {
    /** Raw bytes per `file.chunk` (the base64 text stays far below the 900 KiB frame limit). */
    const val CHUNK = 96 * 1024
    const val MAX_IN_FLIGHT = 8

    /**
     * The relay drops frames above 4 MiB/s per room (both directions together), so the sender paces itself well below that.
     * The limit also bounds the frame rate: 3 MiB/s of 96 KiB chunks is about 31 frames/s, under the relay's 60.
     */
    const val MAX_BYTES_PER_SEC = 3L * 1024 * 1024

    fun paceDelayMs(bytes: Int): Long = bytes * 1000L / MAX_BYTES_PER_SEC

    fun encode(buf: ByteArray, len: Int): String = Base64.getEncoder().encodeToString(if (len == buf.size) buf else buf.copyOf(len))
    fun decode(d: String): ByteArray? = try { Base64.getDecoder().decode(d) } catch (_: IllegalArgumentException) { null }
}

/** Sender side flow control: at most [maxInFlight] chunks that the receiver has not acknowledged yet. */
class SendWindow(private val maxInFlight: Int = ChunkFlow.MAX_IN_FLIGHT) {
    private val ends = ArrayDeque<Long>()
    var acked = 0L
        private set

    val inFlight: Int get() = ends.size
    fun canSend(): Boolean = ends.size < maxInFlight

    /** A chunk ending at byte [end] (exclusive) went out. */
    fun onSent(end: Long) { ends.addLast(end) }

    /** `file.ack {upto}` is cumulative; a stale or repeated ack changes nothing. */
    fun onAck(upto: Long) {
        if (upto <= acked) return
        acked = upto
        while (ends.isNotEmpty() && ends.first() <= upto) ends.removeFirst()
    }
}

/** Receiver side: chunks must arrive in order and add up to exactly the announced size. */
class ChunkReceiver(private val expectedSize: Long) {
    var received = 0L
        private set

    sealed interface Result {
        data object Ok : Result
        /** Not the next byte (a frame was lost or reordered): the transfer cannot continue. */
        data object BadOffset : Result
        /** More than announced. */
        data object TooMuch : Result
    }

    fun accept(off: Long, len: Int): Result = when {
        off != received -> Result.BadOffset
        received + len > expectedSize -> Result.TooMuch
        else -> { received += len; Result.Ok }
    }

    /** `file.end {size}`: the sender's count must match ours and the announced size. */
    fun complete(size: Long): Boolean = size == received && received == expectedSize
}
