#if GBC_CUBE
import Foundation
import GbcCube

/// A Game Boy Color game on the die, in place of the die firmware: the GBC
/// cube experiment (`experiments/gbc-cube`) behind the same `DieFirmware`
/// the Simulator tab already runs. The die's motion and touches go in, the
/// six faces come out; the tab doesn't know the difference.
///
/// Only built with `SUGARCUBE_GBC_CUBE = YES` in Local.xcconfig.
@MainActor
final class GbcCubeFirmware: DieFirmware {
    struct OpenError: Error {
        let message: String
    }

    private let cube: OpaquePointer
    let panelSide = 64
    let title: String
    /// Joypad buttons held on screen (`GC_KEY_*` bits), pressed on top of
    /// what the cube's touches and tilts press.
    var keys: UInt8 = 0

    /// Boots `rom` (its save goes next to it), or the built-in demo cart.
    init(rom: URL?) throws {
        let opened: OpaquePointer?
        if let rom {
            opened = rom.path.withCString { gc_cube_open($0) }
        } else {
            opened = gc_cube_open(nil)
        }
        guard let opened else {
            throw OpenError(message: Self.text { gc_cube_last_error($0, $1) })
        }
        cube = opened
        title = Self.text { gc_cube_title(opened, $0, $1) }
    }

    deinit {
        // Writes the save if the game changed it.
        gc_cube_free(cube)
    }

    func tick(_ imu: ImuReading, touchMask: UInt8) -> UInt64 {
        let a = imu.accelMg, g = imu.gyroMdps
        let sample = GcImu(
            accel_mg: (Int16(clamping: a.x), Int16(clamping: a.y), Int16(clamping: a.z)),
            gyro_mdps: (g.x, g.y, g.z))
        return gc_cube_tick(cube, sample, touchMask, keys)
    }

    func faceRGBA(_ face: DieFace, into buffer: inout [UInt8]) -> Bool {
        let count = buffer.count
        return buffer.withUnsafeMutableBufferPointer { out in
            gc_cube_face_rgba(cube, UInt8(face.index), out.baseAddress, count)
        }
    }

    func nextHaptic() -> Int? { nil }

    var mode: String { Self.text { gc_cube_status(cube, $0, $1) } }

    /// The game has no menu that lifts the die.
    var menuFront: DieFace? { nil }

    /// Text, menus and battles: `GC_UI_FRONT`, `GC_UI_PAN` or `GC_UI_SPREAD`.
    func setLayout(_ style: UInt8) {
        gc_cube_set_ui(cube, style)
    }

    /// Asks a C function for text: its length first, then the text.
    private static func text(_ read: (UnsafeMutablePointer<CChar>?, Int) -> Int) -> String {
        let length = read(nil, 0)
        guard length > 0 else { return "" }
        var buffer = [CChar](repeating: 0, count: length + 1)
        _ = buffer.withUnsafeMutableBufferPointer { read($0.baseAddress, $0.count) }
        return String(cString: buffer)
    }
}
#endif
