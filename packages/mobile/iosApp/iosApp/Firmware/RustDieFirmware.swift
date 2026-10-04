import Foundation
import SmokebombFirmware

/// The firmware core from `packages/firmware/ffi`, linked in as a static
/// library (built by `scripts/build_firmware.sh` before each app build).
@MainActor
final class RustDieFirmware: DieFirmware {
    private let die: OpaquePointer
    let panelSide: Int

    init?(panel: PanelKind) {
        guard let die = sb_die_new(panel == .grey96 ? 0 : 1) else { return nil }
        self.die = die
        panelSide = Int(sb_die_panel_side(die))
        sb_die_set_local_time(die, Self.secondsSinceMidnight())
    }

    deinit {
        sb_die_free(die)
    }

    func tick(_ imu: ImuReading, touchMask: UInt8) -> UInt64 {
        let a = imu.accelMg, g = imu.gyroMdps
        let sample = SbImu(
            accel_mg: (Int16(clamping: a.x), Int16(clamping: a.y), Int16(clamping: a.z)),
            gyro_mdps: (g.x, g.y, g.z))
        return sb_die_tick(die, sample, touchMask)
    }

    func faceRGBA(_ face: DieFace, into buffer: inout [UInt8]) -> Bool {
        let count = buffer.count
        return buffer.withUnsafeMutableBufferPointer { out in
            sb_die_face_rgba(die, UInt8(face.index), out.baseAddress, count)
        }
    }

    func nextHaptic() -> Int? {
        let effect = sb_die_next_haptic(die)
        return effect >= 0 ? Int(effect) : nil
    }

    var mode: String {
        var text = [CChar](repeating: 0, count: 48)
        _ = sb_die_mode(die, &text, text.count)
        return String(cString: text)
    }

    private static func secondsSinceMidnight() -> UInt32 {
        let now = Date()
        return UInt32(max(0, now.timeIntervalSince(Calendar.current.startOfDay(for: now))))
    }
}
