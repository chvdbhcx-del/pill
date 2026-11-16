package app.pill.phone

import app.pill.phone.data.UpdateFail
import app.pill.phone.data.UpdateManifest
import app.pill.phone.data.UpdatePolicy
import app.pill.phone.net.UpdateFailure
import app.pill.phone.net.copyVerified
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest

/** In-app updates: the manifest, the version rule, the 12-hour throttle, the host allow-list and the checked download stream. */
class UpdaterTest {
    private val sha = "a".repeat(64)
    private fun manifest(
        name: String = "\"1.1.0\"", code: String = "2", hash: String = "\"$sha\"", size: String = "4096", extra: String = "",
    ) = """{"versionName":$name,"versionCode":$code,"sha256":$hash,"size":$size$extra}"""

    // ---- version comparison ----

    private val m = UpdateManifest("1.1.0", 2, sha, 4096, minSdk = 26)

    @Test fun newerCodeIsOffered() = assertTrue(UpdatePolicy.offers(m, 1, 34))
    @Test fun sameCodeIsNot() = assertFalse(UpdatePolicy.offers(m, 2, 34))
    @Test fun olderCodeIsNeverOfferedAsDowngrade() = assertFalse(UpdatePolicy.offers(m, 7, 34))
    @Test fun codeBeyondIntRangeCompares() = assertTrue(UpdatePolicy.offers(m.copy(versionCode = 5_000_000_000), 4_999_999_999, 34))
    @Test fun buildThatNeedsANewerAndroidIsNotOffered() = assertFalse(UpdatePolicy.offers(m.copy(minSdk = 34), 1, 33))

    // ---- manifest parsing ----

    @Test fun validManifest() {
        val p = UpdatePolicy.parse(manifest(extra = ""","minSdk":26,"notes":"Updates from inside the app.""""))
        assertEquals(UpdateManifest("1.1.0", 2, sha, 4096, 26, "Updates from inside the app."), p)
    }

    @Test fun minimalManifestDefaults() {
        val p = UpdatePolicy.parse(manifest())!!
        assertEquals(1, p.minSdk)
        assertEquals("", p.notes)
    }

    @Test fun upperCaseHashIsNormalised() = assertEquals(sha, UpdatePolicy.parse(manifest(hash = "\"${sha.uppercase()}\""))!!.sha256)

    @Test fun unknownFieldsAreIgnored() =
        assertNotNull(UpdatePolicy.parse(manifest(extra = ""","channel":"beta","files":[1,2,{"a":null}],"x":{"y":1}""")))

    @Test fun nullOptionalFieldsAreFine() = assertNotNull(UpdatePolicy.parse(manifest(extra = ""","minSdk":null,"notes":null""")))

    @Test fun everyRequiredFieldIsRequired() {
        for (k in listOf("versionName", "versionCode", "sha256", "size")) {
            val without = Regex(""""$k":("[^"]*"|\d+),?""").replace(manifest(), "").replace(",}", "}")
            assertNull("missing $k", UpdatePolicy.parse(without))
        }
    }

    @Test fun wrongTypesAreRejected() {
        assertNull(UpdatePolicy.parse(manifest(code = "\"2\"")))          // a number written as text
        assertNull(UpdatePolicy.parse(manifest(code = "2.5")))
        assertNull(UpdatePolicy.parse(manifest(size = "\"4096\"")))
        assertNull(UpdatePolicy.parse(manifest(name = "1.1")))             // a number where text belongs
        assertNull(UpdatePolicy.parse(manifest(hash = "12345")))
        assertNull(UpdatePolicy.parse(manifest(extra = ""","notes":42""")))
        assertNull(UpdatePolicy.parse(manifest(extra = ""","minSdk":"26"""")))
    }

    @Test fun badValuesAreRejected() {
        assertNull(UpdatePolicy.parse(manifest(code = "0")))
        assertNull(UpdatePolicy.parse(manifest(code = "-3")))
        assertNull(UpdatePolicy.parse(manifest(size = "0")))
        assertNull(UpdatePolicy.parse(manifest(size = "${UpdatePolicy.MAX_APK_BYTES + 1}")))
        assertNull(UpdatePolicy.parse(manifest(hash = "\"${"a".repeat(63)}\"")))
        assertNull(UpdatePolicy.parse(manifest(hash = "\"${"g".repeat(64)}\"")))
        assertNull(UpdatePolicy.parse(manifest(name = "\"\"")))
        assertNull(UpdatePolicy.parse(manifest(name = "\"1.0 <script>\"")))
        assertNull(UpdatePolicy.parse(manifest(extra = ""","minSdk":0""")))
    }

    @Test fun notJsonObjectIsRejected() {
        for (s in listOf("", "   ", "null", "[]", "42", "\"x\"", "{", "{\"versionName\":", "<html>404</html>")) assertNull(s, UpdatePolicy.parse(s))
    }

    @Test fun hugeManifestIsRejected() =
        assertNull(UpdatePolicy.parse(manifest(extra = ""","pad":"${"x".repeat(UpdatePolicy.MAX_MANIFEST_BYTES.toInt())}"""")))

    @Test fun longNotesAreCutAndControlCharactersDropped() {
        val long = "n".repeat(5000)
        val p = UpdatePolicy.parse(manifest(extra = ""","notes":"$long\u0007""""))!!
        assertEquals(UpdatePolicy.MAX_NOTES, p.notes.length)
        assertEquals("a\nb", UpdatePolicy.parse(manifest(extra = ""","notes":"a\nb\u0000""""))!!.notes)
    }

    // ---- the 12-hour throttle ----

    private val h = 3_600_000L

    @Test fun neverCheckedIsDue() = assertTrue(UpdatePolicy.due(true, 0, 100 * h))
    @Test fun justCheckedIsNotDue() = assertFalse(UpdatePolicy.due(true, 100 * h, 100 * h + 5_000))
    @Test fun elevenHoursIsNotDue() = assertFalse(UpdatePolicy.due(true, 100 * h, 111 * h))
    @Test fun twelveHoursIsDue() = assertTrue(UpdatePolicy.due(true, 100 * h, 112 * h))
    @Test fun switchOffIsNeverDue() = assertFalse(UpdatePolicy.due(false, 0, 1000 * h))
    @Test fun clockMovedBackIsDue() = assertTrue(UpdatePolicy.due(true, 100 * h, 50 * h))

    // ---- hosts ----

    @Test fun githubHostsOverHttpsAreAllowed() {
        for (host in listOf("github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com", "GitHub.com")) {
            assertTrue(host, UpdatePolicy.hostAllowed("https", host))
        }
    }

    @Test fun everythingElseIsRefused() {
        assertFalse(UpdatePolicy.hostAllowed("http", "github.com"))               // no plain http, even to GitHub
        assertFalse(UpdatePolicy.hostAllowed("https", "evil.example"))
        assertFalse(UpdatePolicy.hostAllowed("https", "github.com.evil.example"))
        assertFalse(UpdatePolicy.hostAllowed("https", "evilgithub.com"))
        assertFalse(UpdatePolicy.hostAllowed("https", "raw.githubusercontent.com"))   // not on the list: only the release hosts
        assertFalse(UpdatePolicy.hostAllowed("https", "api.github.com"))
        assertFalse(UpdatePolicy.hostAllowed("ftp", "github.com"))
        assertFalse(UpdatePolicy.hostAllowed(null, "github.com"))
        assertFalse(UpdatePolicy.hostAllowed("https", null))
    }

    // ---- the checked stream ----

    private fun sha256(b: ByteArray) = MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }
    private val apk = ByteArray(300_000) { (it * 31 + 7).toByte() }

    private fun run(data: ByteArray, size: Long, hash: String, input: InputStream = ByteArrayInputStream(data), out: OutputStream = ByteArrayOutputStream()): UpdateFail? =
        try { copyVerified(input, out, size, hash); null } catch (e: UpdateFailure) { e.fail }

    @Test fun intactStreamPassesAndIsCopiedWhole() {
        val out = ByteArrayOutputStream()
        assertNull(run(apk, apk.size.toLong(), sha256(apk), out = out))
        assertTrue(apk.contentEquals(out.toByteArray()))
    }

    @Test fun progressEndsAtTheFullSize() {
        var last = 0L
        copyVerified(ByteArrayInputStream(apk), ByteArrayOutputStream(), apk.size.toLong(), sha256(apk)) { last = it }
        assertEquals(apk.size.toLong(), last)
    }

    @Test fun upperCaseHashMatches() = assertNull(run(apk, apk.size.toLong(), sha256(apk).uppercase()))

    @Test fun corruptByteIsRejected() {
        val bad = apk.copyOf().also { it[1234] = (it[1234] + 1).toByte() }
        assertEquals(UpdateFail.Mismatch, run(bad, apk.size.toLong(), sha256(apk)))
    }

    @Test fun shortStreamIsRejected() =
        assertEquals(UpdateFail.Mismatch, run(apk.copyOf(apk.size - 1), apk.size.toLong(), sha256(apk)))

    @Test fun longStreamIsCutOffAtOnce() {
        val out = ByteArrayOutputStream()
        val endless = object : InputStream() {
            var served = 0L
            override fun read(): Int { served++; return 1 }
            override fun read(b: ByteArray, off: Int, len: Int): Int { served += len; return len }
        }
        assertEquals(UpdateFail.Mismatch, run(apk, 100_000, sha256(apk), input = endless, out = out))
        assertTrue("stopped soon after the limit", endless.served < 100_000 + 2 * 64 * 1024)
        assertTrue("never wrote past the limit", out.size() <= 100_000)
    }

    @Test fun rightSizeButWrongHashIsRejected() = assertEquals(UpdateFail.Mismatch, run(apk, apk.size.toLong(), "0".repeat(64)))
    @Test fun unusableHashIsRejected() = assertEquals(UpdateFail.Mismatch, run(apk, apk.size.toLong(), "not-a-hash"))
    @Test fun emptyStreamIsRejected() = assertEquals(UpdateFail.Mismatch, run(ByteArray(0), 10, sha256(apk)))

    @Test fun brokenConnectionIsADownloadError() {
        val dropped = object : InputStream() {
            var n = 0
            override fun read(): Int = throw IOException("reset")
            override fun read(b: ByteArray, off: Int, len: Int): Int { if (n++ == 0) { b.fill(1, off, off + 10); return 10 }; throw IOException("reset") }
        }
        assertEquals(UpdateFail.Download, run(apk, apk.size.toLong(), sha256(apk), input = dropped))
    }

    @Test fun fullDiskIsAStorageError() {
        val full = object : OutputStream() {
            override fun write(b: Int) = throw IOException("ENOSPC")
            override fun write(b: ByteArray, off: Int, len: Int) = throw IOException("ENOSPC")
        }
        assertEquals(UpdateFail.Storage, run(apk, apk.size.toLong(), sha256(apk), out = full))
    }

    @Test fun failureCarriesNoRawExceptionText() {
        try { copyVerified(ByteArrayInputStream(apk), ByteArrayOutputStream(), 5, sha); fail() } catch (e: UpdateFailure) { assertEquals("Mismatch", e.message) }
    }
}
