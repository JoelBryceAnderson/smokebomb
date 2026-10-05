package com.smokebomb.app.ble

import com.smokebomb.shared.DieKind
import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

/**
 * Link to a Sugarcube over BLE. Common code only sees this interface; each
 * platform supplies an implementation through [createBleManager]
 * (CoreBluetooth on iOS, android.bluetooth.le on Android).
 *
 * GATT payloads are defined in `packages/shared/src/protocol.rs`.
 */
interface BleManager {
    val state: StateFlow<BleState>

    /** Signed rolls pushed by the connected die as they happen. */
    val rolls: Flow<SignedRoll>

    /** The connected die's modes, as it last reported them; null when not connected. */
    val inventory: StateFlow<Inventory?>

    fun startScan()
    fun stopScan()
    suspend fun connect(die: DiscoveredDie)
    suspend fun disconnect()

    /** Pull rolls made while the phone was away. */
    suspend fun syncHistory(sinceCounter: Long): List<SignedRoll>

    /**
     * Choose which licensed modes the die's Apps page offers
     * (`PhoneToDie::SetEnabledModes`). The die answers with a new [inventory].
     */
    suspend fun setEnabledModes(modes: Set<ModeId>)

    /** Set up the dice a throw rolls (`PhoneToDie::SetDie`); Pass the Pot switches to that game. */
    suspend fun setDie(kind: DieKind, count: Int)
}

data class DiscoveredDie(val id: String, val name: String, val rssi: Int)

sealed interface BleState {
    data object Off : BleState
    data object Idle : BleState
    data class Scanning(val found: List<DiscoveredDie>) : BleState
    data class Connecting(val die: DiscoveredDie) : BleState
    data class Connected(val die: DiscoveredDie, val batteryPercent: Int?) : BleState
    data class Error(val message: String) : BleState
}

/** Whatever the platform needs to reach its Bluetooth stack (Android `Context`, nothing on iOS). */
expect abstract class PlatformContext

expect fun createBleManager(context: PlatformContext): BleManager
