package com.smokebomb.app.ble

/**
 * The die in the Simulator tab, on this phone, as a die the app connects to:
 * the same messages as the desktop simulator ([SimulatorLink]), passed
 * straight across through [port] instead of a WebSocket.
 */
class ArDieLink(private val port: DiePort) :
    JsonDieLink(DiscoveredDie(id = "simulator-tab", name = "The Simulator tab's die", rssi = 0)) {
    private val listener = object : DiePortListener {
        override fun receive(message: String) = fromDie(message)

        override fun closed() = dieGone()
    }

    private fun fromDie(message: String) = received(message)

    private fun dieGone() = lost(NO_DIE)

    override suspend fun open(): String? {
        port.open(listener)
        return null
    }

    override suspend fun closeTransport() = port.close()

    override suspend fun sendText(text: String): Boolean = port.send(text)

    /** Disconnect for good. */
    suspend fun close() = disconnect()

    private companion object {
        const val NO_DIE = "The Simulator tab's die isn't running. Open the Simulator tab, with Live on."
    }
}
