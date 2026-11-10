package app.pill.phone.transfer

import android.Manifest
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.core.content.ContextCompat
import app.pill.phone.protocol.FileNames
import java.io.File
import java.io.FileOutputStream
import java.io.OutputStream

/** A file being written into Downloads/Pill. Call [finish] on success or [abort] to remove the partial file. */
class SavedFile(val out: OutputStream, private val onFinish: ((Uri?) -> Unit) -> Unit, private val onAbort: () -> Unit) {
    fun finish(done: (Uri?) -> Unit) = onFinish(done)
    fun abort() = onAbort()
}

object Saver {
    private const val FOLDER = "Pill"

    fun guessMime(name: String, given: String): String {
        if (given.isNotBlank() && given != "application/octet-stream") return given
        val ext = name.substringAfterLast('.', "").lowercase()
        return MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
    }

    fun open(ctx: Context, name: String, mime: String): SavedFile =
        if (Build.VERSION.SDK_INT >= 29) viaMediaStore(ctx, name, guessMime(name, mime)) else viaFile(ctx, name)

    // Android 10+: scoped storage, no permission needed for our own Downloads files.
    @androidx.annotation.RequiresApi(29)
    private fun viaMediaStore(ctx: Context, name: String, mime: String): SavedFile {
        val resolver = ctx.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mime)
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/$FOLDER")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: throw java.io.IOException("insert failed")
        val out = resolver.openOutputStream(uri) ?: throw java.io.IOException("open failed")
        return SavedFile(
            out,
            onFinish = { done ->
                out.close()
                resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
                done(uri)
            },
            onAbort = {
                runCatching { out.close() }
                runCatching { resolver.delete(uri, null, null) }
            },
        )
    }

    // Android 8-9: plain file in the public Downloads (needs the storage permission), else the app's own folder.
    @Suppress("DEPRECATION")
    private fun viaFile(ctx: Context, name: String): SavedFile {
        val granted = ContextCompat.checkSelfPermission(ctx, Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED
        val base = if (granted) Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS) else ctx.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
        val dir = File(base ?: ctx.filesDir, FOLDER).apply { mkdirs() }
        var n = 0
        var f = File(dir, name)
        while (f.exists()) f = File(dir, FileNames.withSuffix(name, ++n))
        val out = FileOutputStream(f)
        return SavedFile(
            out,
            onFinish = { done ->
                out.close()
                MediaScannerConnection.scanFile(ctx, arrayOf(f.path), null) { _, uri -> done(uri) }
            },
            onAbort = { runCatching { out.close() }; f.delete() },
        )
    }
}
