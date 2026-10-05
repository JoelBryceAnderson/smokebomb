import CoreGraphics
import Foundation
import simd

/// Tunable constants for throwing the die, and the maths for reading the result.
/// These are starting points that need tuning on a real table; see AR_VIEWER.md.
enum DiePhysics {
    // Surface: titanium on a wooden or laminate tabletop.
    static let staticFriction: Float = 0.45
    static let dynamicFriction: Float = 0.35
    /// How much of the speed a bounce keeps. Lower for a cloth or felt table.
    static let restitution: Float = 0.30
    /// RealityKit combines the two bodies' materials, so the table gets its own.
    static let tableStaticFriction: Float = 0.55
    static let tableDynamicFriction: Float = 0.45
    static let tableRestitution: Float = 0.30

    /// Air drag on the throw, per second.
    static let linearDamping: Float = 0.05
    /// Stands in for rolling resistance, which PhysX doesn't model; raise it if the die spins too long on an edge.
    static let angularDamping: Float = 0.6

    /// Throw speed along the table, in m/s, from a gentle flick to a hard one.
    static let throwSpeed: ClosedRange<Float> = 0.15...0.6
    /// Upward speed added to every throw, in m/s.
    static let throwLift: Float = 0.35
    /// Invisible walls this far around where a throw starts keep the die
    /// close, in metres; nil lets it roll anywhere.
    static let corralRadius: Float? = 0.18
    /// Height of those walls, in metres.
    static let corralHeight: Float = 0.15

    // Handling on the table. Linear acceleration is capped below the
    // firmware's shake threshold (0.7 g off 1 g) while the die is slid or
    // turned, and a dragged die eases toward the finger.
    static let handlingMaxLinearMg: Float = 400
    /// How quickly a dragged die catches up with the finger, per second.
    static let dragFollowRate: Float = 18

    // Held for the menu: lifted off the table with the menu's face toward you.
    /// How high the die's bottom floats above the table, in metres.
    static let heldHeight: Float = 0.08
    /// The most the held face tilts up toward the camera, in radians.
    static let heldMaxTilt: Float = 0.6
    /// Seconds to lift the die into the hand, or set it back down.
    static let heldMoveTime: TimeInterval = 0.5
    /// Seconds for one quarter turn from the turn pad (a menu tip).
    static let tipTime: TimeInterval = 0.35
    // The studio, with AR off: the desktop simulator's framing.
    /// The studio camera's vertical field of view, in degrees.
    static let studioFieldOfView: Float = 32
    /// How far the studio camera is from the die, in metres. The die stays
    /// put with AR off, so this frames it closely.
    static let studioCameraDistance: Float = 0.2
    /// Locked in place (AR off), a roll goes straight up this fast, in m/s.
    static let lockedTossLift: Float = 0.45
    /// …and lands inside walls this many die sides from its centre: room to
    /// tumble over an edge, not to wander off.
    static let lockedCorralFactor: Float = 1.4

    /// Space between the held die's edge and the glass turn pad, in metres.
    static let menuPanelGap: Float = 0.008
    /// How far the pad sits in front of the menu screen's plane, in metres
    /// (just clear of the sapphire).
    static let menuPanelLift: Float = 0.0005
    /// Tumble speed, in rad/s.
    static let throwSpin: ClosedRange<Float> = 10...30
    /// Flick speed, in points per second, that maps to the top of `throwSpeed`.
    static let flickForFullSpeed: CGFloat = 3000
    /// A drag on the die that ends faster than this (points per second) is a throw, not a move.
    static let flickThreshold: CGFloat = 900

    // The windup: before a throw the die is picked up and shaken in the air,
    // as a hand would. The firmware reads that from the IMU (Held, Shaking)
    // and only counts a release after it as a roll; the shake also fills the
    // smoke. Amplitudes keep the accelerometer between ~0.6 g and ~2.2 g, so
    // the shake never reads as free fall or as an impact.
    /// How high the die is lifted, in metres.
    static let windupLift: Float = 0.05
    /// Seconds to lift it (eased, so the push stays under ~0.35 g).
    static let windupLiftTime: TimeInterval = 0.3
    /// Seconds of shaking before release.
    static let windupShakeTime: TimeInterval = 0.5
    /// Shake amplitude along the table (x, z) and up (y), in metres: about
    /// 1.3 g on x and z (A·ω²) and 0.4 g up.
    static let shakeAmplitude = SIMD3<Float>(0.0075, 0.002, 0.0057)
    /// Shake frequency per axis, in Hz.
    static let shakeFrequency = SIMD3<Float>(6.5, 7.0, 7.5)
    /// How far the die rocks while shaken, in radians.
    static let shakeWobble: Float = 0.15

    /// Continuous collision detection: keeps a fast, small die from passing through the table.
    static let continuousCollisionDetection = true
    /// On LiDAR devices, let the scanned room mesh collide too, not just the placement plane.
    static let useSceneReconstruction = true

    // Settling.
    static let settleLinearSpeed: Float = 0.004  // m/s
    static let settleAngularSpeed: Float = 0.15  // rad/s
    static let settleTime: TimeInterval = 0.35
    /// Give up waiting after this long and read the face anyway.
    static let maxRollTime: TimeInterval = 8
    /// Below this, the die has fallen off the table; put it back.
    static let lostBelow: Float = -1.0

    /// The die's offset from its spot and its rocking, `t` seconds into the
    /// windup: an eased lift, then a shake that starts from rest (1 − cos).
    static func windup(at t: TimeInterval) -> (offset: SIMD3<Float>, rock: simd_quatf) {
        let u = Float(min(max(t / windupLiftTime, 0), 1))
        var offset = SIMD3<Float>(0, windupLift * u * u * (3 - 2 * u), 0)
        var rock = simd_quatf(angle: 0, axis: [0, 1, 0])
        if t > windupLiftTime {
            let ts = Float(t - windupLiftTime)
            let phase = 2 * Float.pi * shakeFrequency * ts
            offset += shakeAmplitude * (SIMD3<Float>(repeating: 1) - SIMD3(cos(phase.x), cos(phase.y), cos(phase.z)))
            let a = shakeWobble * sin(2 * .pi * 6 * ts)
            rock = simd_quatf(angle: a, axis: [1, 0, 0]) * simd_quatf(angle: a * 0.7, axis: [0, 0, 1])
        }
        return (offset, rock)
    }

    static func throwSpeed(forFlick pointsPerSecond: CGFloat) -> Float {
        let t = Float(min(max(pointsPerSecond / flickForFullSpeed, 0), 1))
        return throwSpeed.lowerBound + (throwSpeed.upperBound - throwSpeed.lowerBound) * t
    }
}

/// A face of the die, named as in the models: `Screen_px` … `Screen_nz`. `ny` is the lid.
enum DieFace: String, CaseIterable, Sendable {
    case px, nx, py, ny, pz, nz

    /// Outward normal in the model's own space (Y up).
    var normal: SIMD3<Float> {
        switch self {
        case .px: [1, 0, 0]
        case .nx: [-1, 0, 0]
        case .py: [0, 1, 0]
        case .ny: [0, -1, 0]
        case .pz: [0, 0, 1]
        case .nz: [0, 0, -1]
        }
    }

    var label: String {
        switch self {
        case .px: "+X"
        case .nx: "−X"
        case .py: "+Y (top)"
        case .ny: "−Y (lid)"
        case .pz: "+Z"
        case .nz: "−Z"
        }
    }

    /// The face whose normal is closest to world up, for a die with this world orientation.
    static func faceUp(orientation: simd_quatf) -> DieFace {
        let up = SIMD3<Float>(0, 1, 0)
        return allCases.max { simd_dot(orientation.act($0.normal), up) < simd_dot(orientation.act($1.normal), up) }!
    }
}

/// Decides when a thrown die has come to rest.
struct SettleDetector {
    private var still: TimeInterval = 0
    private(set) var elapsed: TimeInterval = 0

    mutating func reset() {
        still = 0
        elapsed = 0
    }

    /// Feed one frame's speeds; true once the die has been still for `settleTime`, or the roll has run too long.
    mutating func update(linearSpeed: Float, angularSpeed: Float, dt: TimeInterval) -> Bool {
        elapsed += dt
        if linearSpeed < DiePhysics.settleLinearSpeed && angularSpeed < DiePhysics.settleAngularSpeed {
            still += dt
        } else {
            still = 0
        }
        return still >= DiePhysics.settleTime || elapsed >= DiePhysics.maxRollTime
    }
}

/// The turn pad's axes, in the viewer's frame (the firmware's menu tips,
/// SIM_SPEC C3): about the viewer's right (tip up/down), about vertical
/// (tip left/right), and about the line of sight (a twist).
enum TurnAxis: Sendable {
    case pitch, yaw, roll
}

extension DiePhysics {
    /// The rotation taking die axes `a` and `b` (orthonormal) to `a2` and `b2`.
    static func rotation(from a: SIMD3<Float>, _ b: SIMD3<Float>, to a2: SIMD3<Float>, _ b2: SIMD3<Float>) -> simd_quatf {
        let local = simd_float3x3(columns: (a, b, simd_cross(a, b)))
        let target = simd_float3x3(columns: (a2, b2, simd_cross(a2, b2)))
        return simd_normalize(simd_quatf(target * local.transpose))
    }

    /// Of the die's six axes (in its frame), the one `orientation` turns
    /// closest to `direction`; returned in world space.
    static func nearestDieAxis(to direction: SIMD3<Float>, orientation: simd_quatf) -> SIMD3<Float> {
        DieFace.allCases.map { orientation.act($0.normal) }.max { simd_dot($0, direction) < simd_dot($1, direction) }!
    }

    /// Held for the menu: the menu's `front` face toward the camera, tilted up
    /// toward it by at most `heldMaxTilt`, and turned so whichever of its edges
    /// was most nearly up stays up. `toCamera` is from the die to the camera.
    static func heldOrientation(front: DieFace, current: simd_quatf, toCamera: SIMD3<Float>) -> simd_quatf {
        let up = SIMD3<Float>(0, 1, 0)
        var flat = SIMD3<Float>(toCamera.x, 0, toCamera.z)
        flat = simd_length(flat) < 1e-5 ? [0, 0, 1] : simd_normalize(flat)
        let tilt = min(max(atan2(toCamera.y, simd_length(SIMD2(toCamera.x, toCamera.z))), 0), heldMaxTilt)
        let facing = flat * cos(tilt) + up * sin(tilt)
        let upright = simd_normalize(up - facing * simd_dot(up, facing))
        let (right, screenUp) = front.screenAxes
        let edge = [screenUp, -screenUp, right, -right].max { simd_dot(current.act($0), up) < simd_dot(current.act($1), up) }!
        return rotation(from: front.normal, edge, to: facing, upright)
    }

    /// The glass turn pad's orientation: in the plane of the held die's `front`
    /// face, facing the same way, with the pad's up along whichever of that
    /// face's edges is most nearly up (the way the menu reads).
    static func menuPanelRotation(dieRotation: simd_quatf, front: DieFace) -> simd_quatf {
        let normal = dieRotation.act(front.normal)
        let (right, screenUp) = front.screenAxes
        let up = [screenUp, -screenUp, right, -right].map { dieRotation.act($0) }.max { $0.y < $1.y }!
        return rotation(from: [0, 0, 1], [0, 1, 0], to: normal, up)
    }

    /// Set down after the menu: whichever face is lowest goes flat on the
    /// table, keeping the die's heading.
    static func setDownOrientation(current: simd_quatf) -> simd_quatf {
        let down = DieFace.allCases.min { current.act($0.normal).y < current.act($1.normal).y }!
        let side = down.screenAxes.right
        var heading = current.act(side)
        heading.y = 0
        heading = simd_length(heading) < 1e-5 ? [1, 0, 0] : simd_normalize(heading)
        return rotation(from: down.normal, side, to: [0, -1, 0], heading)
    }
}
