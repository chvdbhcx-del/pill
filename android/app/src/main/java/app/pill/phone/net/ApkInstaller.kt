package app.pill.phone.net

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.pm.PackageInfoCompat
import app.pill.phone.data.UpdateFail
import app.pill.phone.data.UpdateManifest
import java.io.File

/** Installing a downloaded APK through the PackageInstaller session API. Android may show its own confirmation; that is never bypassed. */
object ApkInstaller {
    const val ACTION = "app.pill.phone.UPDATE_RESULT"

    fun canInstall(ctx: Context): Boolean = ctx.packageManager.canRequestPackageInstalls()

    /** The "install unknown apps" page of this very app. */
    fun permissionIntent(ctx: Context): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${ctx.packageName}"))

    fun installedCode(ctx: Context): Long =
        PackageInfoCompat.getLongVersionCode(ctx.packageManager.getPackageInfo(ctx.packageName, 0))

    /**
     * Reads the downloaded file as Android will: it must be this app, the published version, and carry the certificate of the installed one.
     * Android refuses another signature anyway; checking here turns its cryptic "app not installed" into a sentence.
     */
    @Suppress("DEPRECATION")
    fun preflight(ctx: Context, file: File, offer: UpdateManifest) {
        val pm = ctx.packageManager
        val flags = if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
        val archive = pm.getPackageArchiveInfo(file.path, flags) ?: throw UpdateFailure(UpdateFail.Mismatch)
        if (archive.packageName != ctx.packageName || PackageInfoCompat.getLongVersionCode(archive) != offer.versionCode) throw UpdateFailure(UpdateFail.Mismatch)
        fun signers(p: android.content.pm.PackageInfo) =
            (if (Build.VERSION.SDK_INT >= 28) p.signingInfo?.apkContentsSigners else p.signatures)?.map { it.toCharsString() }?.toSet().orEmpty()
        val mine = signers(pm.getPackageInfo(ctx.packageName, flags))
        val theirs = signers(archive)
        if (mine.isNotEmpty() && theirs.isNotEmpty() && mine != theirs) throw UpdateFailure(UpdateFail.Install)
    }

    /** Writes the file into a session and commits it; the outcome comes back to [UpdateResultReceiver]. */
    fun install(ctx: Context, file: File) {
        val installer = ctx.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(ctx.packageName)
            setSize(file.length())
            // only skips the prompt where Android allows it (12+, and this app updating itself); otherwise the usual confirmation appears
            if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        }
        val id = installer.createSession(params)
        try {
            installer.openSession(id).use { session ->
                file.inputStream().use { input ->
                    session.openWrite("PillPhone.apk", 0, file.length()).use { out ->
                        input.copyTo(out)
                        session.fsync(out)
                    }
                }
                val result = Intent(ctx, UpdateResultReceiver::class.java).setAction(ACTION).setPackage(ctx.packageName)
                // mutable: the system adds the status and the confirmation intent to it
                val pi = PendingIntent.getBroadcast(ctx, id, result, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE)
                session.commit(pi.intentSender)
            }
        } catch (e: Exception) {
            try { installer.abandonSession(id) } catch (_: Exception) { }
            throw UpdateFailure(if (e is java.io.IOException) UpdateFail.Storage else UpdateFail.Install, e)
        }
    }
}

/** The installer's answers: "needs your confirmation" (shown), done, aborted by the user, or failed. */
class UpdateResultReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        @Suppress("DEPRECATION")
        val confirm = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java) else intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
        Updater.onInstallStatus(ctx.applicationContext, status, confirm)
    }
}
