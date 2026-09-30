package com.smokebomb.app.ble

import com.smokebomb.shared.DieKind
import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.stateIn

/**
 * The link the app is using: the phone's Bluetooth, or a [SimulatorLink] to
 * the desktop simulator. Screens talk to this and don't care which.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class DieLinks(private val bluetooth: BleManager, private val scope: CoroutineScope) : BleManager {
    private val current = MutableStateFlow(bluetooth)

    /** The simulator in use, or null on Bluetooth. */
    private val _simulator = MutableStateFlow<SimulatorLink?>(null)
    val simulator: StateFlow<SimulatorLink?> = _simulator.asStateFlow()

    override val state: StateFlow<BleState> =
        current.flatMapLatest { it.state }.stateIn(scope, SharingStarted.Eagerly, bluetooth.state.value)
    override val inventory: StateFlow<Inventory?> =
        current.flatMapLatest { it.inventory }.stateIn(scope, SharingStarted.Eagerly, null)
    override val rolls: Flow<SignedRoll> = current.flatMapLatest { it.rolls }

    /** Switch to the simulator at `host:port` and connect to it. */
    suspend fun useSimulator(address: String) {
        leave()
        val link = SimulatorLink(address, scope)
        _simulator.value = link
        current.value = link
        link.connect(DiscoveredDie("simulator", address, 0))
    }

    suspend fun useBluetooth() {
        leave()
        _simulator.value = null
        current.value = bluetooth
    }

    private suspend fun leave() {
        val link = current.value
        if (link is SimulatorLink) link.close() else link.disconnect()
    }

    override fun startScan() = current.value.startScan()
    override fun stopScan() = current.value.stopScan()
    override suspend fun connect(die: DiscoveredDie) = current.value.connect(die)
    override suspend fun disconnect() = current.value.disconnect()
    override suspend fun syncHistory(sinceCounter: Long) = current.value.syncHistory(sinceCounter)
    override suspend fun setEnabledModes(modes: Set<ModeId>) = current.value.setEnabledModes(modes)
    override suspend fun setDie(kind: DieKind, count: Int) = current.value.setDie(kind, count)
}
