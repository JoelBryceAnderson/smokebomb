import simd

/// Which panels a die has, as the firmware builds them.
enum PanelKind: Sendable {
    /// The 34 mm die: 96×96, 16 grey levels.
    case grey96
    /// The 30 mm die: 64×64 RGB565.
    case rgb64
}

/// One IMU reading in the die's own frame (Y up, −Y is the lid), as the
/// firmware's `ImuSample` takes it.
struct ImuReading: Equatable, Sendable {
    /// Milli-g. At rest the face pointing up reads about +1000 on its axis.
    var accelMg: SIMD3<Int32>
    /// Milli-degrees per second.
    var gyroMdps: SIMD3<Int32>
}

/// The real firmware core, running on the phone. The app supplies it
/// (`iosApp/Firmware/RustDieFirmware.swift`, over the Rust C ABI); the viewer
/// only sees this, so it builds and tests without Rust.
@MainActor
protocol DieFirmware: AnyObject {
    var panelSide: Int { get }
    /// One firmware tick (1/60 s). Returns a counter that changes whenever
    /// the panels show something new.
    func tick(_ imu: ImuReading, touchMask: UInt8) -> UInt64
    /// One face's panel as RGBA8, top row first, in the face's own screen
    /// orientation (`orientation::BASES`). False if not drawn yet.
    func faceRGBA(_ face: DieFace, into buffer: inout [UInt8]) -> Bool
    /// The next haptic effect the firmware played, numbered as
    /// `smokebomb_hal::HapticEffect` is ordered.
    func nextHaptic() -> Int?
    /// The firmware's mode, for a debug readout.
    var mode: String { get }
    /// While the menu is open, the face it's read on (held toward the person).
    var menuFront: DieFace? { get }

    /// The phone link: the app's BLE messages as JSON, as on the desktop
    /// simulator's `/phone` WebSocket. Connecting, the die greets the app
    /// with Hello and its inventory.
    func setPhoneConnected(_ connected: Bool)
    /// One message from the app; false if it isn't one.
    func sendFromPhone(_ message: String) -> Bool
    /// The next message for the app, if any: answers, rolls, mode changes.
    func nextMessageForPhone() -> String?
}

extension DieFirmware {
    /// A firmware without a phone link (the tests' stand-ins).
    func setPhoneConnected(_ connected: Bool) {}
    func sendFromPhone(_ message: String) -> Bool { false }
    func nextMessageForPhone() -> String? { nil }
}

/// Connects the die the Simulator tab runs to the app, as if over BLE. The
/// app supplies it (`iosApp/iosApp/ARDiePort.swift`, which the app's die
/// links reach); nil leaves the die on its own, as in the tests.
@MainActor
protocol DiePhoneLink: AnyObject {
    /// The die running now, or nil when there's none.
    func attach(_ firmware: DieFirmware?)
    /// Passes on what the die has said since. Called after each step.
    func pump()
}

/// Makes a firmware for a panel kind; nil if it can't boot.
typealias DieFirmwareFactory = @MainActor (PanelKind) -> DieFirmware?

extension SugarcubeModel {
    /// The panels this model's die has, for live screens. The 40 mm die has no
    /// firmware target of its own yet, so it borrows the 30 mm colour one.
    /// The line-up has no single die to run.
    var panel: PanelKind? {
        switch kind {
        case .lineup: nil
        default: size == "34 mm" ? .grey96 : .rgb64
        }
    }
}

extension DieFace {
    /// The face's screen axes in the die's frame: the panel's right (x) and
    /// up (y), as the firmware's `orientation::BASES` defines them.
    var screenAxes: (right: SIMD3<Float>, up: SIMD3<Float>) {
        switch self {
        case .px: ([0, 0, -1], [0, 1, 0])
        case .nx: ([0, 0, 1], [0, 1, 0])
        case .py: ([1, 0, 0], [0, 0, -1])
        case .ny: ([1, 0, 0], [0, 0, 1])
        case .pz: ([1, 0, 0], [0, 1, 0])
        case .nz: ([-1, 0, 0], [0, 1, 0])
        }
    }

    /// Bit for this face in the firmware's touch mask (and its index there).
    var index: Int { DieFace.allCases.firstIndex(of: self)! }
}
