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
    static let throwSpeed: ClosedRange<Float> = 0.25...1.4
    /// Upward speed added to every throw, in m/s.
    static let throwLift: Float = 0.5
    /// Tumble speed, in rad/s.
    static let throwSpin: ClosedRange<Float> = 10...30
    /// Flick speed, in points per second, that maps to the top of `throwSpeed`.
    static let flickForFullSpeed: CGFloat = 3000
    /// A drag on the die that ends faster than this (points per second) is a throw, not a move.
    static let flickThreshold: CGFloat = 900
    /// How high the die is lifted before it's released, in metres.
    static let releaseHeight: Float = 0.02

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
