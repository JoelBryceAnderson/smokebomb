import Foundation
import simd

/// Where the die is: world space, metres, Y up.
struct DiePose: Equatable, Sendable {
    var position: SIMD3<Float>
    var orientation: simd_quatf
}

/// What the die's IMU reads as it moves, from its pose at each firmware tick.
/// The same sums as the desktop simulator's `World::imu`: the accelerometer
/// reads gravity's reaction plus linear acceleration, both turned into the
/// die's frame, and the gyro reads the turn since the last tick.
struct ImuSynth {
    static let tickHz: Double = 60
    static let tick: Float = Float(1 / tickHz)
    static let gravity: Float = 9.80665

    private var previous: [DiePose] = []

    /// Forget the motion so far: after a jump (placing, Reset), so the jump
    /// doesn't read as a huge acceleration.
    mutating func reset() {
        previous.removeAll()
    }

    /// The reading for the tick ending at `pose`. Poses must be one tick apart.
    /// `maxLinearMg` caps the linear acceleration: for a die being slid or
    /// turned on the table, where a finger's jitter would otherwise read as a
    /// shake. Nil (throws) reads it all.
    mutating func reading(at pose: DiePose, maxLinearMg: Float? = nil) -> ImuReading {
        defer {
            previous.append(pose)
            if previous.count > 2 { previous.removeFirst() }
        }
        let dt = Self.tick
        let toBody = pose.orientation.inverse

        // Linear acceleration from the last three positions, in milli-g.
        var linearMg = SIMD3<Float>.zero
        if previous.count == 2 {
            let a = (pose.position - 2 * previous[1].position + previous[0].position) / (dt * dt)
            linearMg = a / Self.gravity * 1000
            if let cap = maxLinearMg, simd_length(linearMg) > cap {
                linearMg = simd_normalize(linearMg) * cap
            }
        }
        // At rest the accelerometer reads +1 g toward the sky.
        let force = toBody.act(SIMD3<Float>(0, 1000, 0) + linearMg)

        var gyro = SIMD3<Float>.zero
        if let last = previous.last {
            let turn = pose.orientation * last.orientation.inverse
            var angle = turn.angle
            if angle > .pi { angle -= 2 * .pi }
            if abs(angle) > 1e-6 {
                let omegaWorld = simd_normalize(turn.axis) * (angle / dt)
                gyro = toBody.act(omegaWorld) * (180 / .pi * 1000)
            }
        }

        return ImuReading(
            accelMg: SIMD3(Self.clamp16(force.x), Self.clamp16(force.y), Self.clamp16(force.z)),
            gyroMdps: SIMD3(Self.clamp32(gyro.x), Self.clamp32(gyro.y), Self.clamp32(gyro.z)))
    }

    private static func clamp16(_ v: Float) -> Int32 {
        Int32(max(min(v.rounded(), Float(Int16.max)), Float(Int16.min)))
    }

    private static func clamp32(_ v: Float) -> Int32 {
        Int32(max(min(v.rounded(), 2e9), -2e9))
    }
}

/// Turns poses sampled at the render rate (60 or 120 Hz, with jitter) into
/// poses at exact firmware ticks, by interpolating between frames. Feeding
/// the firmware one tick per frame instead would make the same pose repeat
/// or skip, which reads as bursts of acceleration mid-flight.
struct TickClock {
    /// At most this many ticks per frame; after a stall the rest are dropped.
    static let maxTicksPerFrame = 8

    private var lastTick: TimeInterval = 0
    private var last: (time: TimeInterval, pose: DiePose)?

    mutating func reset() {
        last = nil
    }

    /// The die's pose at every tick between the last frame and this one.
    mutating func advance(to now: TimeInterval, pose: DiePose) -> [DiePose] {
        guard let previous = last, now > previous.time else {
            last = (now, pose)
            lastTick = now
            return []
        }
        let tick = 1 / ImuSynth.tickHz
        var poses: [DiePose] = []
        while lastTick + tick <= now + 1e-9 {
            lastTick += tick
            let u = Float((lastTick - previous.time) / (now - previous.time))
            let t = min(max(u, 0), 1)
            poses.append(DiePose(
                position: simd_mix(previous.pose.position, pose.position, SIMD3(repeating: t)),
                orientation: simd_slerp(previous.pose.orientation, pose.orientation, t)))
            if poses.count == Self.maxTicksPerFrame {
                lastTick = now
                break
            }
        }
        last = (now, pose)
        return poses
    }
}
