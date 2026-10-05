package com.smokebomb.app.ble

import com.smokebomb.shared.DieKind
import com.smokebomb.shared.DieMessage
import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import com.smokebomb.shared.PhoneCodec
import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull

/**
 * A simulated die that speaks the die's BLE messages as JSON text, one
 * message at a time ([PhoneCodec]): the desktop simulator over a WebSocket
 * ([SimulatorLink]), or the die in the Simulator tab, on this phone
 * ([ArDieLink]). Subclasses supply the transport; the messages, the state and
 * the history sync are the same for both.
 */
abstract class JsonDieLink(protected val die: DiscoveredDie) : BleManager {
    private val _state = MutableStateFlow<BleState>(BleState.Idle)
    override val state: StateFlow<BleState> = _state.asStateFlow()

    private val _rolls = MutableSharedFlow<SignedRoll>(extraBufferCapacity = 16)
    override val rolls: Flow<SignedRoll> = _rolls.asSharedFlow()

    private val _inventory = MutableStateFlow<Inventory?>(null)
    override val inventory: StateFlow<Inventory?> = _inventory.asStateFlow()

    /** History items as they arrive; null ends a sync. */
    private val historyItems = Channel<SignedRoll?>(Channel.UNLIMITED)
    private val syncing = Mutex()

    /** Opens the transport; the die greets it with Hello. Returns why it couldn't, or null. */
    protected abstract suspend fun open(): String?

    protected abstract suspend fun closeTransport()

    /** Sends one message; false if the transport isn't open. */
    protected abstract suspend fun sendText(text: String): Boolean

    /** There is nothing to scan for: the simulated die is the one die in the list. */
    override fun startScan() {
        _state.value = BleState.Scanning(listOf(die))
    }

    override fun stopScan() {
        _state.value = BleState.Idle
    }

    override suspend fun connect(die: DiscoveredDie) {
        _state.value = BleState.Connecting(this.die)
        // The die greets a new connection with Hello, which moves us to Connected.
        open()?.let { _state.value = BleState.Error(it) }
    }

    override suspend fun disconnect() {
        _state.value = BleState.Idle
        closeTransport()
        _inventory.value = null
    }

    /**
     * The die's kept rolls (its last 500) from [sinceCounter] on, oldest
     * first. Gives up after [SYNC_TIMEOUT_MS] with what arrived.
     */
    override suspend fun syncHistory(sinceCounter: Long): List<SignedRoll> = syncing.withLock {
        // Leftovers from a sync that timed out.
        while (historyItems.tryReceive().isSuccess) Unit
        if (!sendText(PhoneCodec.syncHistory(sinceCounter))) return emptyList()
        val rolls = mutableListOf<SignedRoll>()
        withTimeoutOrNull(SYNC_TIMEOUT_MS) {
            while (true) rolls += historyItems.receive() ?: break
        }
        rolls
    }

    override suspend fun setEnabledModes(modes: Set<ModeId>) {
        sendText(PhoneCodec.setEnabledModes(modes))
    }

    override suspend fun setDie(kind: DieKind, count: Int) {
        sendText(PhoneCodec.setDie(kind, count))
    }

    /** One message from the die. */
    protected fun received(text: String) {
        when (val m = PhoneCodec.decode(text)) {
            is DieMessage.Hello -> _state.value = BleState.Connected(die, m.batteryPercent)
            is DieMessage.Inventory -> _inventory.value = m.inventory
            is DieMessage.Roll -> _rolls.tryEmit(m.roll)
            is DieMessage.HistoryItem -> historyItems.trySend(m.roll)
            null -> Unit
        }
    }

    /** The transport closed on its own: say why, unless we were the ones leaving. */
    protected fun lost(message: String) {
        _inventory.value = null
        if (_state.value !is BleState.Idle) _state.value = BleState.Error(message)
    }
}

private const val SYNC_TIMEOUT_MS = 10_000L
