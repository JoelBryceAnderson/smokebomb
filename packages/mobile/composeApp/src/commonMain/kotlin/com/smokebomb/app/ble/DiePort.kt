package com.smokebomb.app.ble

/**
 * The die in the Simulator tab, as the app reaches it: the real firmware,
 * running on this phone (iOS: `iosApp/iosApp/ARDiePort.swift`). Messages are
 * the die's BLE messages as JSON text, as on the desktop simulator's
 * `/phone` WebSocket.
 */
interface DiePort {
    /**
     * Connect. The die greets [listener] with Hello and its inventory, and
     * from then on sends it answers, rolls and mode changes. While there's no
     * die (the Simulator tab hasn't started one, or live screens are off) the
     * port calls [DiePortListener.closed]; the listener stays registered, and
     * the next die to start greets it.
     */
    fun open(listener: DiePortListener)

    /** Disconnect; the listener hears nothing more. */
    fun close()

    /** One message for the die; false if there's no die to take it. */
    fun send(message: String): Boolean
}

interface DiePortListener {
    fun receive(message: String)

    /** The die stopped, or there isn't one yet. */
    fun closed()
}
