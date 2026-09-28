package com.smokebomb.app.ble

import android.content.Context
import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.emptyFlow

actual typealias PlatformContext = Context

actual fun createBleManager(context: PlatformContext): BleManager = AndroidBleManager(context)

/**
 * Stub. Real implementation: `BluetoothLeScanner` filtered on
 * `SMOKEBOMB_SERVICE_UUID`, `BluetoothGatt` for the connection, runtime
 * permission requests for BLUETOOTH_SCAN / BLUETOOTH_CONNECT.
 */
class AndroidBleManager(@Suppress("unused") private val context: Context) : BleManager {
    private val _state = MutableStateFlow<BleState>(BleState.Idle)
    override val state: StateFlow<BleState> = _state.asStateFlow()
    override val rolls: Flow<SignedRoll> = emptyFlow()

    override fun startScan() {
        _state.value = BleState.Error("Android BLE is not implemented yet")
    }

    override fun stopScan() {
        _state.value = BleState.Idle
    }

    override suspend fun connect(die: DiscoveredDie) {
        _state.value = BleState.Error("Android BLE is not implemented yet")
    }

    override suspend fun disconnect() {
        _state.value = BleState.Idle
    }

    override suspend fun syncHistory(sinceCounter: Long): List<SignedRoll> = emptyList()
}
