package app.pill.phone.data

/** Which apps are mirrored: the user's explicit choice wins, otherwise a sensible default. */
object AppFilter {
    /** Messengers, mail and calls: on by default, even when the system ships them. */
    val KNOWN_ON: Set<String> = setOf(
        "org.telegram.messenger", "org.telegram.messenger.web", "org.thunderdog.challegram", "org.telegram.plus",
        "com.whatsapp", "com.whatsapp.w4b", "org.thoughtcrime.securesms", "com.viber.voip",
        "com.vkontakte.android", "com.vk.im", "ru.ok.android", "com.discord", "com.slack",
        "com.facebook.orca", "com.facebook.mlite", "com.instagram.android", "im.vector.app", "com.skype.raider",
        "com.tencent.mm", "jp.naver.line.android", "com.microsoft.teams", "us.zoom.videomeetings",
        "com.google.android.apps.messaging", "com.android.mms", "com.samsung.android.messaging",
        "com.google.android.gm", "ru.yandex.mail", "ru.mail.mailapp", "com.microsoft.office.outlook",
        "com.google.android.dialer", "com.android.dialer", "com.samsung.android.dialer", "com.android.incallui",
        "com.google.android.apps.tachyon", "com.google.android.calendar",
    )

    /** Never mirrored, whatever the switches say: ourselves. */
    const val SELF = "app.pill.phone"

    fun defaultEnabled(pkg: String, isSystem: Boolean): Boolean = when {
        pkg == SELF -> false
        pkg in KNOWN_ON -> true
        isSystem -> false
        else -> true
    }

    fun isEnabled(pkg: String, isSystem: Boolean, overrides: Map<String, Boolean>): Boolean =
        pkg != SELF && (overrides[pkg] ?: defaultEnabled(pkg, isSystem))
}
