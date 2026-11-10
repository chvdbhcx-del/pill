package app.pill.phone.service

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.util.Base64
import java.io.ByteArrayOutputStream

/** Small PNGs for the protocol's `icon` / `avatar` fields: base64, at most 96x96. */
object IconCodec {
    const val MAX = 96

    fun toBase64Png(d: Drawable, square: Boolean): String? = try {
        val bmp = render(d, square)
        val out = ByteArrayOutputStream(8 * 1024)
        bmp.compress(Bitmap.CompressFormat.PNG, 100, out)
        Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
    } catch (_: Exception) {
        null
    }

    fun render(d: Drawable, square: Boolean): Bitmap {
        val iw = if (d.intrinsicWidth > 0) d.intrinsicWidth else MAX
        val ih = if (d.intrinsicHeight > 0) d.intrinsicHeight else MAX
        // Fit inside MAX x MAX keeping the aspect ratio; app icons are drawn on the full square.
        val scale = MAX.toFloat() / maxOf(iw, ih)
        val w = if (square) MAX else maxOf(1, (iw * scale).toInt())
        val h = if (square) MAX else maxOf(1, (ih * scale).toInt())
        if (d is BitmapDrawable && !square && d.bitmap != null && iw <= MAX && ih <= MAX) return d.bitmap.copy(Bitmap.Config.ARGB_8888, false)
        val bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        d.setBounds(0, 0, w, h)
        d.draw(c)
        return bmp
    }
}
