package app.pill.phone.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json

/** One remembered text. [from] is the name of the PC it came from, null for this phone. */
@Serializable
data class ClipItem(val text: String, val at: Long, val from: String? = null)

/**
 * The clipboard history: newest first, at most [MAX_ITEMS] items and [MAX_BYTES] of text in all, each item up to the 256 KiB protocol cap.
 * No Android in here; [save] gets the JSON after every change (the Store writes it to a private file). Texts are never logged.
 * [keep] is the "Keep clipboard history" switch: off means nothing is added and the list is emptied.
 */
class ClipHistory(stored: String?, keep: Boolean = true, private val save: (String) -> Unit = {}) {
    private val _keep = MutableStateFlow(keep)
    val keep: StateFlow<Boolean> = _keep.asStateFlow()
    private val _items = MutableStateFlow(if (keep) decode(stored) else emptyList())
    val items: StateFlow<List<ClipItem>> = _items.asStateFlow()

    /**
     * Puts [text] on top. The same text as the newest item only refreshes its time (and keeps its source); the same text further down
     * moves to the top. Empty, oversized or (with the switch off) any text is ignored.
     */
    @Synchronized fun add(text: String, at: Long, from: String? = null) {
        if (!_keep.value || text.isEmpty() || ClipSync.utf8Size(text) > ClipSync.MAX_BYTES) return
        val old = _items.value
        val top = old.firstOrNull()
        val item = if (top?.text == text) top.copy(at = at) else ClipItem(text, at, from)
        set(trim(listOf(item) + old.filter { it.text != text }))
    }

    @Synchronized fun remove(item: ClipItem) = set(_items.value.filter { it != item })
    fun clear() = set(emptyList())

    /** Switching off also clears. */
    fun setKeep(on: Boolean) { _keep.value = on; if (!on) clear() }

    private fun set(next: List<ClipItem>) {
        if (next == _items.value) return
        _items.value = next
        save(encode(next))
    }

    companion object {
        const val MAX_ITEMS = 100
        const val MAX_BYTES = 1024 * 1024
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }

        /** The newest items that fit both caps. */
        fun trim(list: List<ClipItem>): List<ClipItem> {
            var bytes = 0
            return list.take(MAX_ITEMS).takeWhile { bytes += ClipSync.utf8Size(it.text); bytes <= MAX_BYTES }
        }

        fun encode(list: List<ClipItem>): String = json.encodeToString(ListSerializer(ClipItem.serializer()), list)

        /** Whatever is readable in [text]: a damaged or foreign file gives an empty list, a damaged item is skipped, the caps are applied again. */
        fun decode(text: String?): List<ClipItem> {
            if (text.isNullOrBlank()) return emptyList()
            return try {
                json.parseToJsonElement(text).let { it as kotlinx.serialization.json.JsonArray }
                    .mapNotNull { runCatching { json.decodeFromJsonElement(ClipItem.serializer(), it) }.getOrNull() }
                    .filter { it.text.isNotEmpty() && ClipSync.utf8Size(it.text) <= ClipSync.MAX_BYTES }
                    .sortedByDescending { it.at }.let(::trim)
            } catch (_: Exception) { emptyList() }
        }
    }
}
