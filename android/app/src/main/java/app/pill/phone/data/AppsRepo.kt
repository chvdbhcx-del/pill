package app.pill.phone.data

import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.util.LruCache
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import app.pill.phone.service.IconCodec
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

data class InstalledApp(val pkg: String, val label: String, val system: Boolean)

/** The installed apps shown on the filter screen (those with a launcher entry, which are the ones that post notifications). */
object AppsRepo {
    private val icons = LruCache<String, ImageBitmap>(200)

    suspend fun load(ctx: Context): List<InstalledApp> = withContext(Dispatchers.IO) {
        val pm = ctx.packageManager
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        pm.queryIntentActivities(launcher, 0)
            .map { it.activityInfo.applicationInfo }
            .distinctBy { it.packageName }
            .filter { it.packageName != AppFilter.SELF }
            .map {
                val system = it.flags and ApplicationInfo.FLAG_SYSTEM != 0 && it.flags and ApplicationInfo.FLAG_UPDATED_SYSTEM_APP == 0
                InstalledApp(it.packageName, pm.getApplicationLabel(it).toString(), system)
            }
            .sortedBy { it.label.lowercase() }
    }

    suspend fun icon(ctx: Context, pkg: String): ImageBitmap? {
        icons.get(pkg)?.let { return it }
        return withContext(Dispatchers.IO) {
            try {
                IconCodec.render(ctx.packageManager.getApplicationIcon(pkg), square = true).asImageBitmap().also { icons.put(pkg, it) }
            } catch (_: Exception) {
                null
            }
        }
    }
}
