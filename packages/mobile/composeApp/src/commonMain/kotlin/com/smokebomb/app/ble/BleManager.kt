package com.smokebomb.app.ble

import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

/**
 * Link to a Smokebomb over BLE. Common code only sees this interface; each
 * platform supplies an implementation through [createBleManager]
 * (CoreBluetooth on iOS, android.bluetooth.le on Android).
 *
 * GATT payloads are defined in `packages/shared/src/protocol.rs`.
 */
interface BleManager {
    val state: StateFlow<BleState>

    /** Signed rolls pushed by the connected die as they happen. */
    val rolls: Flow<SignedRoll>

    fun startScan()
    fun stopScan()
    suspend fun connect(die: DiscoveredDie)
    suspend fun disconnect()

    /** Pull rolls made while the phone was away. */
    suspend fun syncHistory(sinceCounter: Long): List<SignedRoll>
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
