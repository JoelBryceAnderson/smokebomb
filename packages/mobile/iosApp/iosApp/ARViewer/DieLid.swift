import Foundation
import RealityKit

/// The lid, taken off the die: the charging face (−Y) and the lower half of
/// its four edges, split from the cup at the seam 45° round the bottom edge
/// (the desktop mockup's "Lid off", rev 2 joint).
///
/// Today's models group geometry by material, so the lid isn't a part of its
/// own: it's cut out of the meshes it shares with the cup (`Shell`,
/// `SapphireWindows`, …) by triangle, and the parts that are only lid
/// (`Etching`, the lid screws, the −Y screen, the board) move over whole. Per-
/// part models move their `Lid` prim. `restore()` puts every mesh and part
/// back exactly as it was.
///
/// `root` starts as a child of the reference entity with an identity
/// transform, so the lid sits on the die until it's moved.
@MainActor
final class DieLid {
    /// Where the cut is, in the die's own metres (origin at the bottom centre, Y up, lid down).
    struct Geometry: Equatable {
        /// Half the die's side.
        let half: Float
        /// The edge radius: 2.5 mm on the 34 mm die, in proportion on the others.
        let edgeRadius: Float
        /// The seam's height above the lid face, 45° round the bottom edge.
        var seamHeight: Float { edgeRadius * (1 - sin(.pi / 4)) }

        init(side: Float) {
            half = side / 2
            edgeRadius = 0.0025 * side / 0.034
        }

        /// Whether a triangle with this centre belongs to the lid: below the
        /// seam, or inside the lid's flat (its inner face is an edge radius
        /// up), away from the cup's walls and lower edges.
        func isLid(_ centre: SIMD3<Float>) -> Bool {
            if centre.y < seamHeight { return true }
            let tolerance: Float = 0.0002
            return centre.y < edgeRadius + tolerance
                && max(abs(centre.x), abs(centre.z)) < half - edgeRadius + tolerance
        }
    }

    /// Parts that come off with the lid whatever their shape.
    static let lidNames: Set<String> = [
        "Lid", "LidScrews", "ScrewSleevesAndSlots", "Etching",
        "Window_ny", "Module_ny", "Screen_ny", "LiveScreen_ny",
        "Board", "Internal_board",
    ]
    static let lidPrefixes = ["Screw_"]
    /// Parts that stay in the cup: the screw pillars run up from the lid's flat.
    static let cupNames: Set<String> = ["Internal_pillar"]
    static let cupPrefixes = ["Pillar_"]
    /// The seam's hairline only shows with the lid on.
    static let hiddenNames: Set<String> = ["LidSeam"]

    static func side(of name: String) -> Bool? {
        if lidNames.contains(name) || lidPrefixes.contains(where: name.hasPrefix) { return true }
        if cupNames.contains(name) || cupPrefixes.contains(where: name.hasPrefix) { return false }
        return nil
    }

    let root = Entity()
    let geometry: Geometry
    private let reference: Entity
    private var moved: [(entity: Entity, parent: Entity, transform: Transform, opacity: OpacityComponent?)] = []
    private var split: [(entity: Entity, original: ModelComponent)] = []
    private var hidden: [Entity] = []

    /// Cuts the lid out of `dieRoot`, a child of `reference`. Nil if nothing
    /// came off (a model without a lid face, like the line-up).
    init?(dieRoot: Entity, reference: Entity, side: Float) {
        self.reference = reference
        geometry = Geometry(side: side)
        root.name = "LidOff"
        reference.addChild(root)
        visit(dieRoot)
        guard !moved.isEmpty || !split.isEmpty else {
            restore()
            return nil
        }
    }

    /// Puts everything back on the die and drops the lid entity.
    func restore() {
        for item in moved.reversed() {
            item.parent.addChild(item.entity)
            item.entity.transform = item.transform
            if let opacity = item.opacity {
                item.entity.components.set(opacity)
            } else {
                item.entity.components.remove(OpacityComponent.self)
            }
        }
        for item in split { item.entity.components.set(item.original) }
        for entity in hidden { entity.isEnabled = true }
        moved.removeAll()
        split.removeAll()
        hidden.removeAll()
        root.removeFromParent()
    }

    // MARK: Cutting

    private func visit(_ entity: Entity) {
        if Self.hiddenNames.contains(entity.name) {
            if entity.isEnabled {
                entity.isEnabled = false
                hidden.append(entity)
            }
            return
        }
        switch Self.side(of: entity.name) {
        case true?:
            move(entity)
            return
        case false?:
            return
        case nil:
            break
        }
        if let model = entity.components[ModelComponent.self], cut(entity, model) { return }
        for child in Array(entity.children) { visit(child) }
    }

    /// Moves a whole part onto the lid, keeping where it is and how see-through.
    private func move(_ entity: Entity) {
        guard let parent = entity.parent else { return }
        let own = entity.components[OpacityComponent.self]
        let opacity = inheritedOpacity(from: parent) * (own?.opacity ?? 1)
        moved.append((entity, parent, entity.transform, own))
        root.addChild(entity, preservingWorldTransform: true)
        if opacity < 1 { entity.components.set(OpacityComponent(opacity: opacity)) }
    }

    /// The opacity an entity gets from its ancestors up to the reference.
    private func inheritedOpacity(from entity: Entity?) -> Float {
        var opacity: Float = 1
        var current = entity
        while let e = current, e !== reference {
            opacity *= e.components[OpacityComponent.self]?.opacity ?? 1
            current = e.parent
        }
        return opacity
    }

    /// Splits one mesh by triangle. True if the whole entity went to the lid
    /// (its children went with it).
    private func cut(_ entity: Entity, _ model: ModelComponent) -> Bool {
        let toReference = entity.transformMatrix(relativeTo: reference)
        let contents = model.mesh.contents
        var lid: (models: [MeshResource.Model], instances: [MeshResource.Instance]) = ([], [])
        var cup: (models: [MeshResource.Model], instances: [MeshResource.Instance]) = ([], [])
        var lidCount = 0, cupCount = 0

        for (i, instance) in contents.instances.enumerated() {
            guard let source = contents.models[instance.model] else { continue }
            let matrix = toReference * instance.transform
            var lidParts: [MeshResource.Part] = [], cupParts: [MeshResource.Part] = []
            for part in source.parts {
                let positions = part.positions.elements
                let indices = part.triangleIndices?.elements ?? Array(0..<UInt32(positions.count))
                var lidIndices: [UInt32] = [], cupIndices: [UInt32] = []
                var t = 0
                while t + 2 < indices.count {
                    let a = indices[t], b = indices[t + 1], c = indices[t + 2]
                    let centre = (positions[Int(a)] + positions[Int(b)] + positions[Int(c)]) / 3
                    let p = matrix * SIMD4(centre, 1)
                    if geometry.isLid(SIMD3(p.x, p.y, p.z)) {
                        lidIndices += [a, b, c]
                    } else {
                        cupIndices += [a, b, c]
                    }
                    t += 3
                }
                lidCount += lidIndices.count
                cupCount += cupIndices.count
                if !lidIndices.isEmpty {
                    var p = part
                    p.triangleIndices = MeshBuffer(lidIndices)
                    lidParts.append(p)
                }
                if !cupIndices.isEmpty {
                    var p = part
                    p.triangleIndices = MeshBuffer(cupIndices)
                    cupParts.append(p)
                }
            }
            let id = "\(instance.model)#\(i)"
            if !lidParts.isEmpty {
                lid.models.append(MeshResource.Model(id: id, parts: lidParts))
                lid.instances.append(MeshResource.Instance(id: id, model: id, at: instance.transform))
            }
            if !cupParts.isEmpty {
                cup.models.append(MeshResource.Model(id: id, parts: cupParts))
                cup.instances.append(MeshResource.Instance(id: id, model: id, at: instance.transform))
            }
        }

        if lidCount == 0 { return false }
        if cupCount == 0 {
            move(entity)
            return true
        }
        guard let lidMesh = Self.mesh(lid.models, lid.instances),
              let cupMesh = Self.mesh(cup.models, cup.instances)
        else { return false }

        split.append((entity, model))
        // Both halves show their insides now the die is open.
        let materials = model.materials.map(Self.doubleSided)
        var cupModel = model
        cupModel.mesh = cupMesh
        cupModel.materials = materials
        entity.components.set(cupModel)

        let fragment = Entity()
        fragment.name = "\(entity.name)_lid"
        var lidModel = model
        lidModel.mesh = lidMesh
        lidModel.materials = materials
        fragment.components.set(lidModel)
        root.addChild(fragment)
        fragment.setTransformMatrix(entity.transformMatrix(relativeTo: root), relativeTo: root)
        let opacity = inheritedOpacity(from: entity)
        if opacity < 1 { fragment.components.set(OpacityComponent(opacity: opacity)) }
        return false
    }

    private static func mesh(_ models: [MeshResource.Model], _ instances: [MeshResource.Instance]) -> MeshResource? {
        var contents = MeshResource.Contents()
        contents.models = MeshModelCollection(models)
        contents.instances = MeshInstanceCollection(instances)
        return try? MeshResource.generate(from: contents)
    }

    private static func doubleSided(_ material: any Material) -> any Material {
        if var pbr = material as? PhysicallyBasedMaterial {
            pbr.faceCulling = .none
            return pbr
        }
        if var unlit = material as? UnlitMaterial {
            unlit.faceCulling = .none
            return unlit
        }
        return material
    }
}
