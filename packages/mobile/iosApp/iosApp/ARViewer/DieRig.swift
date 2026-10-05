import RealityKit
import UIKit

/// A loaded model plus what the viewer found in it: the per-part prims of the
/// naming contract (AR_VIEWER.md). Features whose prims are missing switch
/// off, so material-grouped models still work.
@MainActor
final class DieRig {
    /// Window travel and module travel at full explode, in metres along the face normal.
    static let windowTravel: Float = 0.026
    static let moduleTravel: Float = 0.014
    /// Shell opacity when x-ray is on for a per-part model.
    static let xrayShellOpacity: Float = 0.25

    let model: SugarcubeModel
    /// The loaded tree.
    let root: Entity
    /// Explode and picking work in this entity's space: the die's own metres, Y up, origin at the bottom centre.
    private let reference: Entity

    private(set) var shell: Entity?
    private var windows: [(entity: Entity, face: DieFace, base: SIMD3<Float>)] = []
    private var modules: [(entity: Entity, face: DieFace, base: SIMD3<Float>)] = []

    /// True when the model has Window_* or Module_* prims to explode.
    var canExplode: Bool { !windows.isEmpty || !modules.isEmpty }
    /// True when x-ray can fade this model's Shell instead of loading a separate x-ray file.
    var canFadeShell: Bool { shell != nil && canExplode }

    /// `root` must already be a child of `reference`.
    init(model: SugarcubeModel, root: Entity, reference: Entity) {
        self.model = model
        self.root = root
        self.reference = reference

        shell = root.findEntity(named: "Shell")
        for face in DieFace.allCases {
            if let w = root.findEntity(named: "Window_\(face.rawValue)") {
                windows.append((w, face, w.position(relativeTo: reference)))
            }
            if let m = root.findEntity(named: "Module_\(face.rawValue)") {
                modules.append((m, face, m.position(relativeTo: reference)))
            }
        }
    }

    // MARK: Explode and x-ray

    /// Moves windows and modules out along their face normals; `e` runs from 0 (assembled) to 1.
    func setExplode(_ e: Float) {
        let e = min(max(e, 0), 1)
        for w in windows {
            w.entity.setPosition(w.base + w.face.normal * Self.windowTravel * e, relativeTo: reference)
        }
        for m in modules {
            m.entity.setPosition(m.base + m.face.normal * Self.moduleTravel * e, relativeTo: reference)
        }
    }

    func setShellFaded(_ faded: Bool) {
        guard let shell else { return }
        if faded {
            shell.components.set(OpacityComponent(opacity: Self.xrayShellOpacity))
        } else {
            shell.components.remove(OpacityComponent.self)
        }
    }

    /// The model's visual bounds in its own metres (ignores the user's scale).
    func measuredSize() -> SIMD3<Float> {
        root.visualBounds(relativeTo: reference).extents
    }

    // MARK: Hit testing

    /// Whether a world-space ray passes through the model's bounds at all.
    func contains(origin: SIMD3<Float>, direction: SIMD3<Float>) -> Bool {
        let bounds = root.visualBounds(relativeTo: nil)
        return PartPicker.hitDistance(origin: origin, direction: direction, boxMin: bounds.min, boxMax: bounds.max) != nil
    }
}
