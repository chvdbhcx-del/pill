package app.pill.phone.ui

import androidx.compose.runtime.Immutable
import app.pill.phone.net.RelayState
import app.pill.phone.net.Route
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
    /** Which PC the file goes to / comes from; set only when two or more PCs are paired. */
    val pc: String? = null,
)

/** One paired PC as the Home switcher shows it. */
@Immutable
data class PcUi(val pcId: String, val name: String, val link: LinkUi, val route: Route? = null)

/** One paired PC as the Settings "PCs" section shows it. */
@Immutable
data class PcSettingsUi(
    val pcId: String,
    /** The user's name for it (or the PC's own). */
    val name: String,
    /** What the PC calls itself. */
    val pcName: String,
    val address: String,
    val link: LinkUi,
    val route: Route? = null,
    /** Last time a session was up, ms; 0 = never. */
    val lastSeen: Long = 0,
    val mirror: Boolean = true,
    val autoReceive: Boolean = false,
    /** The PC has its own app filter instead of the global one. */
    val ownFilter: Boolean = false,
    val filterCount: Int? = null,
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
    /** Home network or internet, while connected. */
    val route: Route? = null,
    val relay: RelayState = RelayState.Standby,
    /** The "This phone" row shows its details. */
    val phoneExpanded: Boolean = false,
    /** Every paired PC; the switcher on the hero appears with two or more. [link], [pcName], [route]... describe [selectedPcId]. */
    val pcs: List<PcUi> = emptyList(),
    val selectedPcId: String? = null,
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
    /** "Use internet relay when away from home". */
    val relayEnabled: Boolean = true,
    val relay: RelayState = RelayState.Standby,
    val route: Route? = null,
    /** Cap of a file through the relay, MB. */
    val relayMaxMb: Int = 100,
    /** Every paired PC, for the "PCs" section (shown from two on; one PC keeps the single card). */
    val pcs: List<PcSettingsUi> = emptyList(),
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

enum class PairError { BadCode, Unreachable, InvalidLink, Refused, RelayMismatch }
