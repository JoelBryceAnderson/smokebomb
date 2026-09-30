package com.smokebomb.app.ble

import com.smokebomb.shared.DieKind
import com.smokebomb.shared.DieMessage
import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import com.smokebomb.shared.PhoneCodec
import com.smokebomb.shared.SignedRoll
import io.ktor.client.HttpClient
import io.ktor.client.plugins.websocket.DefaultClientWebSocketSession
import io.ktor.client.plugins.websocket.WebSockets
import io.ktor.client.plugins.websocket.webSocketSession
import io.ktor.websocket.Frame
import io.ktor.websocket.close
import io.ktor.websocket.readText
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull

/**
 * A "die" that is the desktop simulator, reached over the network instead of
 * Bluetooth. The simulator speaks the die's BLE messages as JSON on its
 * `/phone` WebSocket (`packages/simulator/server/src/phone.rs`).
 *
 * [address] is `host:port`: `localhost:3000` from the iOS simulator on the
 * same Mac, or the Mac's address (`my-mac.local:3000`) from a phone on the
 * same Wi-Fi, with the simulator started with `HOST=0.0.0.0`.
 */
class SimulatorLink(val address: String, private val scope: CoroutineScope) : BleManager {
    private val client = HttpClient { install(WebSockets) }
    private val die = DiscoveredDie(id = "simulator", name = "Simulator at $address", rssi = 0)

    private val _state = MutableStateFlow<BleState>(BleState.Idle)
    override val state: StateFlow<BleState> = _state.asStateFlow()

    private val _rolls = MutableSharedFlow<SignedRoll>(extraBufferCapacity = 16)
    override val rolls: Flow<SignedRoll> = _rolls.asSharedFlow()

    private val _inventory = MutableStateFlow<Inventory?>(null)
    override val inventory: StateFlow<Inventory?> = _inventory.asStateFlow()

    private var session: DefaultClientWebSocketSession? = null
    private var reader: Job? = null

    /** History items as they arrive; null ends a sync. */
    private val historyItems = Channel<SignedRoll?>(Channel.UNLIMITED)
    private val syncing = Mutex()

    /** There is nothing to scan for: the simulator is the one die in the list. */
    override fun startScan() {
        _state.value = BleState.Scanning(listOf(die))
    }

    override fun stopScan() {
        _state.value = BleState.Idle
    }

    override suspend fun connect(die: DiscoveredDie) {
        _state.value = BleState.Connecting(this.die)
        val s = try {
            client.webSocketSession("ws://$address/phone")
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _state.value = BleState.Error("Couldn't reach the simulator at $address (${e.message})")
            return
        }
        session = s
        // The die greets a new connection with Hello, which moves us to Connected.
        reader = scope.launch {
            try {
                for (frame in s.incoming) {
                    if (frame is Frame.Text) receive(frame.readText())
                }
            } finally {
                session = null
                _inventory.value = null
                if (_state.value !is BleState.Idle) {
                    _state.value = BleState.Error("The simulator closed the connection")
                }
            }
        }
    }

    override suspend fun disconnect() {
        _state.value = BleState.Idle
        reader?.cancel()
        session?.close()
        session = null
        _inventory.value = null
    }

    /** Disconnect and free the HTTP client; the link can't be used again. */
    suspend fun close() {
        disconnect()
        client.close()
    }

    /**
     * The simulator's kept rolls (its last 500) from [sinceCounter] on,
     * oldest first. Gives up after [SYNC_TIMEOUT_MS] with what arrived.
     */
    override suspend fun syncHistory(sinceCounter: Long): List<SignedRoll> = syncing.withLock {
        // Leftovers from a sync that timed out.
        while (historyItems.tryReceive().isSuccess) Unit
        val s = session ?: return emptyList()
        s.send(Frame.Text(PhoneCodec.syncHistory(sinceCounter)))
        val rolls = mutableListOf<SignedRoll>()
        withTimeoutOrNull(SYNC_TIMEOUT_MS) {
            while (true) rolls += historyItems.receive() ?: break
        }
        rolls
    }

    override suspend fun setEnabledModes(modes: Set<ModeId>) = send(PhoneCodec.setEnabledModes(modes))

    override suspend fun setDie(kind: DieKind, count: Int) = send(PhoneCodec.setDie(kind, count))

    private suspend fun send(text: String) {
        session?.send(Frame.Text(text))
    }

    private fun receive(text: String) {
        when (val m = PhoneCodec.decode(text)) {
            is DieMessage.Hello -> _state.value = BleState.Connected(die, m.batteryPercent)
            is DieMessage.Inventory -> _inventory.value = m.inventory
            is DieMessage.Roll -> _rolls.tryEmit(m.roll)
            is DieMessage.HistoryItem -> historyItems.trySend(m.roll)
            null -> Unit
        }
    }
}

private const val SYNC_TIMEOUT_MS = 10_000L
