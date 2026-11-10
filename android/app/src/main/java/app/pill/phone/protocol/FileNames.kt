package app.pill.phone.protocol

import java.util.Locale

object FileNames {
    private val BAD = Regex("[\\\\/:*?\"<>|\\u0000-\\u001F]")
    private val RESERVED = Regex("^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\\..*)?$", RegexOption.IGNORE_CASE)

    /**
     * A name from the PC is untrusted: no path parts, no characters Windows or Android dislike,
     * no leading dots (hidden files), bounded length, never empty. The extension survives a cut.
     */
    fun sanitize(raw: String?, fallback: String = "file"): String {
        var n = (raw ?: "").substringAfterLast('/').substringAfterLast('\\')
        n = BAD.replace(n, "_").trim().trimStart('.').trimEnd('.', ' ')
        if (n.isEmpty()) n = fallback
        if (RESERVED.matches(n)) n = "_$n"
        if (n.length > 120) {
            val ext = n.substringAfterLast('.', "").take(12)
            n = if (ext.isNotEmpty() && ext.length < n.length - 1) n.take(120 - ext.length - 1) + "." + ext else n.take(120)
        }
        return n
    }

    /** "photo.jpg" -> "photo (2).jpg", for the pre-Android-10 path that writes plain files. */
    fun withSuffix(name: String, n: Int): String {
        if (n <= 0) return name
        val dot = name.lastIndexOf('.')
        return if (dot > 0) "${name.substring(0, dot)} ($n)${name.substring(dot)}" else "$name ($n)"
    }

    fun humanSize(bytes: Long): String {
        if (bytes < 1024) return "$bytes B"
        val units = arrayOf("KB", "MB", "GB", "TB")
        var v = bytes.toDouble()
        var i = -1
        while (v >= 1024 && i < units.lastIndex) { v /= 1024; i++ }
        val fmt = if (v >= 100) "%.0f" else "%.1f"
        return "${String.format(Locale.ROOT, fmt, v)} ${units[i]}"
    }
}
