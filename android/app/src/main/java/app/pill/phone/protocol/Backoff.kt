package app.pill.phone.protocol

/** Reconnect schedule of the protocol: 1 s, 2 s, 4 s, 8 s, 16 s, then 30 s forever. */
object Backoff {
    const val MIN_MS = 1_000L
    const val MAX_MS = 30_000L

    /** [attempt] counts consecutive failures, starting at 0. */
    fun delayMs(attempt: Int): Long = when {
        attempt <= 0 -> MIN_MS
        attempt >= 5 -> MAX_MS
        else -> (MIN_MS shl attempt).coerceAtMost(MAX_MS)
    }
}
