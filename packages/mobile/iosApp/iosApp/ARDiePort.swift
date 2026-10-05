import ComposeApp
import Foundation

/// The die the Simulator tab runs, as the app's die links reach it
/// (`DiePort`, composeApp's `ArDieLink`): the firmware's phone link, passed
/// straight across as JSON on this phone, as the desktop simulator carries it
/// over its `/phone` WebSocket.
///
/// The die comes and goes with the Simulator tab (placed, live screens on);
/// the app's listener stays registered, hears `closed()` when there's no die,
/// and is greeted by the next one that starts.
@MainActor
final class ARDiePort: NSObject, @preconcurrency DiePort, DiePhoneLink {
    static let shared = ARDiePort()

    private var listener: DiePortListener?
    private weak var firmware: DieFirmware?

    // MARK: DiePort (the app)

    func open(listener: DiePortListener) {
        self.listener = listener
        guard let firmware else {
            listener.closed()
            return
        }
        firmware.setPhoneConnected(true)
        pump()
    }

    func close() {
        firmware?.setPhoneConnected(false)
        listener = nil
    }

    func send(message: String) -> Bool {
        guard let firmware, listener != nil else { return false }
        let taken = firmware.sendFromPhone(message)
        pump()
        return taken
    }

    // MARK: DiePhoneLink (the Simulator tab)

    func attach(_ next: DieFirmware?) {
        guard next !== firmware else { return }
        firmware = next
        guard let listener else { return }
        if let next {
            next.setPhoneConnected(true)
            pump()
        } else {
            listener.closed()
        }
    }

    func pump() {
        guard let firmware, let listener else { return }
        while let message = firmware.nextMessageForPhone() {
            listener.receive(message: message)
        }
    }
}
