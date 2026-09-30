package com.smokebomb.app.ble

import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.emptyFlow

/** iOS needs no context to reach CoreBluetooth; a singleton satisfies the expect. */
actual abstract class PlatformContext private constructor() {
    companion object INSTANCE : PlatformContext()
}

actual fun createBleManager(context: PlatformContext): BleManager = IosBleManager()

/**
 * Stub. Real implementation: `CBCentralManager` scanning for
 * `SMOKEBOMB_SERVICE_UUID`, `CBPeripheral` for GATT, state restoration for
 * background roll sync. Requires `NSBluetoothAlwaysUsageDescription`.
 */
class IosBleManager : BleManager {
    private val _state = MutableStateFlow<BleState>(BleState.Idle)
    override val state: StateFlow<BleState> = _state.asStateFlow()
    override val rolls: Flow<SignedRoll> = emptyFlow()
    override val inventory: StateFlow<Inventory?> = MutableStateFlow<Inventory?>(null).asStateFlow()

    override fun startScan() {
        _state.value = BleState.Scanning(emptyList())
    }

    override fun stopScan() {
        _state.value = BleState.Idle
    }

    override suspend fun connect(die: DiscoveredDie) {
        _state.value = BleState.Error("iOS BLE is not implemented yet")
    }

    override suspend fun disconnect() {
        _state.value = BleState.Idle
    }

    override suspend fun syncHistory(sinceCounter: Long): List<SignedRoll> = emptyList()

    override suspend fun setEnabledModes(modes: Set<ModeId>) = Unit
}
