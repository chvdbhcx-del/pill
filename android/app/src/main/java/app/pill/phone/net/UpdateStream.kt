package app.pill.phone.net

import app.pill.phone.data.UpdateFail
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest

/** An update step failed for a reason the user can be told; [fail] picks the sentence. */
class UpdateFailure(val fail: UpdateFail, cause: Throwable? = null) : IOException(fail.name, cause)

/**
 * Copies [input] to [out] and checks it against the manifest while streaming: more than [size] bytes aborts at once (a replaced or
 * endless download never fills the disk), fewer or a different SHA-256 is rejected at the end. Network read errors and disk write
 * errors are told apart so the message can be "interrupted" or "no space". The caller deletes [out] on any [UpdateFailure].
 */
fun copyVerified(input: InputStream, out: OutputStream, size: Long, sha256: String, onProgress: (Long) -> Unit = {}) {
    val md = MessageDigest.getInstance("SHA-256")
    val buf = ByteArray(64 * 1024)
    var total = 0L
    while (true) {
        val n = try { input.read(buf) } catch (e: IOException) { throw UpdateFailure(UpdateFail.Download, e) }
        if (n < 0) break
        total += n
        if (total > size) throw UpdateFailure(UpdateFail.Mismatch)
        md.update(buf, 0, n)
        try { out.write(buf, 0, n) } catch (e: IOException) { throw UpdateFailure(UpdateFail.Storage, e) }
        onProgress(total)
    }
    try { out.flush() } catch (e: IOException) { throw UpdateFailure(UpdateFail.Storage, e) }
    if (total != size) throw UpdateFailure(UpdateFail.Mismatch)
    val got = md.digest()
    val want = hex(sha256) ?: throw UpdateFailure(UpdateFail.Mismatch)
    if (!MessageDigest.isEqual(got, want)) throw UpdateFailure(UpdateFail.Mismatch)
}

private fun hex(s: String): ByteArray? =
    if (s.length != 64 || !s.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) null
    else ByteArray(32) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
