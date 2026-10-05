package com.smokebomb.app.ble

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
import kotlinx.coroutines.launch

/**
 * A "die" that is the desktop simulator, reached over the network instead of
 * Bluetooth. The simulator speaks the die's BLE messages as JSON on its
 * `/phone` WebSocket (`packages/simulator/server/src/phone.rs`).
 *
 * [address] is `host:port`: `localhost:3000` from the iOS simulator on the
 * same Mac, or the Mac's address (`my-mac.local:3000`) from a phone on the
 * same Wi-Fi, with the simulator started with `HOST=0.0.0.0`.
 */
class SimulatorLink(val address: String, private val scope: CoroutineScope) :
    JsonDieLink(DiscoveredDie(id = "simulator", name = "Simulator at $address", rssi = 0)) {
    private val client = HttpClient { install(WebSockets) }
    private var session: DefaultClientWebSocketSession? = null
    private var reader: Job? = null

    override suspend fun open(): String? {
        val s = try {
            client.webSocketSession("ws://$address/phone")
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            return "Couldn't reach the simulator at $address (${e.message})"
        }
        session = s
        reader = scope.launch {
            try {
                for (frame in s.incoming) {
                    if (frame is Frame.Text) received(frame.readText())
                }
            } finally {
                session = null
                lost("The simulator closed the connection")
            }
        }
        return null
    }

    override suspend fun closeTransport() {
        reader?.cancel()
        session?.close()
        session = null
    }

    override suspend fun sendText(text: String): Boolean {
        val s = session ?: return false
        s.send(Frame.Text(text))
        return true
    }

    /** Disconnect and free the HTTP client; the link can't be used again. */
    suspend fun close() {
        disconnect()
        client.close()
    }
}
