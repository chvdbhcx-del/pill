package app.pill.phone.ui

import androidx.compose.runtime.Immutable
import app.pill.phone.protocol.Msg

/** What the connection looks like to the user. */
enum class LinkUi { Unpaired, Connecting, Connected, NoNetwork, NotFound, Revoked }

@Immutable
data class PermissionsUi(
    val notificationAccess: Boolean = false,
    /** True also where the permission does not exist (before Android 13). */
    val postNotifications: Boolean = false,
    val batteryOptimization: Boolean = false,
    val wifiName: Boolean = false,
    /** True where it does not apply (Android 10+). */
    val storage: Boolean = true,
) {
    val required: Boolean get() = notificationAccess && postNotifications && batteryOptimization
    val missingCount: Int get() = listOf(notificationAccess, postNotifications, batteryOptimization).count { !it }
}

enum class TransferPhase { Offered, Waiting, Active, Done, Failed }

@Immutable
data class TransferUi(
    val id: String,
    val name: String,
    val sizeText: String,
    val outgoing: Boolean,
    val phase: TransferPhase,
    val progress: Float = 0f,
    /** Short reason for [TransferPhase.Failed], already user-readable. */
    val detail: String? = null,
    val canOpen: Boolean = false,
)

@Immutable
data class HomeUi(
    val link: LinkUi = LinkUi.Unpaired,
    val pcName: String? = null,
    val address: String? = null,
    val battery: Int? = null,
    val charging: Boolean = false,
    /** "wifi", "mobile" or "none". */
    val network: String = "none",
    val ssid: String? = null,
    val perms: PermissionsUi = PermissionsUi(),
    val transfers: List<TransferUi> = emptyList(),
    val mirroredApps: Int? = null,
    /** The newest status snapshot (battery details, storage, memory, network, modes), for the "This phone" card. */
    val status: Msg.Status? = null,
)

@Immutable
data class AppRowUi(val pkg: String, val label: String, val enabled: Boolean, val system: Boolean)

@Immutable
data class SettingsUi(
    val pcName: String? = null,
    val address: String? = null,
    val autoReceive: Boolean = false,
    val includeOngoing: Boolean = false,
    val perms: PermissionsUi = PermissionsUi(),
    val versionName: String = "1.0.0",
    /** "Pixel 8" and "15": what the PC is told about this phone. */
    val model: String = "",
    val androidVersion: String = "",
)

/** The pairing flow, one state at a time. */
sealed interface PairUi {
    /** Choose how to pair, or type the code. */
    data class Entry(val manual: Boolean = false, val address: String = "", val code: String = "", val error: PairError? = null) : PairUi
    data class Confirm(val address: String, val pcName: String?) : PairUi
    data class Working(val address: String) : PairUi
    data class Success(val pcName: String) : PairUi
    data class Failed(val error: PairError, val address: String?) : PairUi
}

enum class PairError { BadCode, Unreachable, InvalidLink, Refused }
