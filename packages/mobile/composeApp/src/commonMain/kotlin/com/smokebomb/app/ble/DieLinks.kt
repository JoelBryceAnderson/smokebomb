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
 * The link the app is using: the phone's Bluetooth, a [SimulatorLink] to the
 * desktop simulator, or an [ArDieLink] to the die in the Simulator tab (where
 * the platform has one: [arDie]). Screens talk to this and don't care which.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class DieLinks(
    private val bluetooth: BleManager,
    private val scope: CoroutineScope,
    private val arDie: DiePort? = null,
) : BleManager {
    private val current = MutableStateFlow(bluetooth)

    /** The simulator in use, or null on Bluetooth. */
    private val _simulator = MutableStateFlow<SimulatorLink?>(null)
    val simulator: StateFlow<SimulatorLink?> = _simulator.asStateFlow()

    /** Whether there's a Simulator tab die to connect to, on this platform. */
    val hasArDie: Boolean get() = arDie != null

    /** True while the app uses the Simulator tab's die. */
    private val _usingArDie = MutableStateFlow(false)
    val usingArDie: StateFlow<Boolean> = _usingArDie.asStateFlow()

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

    /** Switch to the die in the Simulator tab and connect to it. */
    suspend fun useArDie() {
        val port = arDie ?: return
        leave()
        val link = ArDieLink(port)
        _usingArDie.value = true
        current.value = link
        link.connect(DiscoveredDie("simulator-tab", "Simulator tab", 0))
    }

    suspend fun useBluetooth() {
        leave()
        current.value = bluetooth
    }

    private suspend fun leave() {
        when (val link = current.value) {
            is SimulatorLink -> link.close()
            is ArDieLink -> link.close()
            else -> link.disconnect()
        }
        _simulator.value = null
        _usingArDie.value = false
    }

    override fun startScan() = current.value.startScan()
    override fun stopScan() = current.value.stopScan()
    override suspend fun connect(die: DiscoveredDie) = current.value.connect(die)
    override suspend fun disconnect() = current.value.disconnect()
    override suspend fun syncHistory(sinceCounter: Long) = current.value.syncHistory(sinceCounter)
    override suspend fun setEnabledModes(modes: Set<ModeId>) = current.value.setEnabledModes(modes)
    override suspend fun setDie(kind: DieKind, count: Int) = current.value.setDie(kind, count)
}
