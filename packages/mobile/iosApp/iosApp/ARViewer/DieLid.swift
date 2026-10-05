import Foundation
import RealityKit
import UIKit

/// The lid, taken off the die: the charging face (−Y) and the lower half of
/// its four edges, split from the cup at the seam 45° round the bottom edge
/// (the desktop mockup's "Lid off", rev 2 joint).
///
/// Today's models group geometry by material, so the lid isn't a part of its
/// own. `DieLid` sorts every mesh by name (`rule(for:)`):
///
/// - the lid's own parts go whole: the board and everything on it, the −Y
///   screen, the etching, the screw sleeves;
/// - the cup's stay: the frame, the cell, the harness, the pillars;
/// - meshes with a piece on every face (windows, display modules, ribbons,
///   screws) give the lid the triangles nearest its face;
/// - the shell is cut at the seam.
///
/// The cut is closed as the mockup draws it: a taper from the seam in to the
/// edge's centre line on both halves, the cup's land round its mouth, and the
/// lid's flat plate with a tunnel down to its window. `restore()` puts every
/// mesh and part back exactly as it was.
///
/// `root` starts as a child of the reference entity with an identity
/// transform, so the lid sits on the die until it's moved.
@MainActor
final class DieLid {
    /// Where the cut is, in the die's own metres (origin at the bottom centre, Y up, lid down).
    struct Geometry: Equatable {
        /// The die's side.
        let side: Float
        /// The edge radius: 2.5 mm on the 34 mm die, in proportion on the others.
        let edgeRadius: Float

        init(side: Float) {
            self.side = side
            edgeRadius = 0.0025 * side / 0.034
        }

        var half: Float { side / 2 }
        /// The seam's height above the lid face, 45° round the bottom edge.
        var seamHeight: Float { edgeRadius * (1 - sin(.pi / 4)) }
        /// How far out from the edge's centre line the seam is.
        var seamInset: Float { edgeRadius * cos(.pi / 4) }
        /// Half the flat between the edges' centre lines: the lid's plate and the cup's mouth.
        var flatHalf: Float { half - edgeRadius }
        /// The lid's inside face: an edge radius up, where the joint runs flat across.
        var plateHeight: Float { edgeRadius }
        /// The window bore's floor: the back of the sapphire.
        var boreFloor: Float { 0.0008 }

        /// The lid's window (half its width and its corner radius), per die:
        /// 17.5 mm at 30 mm, 24 mm at 34 mm, 25.9 mm at 40 mm.
        var window: (half: Float, radius: Float) {
            let sizes: [(side: Float, half: Float, radius: Float)] = [
                (0.030, 0.00875, 0.00184), (0.034, 0.012, 0.0025), (0.040, 0.01295, 0.0027),
            ]
            let nearest = sizes.min { abs($0.side - side) < abs($1.side - side) }!
            let k = side / nearest.side
            return (nearest.half * k, nearest.radius * k)
        }

        /// The tunnel the lid's display module sits in, seen from inside: the
        /// carrier's flange, a millimetre round the window.
        var tunnel: (half: Float, radius: Float) {
            let w = window
            return (min(w.half + 0.001, flatHalf - 0.0005), w.radius + 0.001)
        }

        /// Whether a shell triangle with this centre belongs to the lid: below
        /// the seam, or inside the lid's flat, away from the cup's walls.
        func isLid(_ centre: SIMD3<Float>) -> Bool {
            if centre.y < seamHeight { return true }
            let tolerance: Float = 0.0002
            return centre.y < edgeRadius + tolerance
                && max(abs(centre.x), abs(centre.z)) < flatHalf + tolerance
        }

        /// The face a point is nearest, in the die's frame.
        func nearestFace(_ p: SIMD3<Float>) -> DieFace {
            let distances: [(DieFace, Float)] = [
                (.px, half - p.x), (.nx, half + p.x), (.py, side - p.y),
                (.ny, p.y), (.pz, half - p.z), (.nz, half + p.z),
            ]
            return distances.min { $0.1 < $1.1 }!.0
        }
    }

    enum Rule: Equatable {
        /// The whole part comes off with the lid.
        case lid
        /// The whole part stays in the cup.
        case cup
        /// A piece on every face: the lid takes the triangles nearest its face.
        case byFace
        /// Only shown with the lid on.
        case hidden
    }

    /// How a part is sorted; nil means cut at the seam (the shell).
    static func rule(for name: String) -> Rule? {
        switch name {
        case "LidSeam":
            return .hidden
        case "Lid", "Etching", "Window_ny", "Module_ny", "Screen_ny", "LiveScreen_ny", "Board",
             "ScrewSleevesAndSlots",
             // The board and what's on it or under it, and the charge contacts' sleeves and leads.
             "Internal_board", "Internal_ic", "Internal_lra", "Internal_ind",
             "Internal_w_charge", "Internal_w_12v", "Internal_sleeve":
            return .lid
        case "LidScrews", "SapphireWindows",
             "Internal_panel_glass", "Internal_encap", "Internal_chip", "Internal_fpc":
            return .byFace
        default:
            if name.hasPrefix("Screw_") { return .lid }
            // The frame, the cell and its lead, the screens' harness, the
            // tungsten, the pillars and their inserts: the cup's.
            if name.hasPrefix("Pillar_") || name.hasPrefix("Internal_") { return .cup }
            return nil
        }
    }

    let root = Entity()
    let geometry: Geometry
    private let reference: Entity
    /// What's added while the lid is off: the cup's mouth, and a die's borrowed internals.
    private var added: [Entity] = []
    private var moved: [(entity: Entity, parent: Entity, transform: Transform, opacity: OpacityComponent?)] = []
    private var split: [(entity: Entity, original: ModelComponent)] = []
    private var hidden: [Entity] = []

    /// Cuts the lid out of `dieRoot`, a child of `reference`. `internals` is
    /// the matching x-ray's root, for a die that has no insides of its own:
    /// its `Internal_*` parts are shown while the lid is off. `opacity` is
    /// the shell's (x-ray's see-through), for the joint's faces. Nil if
    /// nothing came off (a model without a lid face, like the line-up).
    init?(dieRoot: Entity, reference: Entity, side: Float, internals: Entity? = nil, opacity: Float = 1) {
        self.reference = reference
        geometry = Geometry(side: side)
        root.name = "LidOff"
        reference.addChild(root)
        if let internals { borrow(internals, into: dieRoot) }
        visit(dieRoot)
        guard !moved.isEmpty || !split.isEmpty else {
            restore()
            return nil
        }
        close(opacity: opacity)
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
        for entity in added { entity.removeFromParent() }
        moved.removeAll()
        split.removeAll()
        hidden.removeAll()
        added.removeAll()
        root.removeFromParent()
    }

    // MARK: Cutting

    /// Copies the x-ray's insides into the die, where they sit in the x-ray.
    private func borrow(_ internals: Entity, into dieRoot: Entity) {
        let holder = Entity()
        holder.name = "BorrowedInternals"
        dieRoot.addChild(holder)
        func find(_ entity: Entity) {
            if entity.name.hasPrefix("Internal_") {
                let copy = entity.clone(recursive: true)
                holder.addChild(copy)
                copy.setTransformMatrix(entity.transformMatrix(relativeTo: internals), relativeTo: dieRoot)
                return
            }
            for child in entity.children { find(child) }
        }
        find(internals)
        added.append(holder)
    }

    private func visit(_ entity: Entity) {
        switch Self.rule(for: entity.name) {
        case .hidden?:
            if entity.isEnabled {
                entity.isEnabled = false
                hidden.append(entity)
            }
            return
        case .lid?:
            move(entity)
            return
        case .cup?:
            return
        case .byFace?:
            let g = geometry
            cutAll(entity) { g.nearestFace($0) == .ny }
            return
        case nil:
            break
        }
        let g = geometry
        if let model = entity.components[ModelComponent.self], cut(entity, model, by: { g.isLid($0) }) { return }
        for child in Array(entity.children) { visit(child) }
    }

    /// Cuts an entity and every mesh under it.
    private func cutAll(_ entity: Entity, by isLid: (SIMD3<Float>) -> Bool) {
        if let model = entity.components[ModelComponent.self], cut(entity, model, by: isLid) { return }
        for child in Array(entity.children) { cutAll(child, by: isLid) }
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

    /// Splits one mesh by triangle, by where each triangle's centre is in the
    /// die's frame. True if the whole entity went to the lid (its children
    /// went with it).
    private func cut(_ entity: Entity, _ model: ModelComponent, by isLid: (SIMD3<Float>) -> Bool) -> Bool {
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
                    if isLid(SIMD3(p.x, p.y, p.z)) {
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

    // MARK: Closing the cut

    /// The joint's faces, as the mockup draws them: machined titanium.
    private func close(opacity: Float) {
        var material = PhysicallyBasedMaterial()
        material.baseColor = .init(tint: UIColor(red: 0.60, green: 0.62, blue: 0.64, alpha: 1))
        material.metallic = 1.0
        material.roughness = 0.42
        material.faceCulling = .none

        func add(_ meshes: [MeshDescriptor], name: String, to parent: Entity) -> Entity? {
            guard let mesh = try? MeshResource.generate(from: meshes) else { return nil }
            let entity = ModelEntity(mesh: mesh, materials: [material])
            entity.name = name
            if opacity < 1 { entity.components.set(OpacityComponent(opacity: opacity)) }
            parent.addChild(entity)
            return entity
        }
        let g = geometry
        // The lid: its tapered rim, then flat across, tunnelled for its display module.
        _ = add([Self.taper(g, lid: true)] + Self.plate(g), name: "LidPlate", to: root)
        // The cup: its tapered mouth and the land round the opening.
        if let mouth = add([Self.taper(g, lid: false), Self.land(g)], name: "CupMouth", to: reference) {
            added.append(mouth)
        }
    }

    /// The taper from the seam straight in to the edge's centre line, a cone
    /// round each corner. The lid's side faces out and up (toward the cup),
    /// the cup's in and down.
    static func taper(_ g: Geometry, lid: Bool) -> MeshDescriptor {
        let c = g.flatHalf, segments = 8
        var positions: [SIMD3<Float>] = [], normals: [SIMD3<Float>] = []
        let corners: [(x: Float, z: Float, start: Float)] = [(c, c, 0), (-c, c, .pi / 2), (-c, -c, .pi), (c, -c, .pi * 1.5)]
        for (cx, cz, start) in corners {
            for k in 0...segments {
                let t = start + Float(k) / Float(segments) * .pi / 2
                let out = SIMD3<Float>(cos(t), 0, sin(t))
                positions.append(SIMD3(cx, g.seamHeight, cz) + out * g.seamInset)
                positions.append(SIMD3(cx, g.plateHeight, cz))
                let n = simd_normalize(out + SIMD3(0, 1, 0)) * (lid ? 1 : -1)
                normals += [n, n]
            }
        }
        var indices: [UInt32] = []
        let n = positions.count / 2
        for k in 0..<n {
            let o0 = UInt32(2 * k), i0 = o0 + 1, o1 = UInt32(2 * ((k + 1) % n)), i1 = o1 + 1
            indices += lid ? [o0, i0, o1, o1, i0, i1] : [o0, o1, i0, o1, i1, i0]
        }
        var mesh = MeshDescriptor(name: lid ? "LidTaper" : "CupTaper")
        mesh.positions = MeshBuffer(positions)
        mesh.normals = MeshBuffer(normals)
        mesh.primitives = .triangles(indices)
        return mesh
    }

    /// The lid's flat inside face, the tunnel's walls down to the window bore,
    /// and the bore's floor round the window.
    static func plate(_ g: Geometry) -> [MeshDescriptor] {
        let tunnel = g.tunnel, window = g.window
        let hole = roundedOutline(half: tunnel.half, radius: tunnel.radius)
        let windowHole = roundedOutline(half: window.half, radius: window.radius)
        // The flat's square, sampled where the hole's points look out to it.
        let square = hole.map { p -> SIMD2<Float> in p * (g.flatHalf / max(abs(p.x), abs(p.y))) }
        let up = SIMD3<Float>(0, 1, 0)
        let top = ring(outer: square, inner: hole, y: g.plateHeight, normal: up, name: "LidFlat")
        let floor = ring(outer: hole, inner: windowHole, y: g.boreFloor, normal: up, name: "LidBoreFloor")

        var positions: [SIMD3<Float>] = [], normals: [SIMD3<Float>] = []
        var indices: [UInt32] = []
        let n = hole.count
        for (k, p) in hole.enumerated() {
            let inward = -simd_normalize(SIMD3(p.x, 0, p.y))
            positions += [SIMD3(p.x, g.boreFloor, p.y), SIMD3(p.x, g.plateHeight, p.y)]
            normals += [inward, inward]
            let a = UInt32(2 * k), b = UInt32(2 * ((k + 1) % n))
            indices += [a, b, a + 1, b, b + 1, a + 1]
        }
        var walls = MeshDescriptor(name: "LidTunnel")
        walls.positions = MeshBuffer(positions)
        walls.normals = MeshBuffer(normals)
        walls.primitives = .triangles(indices)
        return [top, walls, floor]
    }

    /// The cup's land: the flat ring round its mouth at the joint, facing the lid.
    static func land(_ g: Geometry) -> MeshDescriptor {
        let outer = roundedOutline(half: g.flatHalf, radius: 0)
        let inner = roundedOutline(half: g.flatHalf - 0.00035 * g.side / 0.030, radius: 0)
        return ring(outer: outer, inner: inner, y: g.plateHeight, normal: [0, -1, 0], name: "CupLand")
    }

    /// A flat ring between two outlines (x, z) that line up point for point.
    private static func ring(outer: [SIMD2<Float>], inner: [SIMD2<Float>], y: Float, normal: SIMD3<Float>, name: String) -> MeshDescriptor {
        let n = outer.count
        var indices: [UInt32] = []
        for k in 0..<n {
            let next = (k + 1) % n
            let (o0, o1, i0, i1) = (UInt32(k), UInt32(next), UInt32(n + k), UInt32(n + next))
            indices += normal.y > 0 ? [o0, i1, o1, o0, i0, i1] : [o0, o1, i1, o0, i1, i0]
        }
        var mesh = MeshDescriptor(name: name)
        mesh.positions = MeshBuffer((outer + inner).map { SIMD3($0.x, y, $0.y) })
        mesh.normals = MeshBuffer(Array(repeating: normal, count: 2 * n))
        mesh.primitives = .triangles(indices)
        return mesh
    }

    /// A rounded square's outline about the origin, 9 points a corner, so any
    /// two line up point for point (each corner's middle point on the diagonal).
    static func roundedOutline(half: Float, radius: Float) -> [SIMD2<Float>] {
        let r = max(0, min(radius, half)), c = half - r, segments = 8
        var outline: [SIMD2<Float>] = []
        let corners: [(centre: SIMD2<Float>, start: Float)] = [([c, c], 0), ([-c, c], .pi / 2), ([-c, -c], .pi), ([c, -c], .pi * 1.5)]
        for (centre, start) in corners {
            for k in 0...segments {
                let a = start + Float(k) / Float(segments) * .pi / 2
                outline.append(centre + r * SIMD2(cos(a), sin(a)))
            }
        }
        return outline
    }
}
