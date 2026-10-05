import RealityKit
import UIKit

/// A loaded model plus what the viewer found in it: the per-part prims of the
/// naming contract (AR_VIEWER.md), and the parts that have labels. Features whose
/// prims are missing switch off, so material-grouped models still work.
@MainActor
final class DieRig {
    /// Window travel and module travel at full explode, in metres along the face normal.
    static let windowTravel: Float = 0.026
    static let moduleTravel: Float = 0.014
    /// Shell opacity when x-ray is on for a per-part model.
    static let xrayShellOpacity: Float = 0.25

    /// Parts that x-ray looks through: a tap prefers whatever is behind them.
    /// Hits closer together than this (metres, along a unit ray) count as flush.
    private static let flushTolerance: Float = 0.0002

    private static let seeThrough: Set<String> = ["Shell", "SapphireWindows", "LidSeam", "ScrewSleevesAndSlots"]

    struct Part {
        let name: String
        let label: String
        let entity: Entity
    }

    let model: SugarcubeModel
    /// The loaded tree.
    let root: Entity
    /// Explode and picking work in this entity's space: the die's own metres, Y up, origin at the bottom centre.
    private let reference: Entity

    private(set) var shell: Entity?
    /// The laser etching on the lid (`Etching`), if this model shows one.
    private(set) var etching: Entity?
    /// The model's visual bounds as loaded, before the etching or explode.
    private let loadedSize: SIMD3<Float>
    private var windows: [(entity: Entity, face: DieFace, base: SIMD3<Float>)] = []
    private var modules: [(entity: Entity, face: DieFace, base: SIMD3<Float>)] = []
    /// Each mesh entity, mapped to the outermost labelled part that contains it.
    private var pickable: [(mesh: Entity, part: Part)] = []
    private var highlighted: (part: Part, materials: [ObjectIdentifier: [any Material]])?

    /// True when the model has Window_* or Module_* prims to explode.
    var canExplode: Bool { !windows.isEmpty || !modules.isEmpty }
    /// True when x-ray can fade this model's Shell instead of loading a separate x-ray file.
    var canFadeShell: Bool { shell != nil && canExplode }

    /// `root` must already be a child of `reference`.
    init(model: SugarcubeModel, root: Entity, reference: Entity, labels: PartLabels) {
        self.model = model
        self.root = root
        self.reference = reference
        loadedSize = root.visualBounds(relativeTo: reference).extents

        shell = root.findEntity(named: "Shell")
        for face in DieFace.allCases {
            if let w = root.findEntity(named: "Window_\(face.rawValue)") {
                windows.append((w, face, w.position(relativeTo: reference)))
            }
            if let m = root.findEntity(named: "Module_\(face.rawValue)") {
                modules.append((m, face, m.position(relativeTo: reference)))
            }
        }
        collectPickable(labels: labels)
        // Added after picking is collected: a tap on the lid names the lid.
        etching = Etching.attach(to: root, reference: reference, model: model)
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
        // X-ray looks through the shell, so its markings go too.
        etching?.isEnabled = !faded
        guard let shell else { return }
        if faded {
            shell.components.set(OpacityComponent(opacity: Self.xrayShellOpacity))
        } else {
            shell.components.remove(OpacityComponent.self)
        }
    }

    /// The model's visual bounds in its own metres (ignores the user's scale),
    /// as loaded: the etching's decal and explode don't count.
    func measuredSize() -> SIMD3<Float> {
        loadedSize
    }

    // MARK: Picking

    /// The labelled part under a world-space ray. With `seeThroughShell`, a part
    /// behind the shell wins over the shell itself.
    func pick(origin: SIMD3<Float>, direction: SIMD3<Float>, seeThroughShell: Bool) -> Part? {
        var hits: [(t: Float, volume: Float, part: Part)] = []
        for item in pickable {
            guard let bounds = item.mesh.components[ModelComponent.self]?.mesh.bounds else { continue }
            let worldFromLocal = item.mesh.transformMatrix(relativeTo: nil)
            let local = PartPicker.ray(origin: origin, direction: direction, into: worldFromLocal)
            if let t = PartPicker.hitDistance(origin: local.origin, direction: local.direction, boxMin: bounds.min, boxMax: bounds.max) {
                let scale = simd_length(SIMD3(worldFromLocal.columns.0.x, worldFromLocal.columns.0.y, worldFromLocal.columns.0.z))
                let e = bounds.extents * scale
                hits.append((t, e.x * e.y * e.z, item.part))
            }
        }
        if seeThroughShell, let inner = Self.nearest(hits.filter { !Self.isSeeThrough($0.part.name) }) {
            return inner
        }
        return Self.nearest(hits)
    }

    /// The nearest hit. A part set flush into a bigger one (a screw in the lid) ties with it; the smaller wins.
    private static func nearest(_ hits: [(t: Float, volume: Float, part: Part)]) -> Part? {
        guard let first = hits.min(by: { $0.t < $1.t }) else { return nil }
        return hits.filter { $0.t - first.t < flushTolerance }.min { $0.volume < $1.volume }?.part
    }

    /// Whether a world-space ray passes through the model's bounds at all.
    func contains(origin: SIMD3<Float>, direction: SIMD3<Float>) -> Bool {
        let bounds = root.visualBounds(relativeTo: nil)
        return PartPicker.hitDistance(origin: origin, direction: direction, boxMin: bounds.min, boxMax: bounds.max) != nil
    }

    /// Tints one part so it stands out; nil clears the highlight.
    func highlight(_ part: Part?) {
        if let old = highlighted {
            visitModels(old.part.entity) { entity, model in
                var model = model
                if let materials = old.materials[ObjectIdentifier(entity)] { model.materials = materials }
                entity.components.set(model)
            }
            highlighted = nil
        }
        guard let part else { return }
        var saved: [ObjectIdentifier: [any Material]] = [:]
        visitModels(part.entity) { entity, model in
            saved[ObjectIdentifier(entity)] = model.materials
            var model = model
            model.materials = model.materials.map(Self.glowing(_:))
            entity.components.set(model)
        }
        highlighted = (part, saved)
    }

    private static func glowing(_ material: any Material) -> any Material {
        let glow = UIColor(red: 0.2, green: 0.85, blue: 1.0, alpha: 1)
        if var pbr = material as? PhysicallyBasedMaterial {
            pbr.emissiveColor = .init(color: glow)
            pbr.emissiveIntensity = 0.8
            return pbr
        }
        return UnlitMaterial(color: glow)
    }

    private static func isSeeThrough(_ name: String) -> Bool {
        seeThrough.contains(name) || name.hasPrefix("Window_")
    }

    private func collectPickable(labels: PartLabels) {
        visitModels(root) { mesh, _ in
            // Walk up to the outermost labelled ancestor below the root, so a tap on a
            // module's screen names the module, as the contract's parts are the unit.
            var part: Part?
            var current: Entity? = mesh
            while let entity = current, entity !== root {
                if let label = labels.label(for: entity.name) {
                    part = Part(name: entity.name, label: label, entity: entity)
                }
                current = entity.parent
            }
            if let part { pickable.append((mesh, part)) }
        }
    }

    private func visitModels(_ entity: Entity, _ body: (Entity, ModelComponent) -> Void) {
        if let model = entity.components[ModelComponent.self] { body(entity, model) }
        for child in entity.children { visitModels(child, body) }
    }
}
