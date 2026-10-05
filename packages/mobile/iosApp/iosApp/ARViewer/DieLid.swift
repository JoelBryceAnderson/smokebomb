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
/// - the cup's stay: the frame, the cell, the pillars;
/// - meshes with a piece on every face (windows, display modules, ribbons)
///   give the lid the triangles nearest its face;
/// - the four screws (and the slots in their heads) each get an entity of
///   their own (`screws`), so they can come out before the lid does;
/// - the wiring that plugs into the board (the screens' harness, the cell's
///   lead) is hidden, as it's unplugged;
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
        /// The lid's screws: each comes out on its own.
        case screws
        /// Only shown with the lid on.
        case hidden
    }

    /// How a part is sorted; nil means cut at the seam (the shell).
    nonisolated static func rule(for name: String) -> Rule? {
        switch name {
        // The seam's hairline, and the wiring that plugs into the board and
        // would hang loose with it gone: the screens' harness and the cell's lead.
        case "LidSeam", "Internal_w_batt":
            return .hidden
        case "Lid", "Etching", "Window_ny", "Module_ny", "Screen_ny", "LiveScreen_ny", "Board",
             // The board and what's on it or under it, and the charge contacts' sleeves and leads.
             "Internal_board", "Internal_ic", "Internal_lra", "Internal_ind",
             "Internal_w_charge", "Internal_w_12v", "Internal_sleeve":
            return .lid
        case "SapphireWindows", "Internal_panel_glass", "Internal_encap", "Internal_chip", "Internal_fpc":
            return .byFace
        // The screws, and the slots in their heads (the sleeves round them stay in the lid).
        case "LidScrews", "ScrewSleevesAndSlots":
            return .screws
        default:
            if name.hasPrefix("Screw_") { return .screws }
            if name.hasPrefix("Internal_w_spi") { return .hidden }
            // The frame, the cell, the tungsten, the pillars and their
            // inserts: the cup's.
            if name.hasPrefix("Pillar_") || name.hasPrefix("Internal_") { return .cup }
            return nil
        }
    }

    /// Where a piece of a mesh goes.
    private enum Destination: Hashable {
        case cup, lid, screw(Int)
    }

    let root = Entity()
    let geometry: Geometry
    /// The four lid screws, each about its own axis: an entity at the screw
    /// head's centre on the lid face, in the die's frame (Y along the screw,
    /// into the die), starting on the reference entity where it sits.
    private(set) var screws: [Entity] = []
    /// Where each screw sits on the die: its head's centre, in the die's frame.
    private(set) var screwSeats: [SIMD3<Float>] = []
    private let reference: Entity
    /// What's added while the lid is off: the cup's mouth, a die's borrowed internals, the screws.
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
        findScrews(in: dieRoot)
        visit(dieRoot)
        guard !moved.isEmpty || !split.isEmpty else {
            restore()
            return nil
        }
        addShanks()
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
        screws.removeAll()
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

    /// Finds the four screws (by corner, from the lid face's screw heads or
    /// the per-part `Screw_*` prims) and makes an entity for each.
    private func findScrews(in dieRoot: Entity) {
        var sums: [SIMD2<Float>] = Array(repeating: .zero, count: 4), counts = [Int](repeating: 0, count: 4)
        let g = geometry
        func add(_ p: SIMD3<Float>) {
            let q = Self.corner(p)
            sums[q] += SIMD2(p.x, p.z)
            counts[q] += 1
        }
        func find(_ entity: Entity) {
            if entity.name == "LidScrews", let model = entity.components[ModelComponent.self] {
                forEachTriangle(entity, model) { if g.nearestFace($0) == .ny { add($0) } }
            } else if entity.name.hasPrefix("Screw_") {
                add(entity.visualBounds(relativeTo: reference).center)
                return
            }
            for child in entity.children { find(child) }
        }
        find(dieRoot)
        guard counts.allSatisfy({ $0 > 0 }) else { return }
        for q in 0..<4 {
            let centre = sums[q] / Float(counts[q])
            let seat = SIMD3<Float>(centre.x, 0, centre.y)
            let screw = Entity()
            screw.name = "LidScrew_\(q)"
            reference.addChild(screw)
            screw.position = seat
            screws.append(screw)
            screwSeats.append(seat)
            added.append(screw)
        }
    }

    /// The dice model only the screws' heads, flush in the lid; out of their
    /// holes they need their shanks. M1.0 (0.5 mm radius), from under the
    /// head to the insert, 4.5 mm down at 30 mm, as the x-ray has them, in
    /// the head's own finish.
    private func addShanks() {
        let k = geometry.side / 0.030
        let top: Float = 0.0003 * k, bottom: Float = 0.0045 * k
        for screw in screws where screw.visualBounds(relativeTo: screw).max.y < 0.001 * k {
            guard let material = Self.firstMaterial(screw) else { continue }
            let shank = ModelEntity(mesh: .generateCylinder(height: bottom - top, radius: 0.0005 * k), materials: [material])
            shank.name = "Shank"
            shank.position = [0, (top + bottom) / 2, 0]
            screw.addChild(shank)
        }
    }

    private static func firstMaterial(_ entity: Entity) -> (any Material)? {
        if let material = entity.components[ModelComponent.self]?.materials.first { return material }
        for child in entity.children {
            if let material = firstMaterial(child) { return material }
        }
        return nil
    }

    /// The corner a point is in: 0 (+x +z), 1 (+x −z), 2 (−x +z), 3 (−x −z).
    private static func corner(_ p: SIMD3<Float>) -> Int {
        (p.x >= 0 ? 0 : 2) + (p.z >= 0 ? 0 : 1)
    }

    /// The screw a point belongs to, if it's within a screw head of one.
    private func screw(near p: SIMD3<Float>) -> Int? {
        let radius = 0.00085 * geometry.side / 0.030
        let q = Self.corner(p)
        guard q < screwSeats.count else { return nil }
        let seat = screwSeats[q]
        return simd_length(SIMD2(p.x - seat.x, p.z - seat.z)) < radius ? q : nil
    }

    private func visit(_ entity: Entity) {
        let g = geometry
        switch Self.rule(for: entity.name) {
        case .hidden?:
            if entity.isEnabled {
                entity.isEnabled = false
                hidden.append(entity)
            }
            return
        case .lid?:
            move(entity, to: root)
            return
        case .cup?:
            return
        case .byFace?:
            cutAll(entity) { g.nearestFace($0) == .ny ? .lid : .cup }
            return
        case .screws?:
            if entity.name.hasPrefix("Screw_"), !screws.isEmpty {
                move(entity, to: screws[Self.corner(entity.visualBounds(relativeTo: reference).center)])
                return
            }
            let slots = entity.name == "ScrewSleevesAndSlots"
            cutAll(entity) { p in
                if let q = self.screw(near: p), slots || g.nearestFace(p) == .ny { return .screw(q) }
                // A sleeve stays in the lid; a screw with nowhere to go goes with the lid.
                return slots || g.nearestFace(p) == .ny ? .lid : .cup
            }
            return
        case nil:
            break
        }
        if let model = entity.components[ModelComponent.self], cut(entity, model, by: { g.isLid($0) ? .lid : .cup }) { return }
        for child in Array(entity.children) { visit(child) }
    }

    /// Cuts an entity and every mesh under it.
    private func cutAll(_ entity: Entity, by place: (SIMD3<Float>) -> Destination) {
        if let model = entity.components[ModelComponent.self], cut(entity, model, by: place) { return }
        for child in Array(entity.children) { cutAll(child, by: place) }
    }

    private func parent(for destination: Destination) -> Entity? {
        switch destination {
        case .cup: return nil
        case .lid: return root
        case .screw(let q): return q < screws.count ? screws[q] : root
        }
    }

    /// Moves a whole part, keeping where it is and how see-through.
    private func move(_ entity: Entity, to destination: Entity) {
        guard let parent = entity.parent else { return }
        let own = entity.components[OpacityComponent.self]
        let opacity = inheritedOpacity(from: parent) * (own?.opacity ?? 1)
        moved.append((entity, parent, entity.transform, own))
        destination.addChild(entity, preservingWorldTransform: true)
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

    /// Calls `body` with each triangle's centre, in the die's frame.
    private func forEachTriangle(_ entity: Entity, _ model: ModelComponent, _ body: (SIMD3<Float>) -> Void) {
        let toReference = entity.transformMatrix(relativeTo: reference)
        let contents = model.mesh.contents
        for instance in contents.instances {
            guard let source = contents.models[instance.model] else { continue }
            let matrix = toReference * instance.transform
            for part in source.parts {
                let positions = part.positions.elements
                let indices = part.triangleIndices?.elements ?? Array(0..<UInt32(positions.count))
                var t = 0
                while t + 2 < indices.count {
                    let centre = (positions[Int(indices[t])] + positions[Int(indices[t + 1])] + positions[Int(indices[t + 2])]) / 3
                    let p = matrix * SIMD4(centre, 1)
                    body(SIMD3(p.x, p.y, p.z))
                    t += 3
                }
            }
        }
    }

    /// Splits one mesh by triangle, by where each triangle's centre is in the
    /// die's frame. True if the whole entity went somewhere else (its
    /// children went with it).
    private func cut(_ entity: Entity, _ model: ModelComponent, by place: (SIMD3<Float>) -> Destination) -> Bool {
        let toReference = entity.transformMatrix(relativeTo: reference)
        let contents = model.mesh.contents
        var pieces: [Destination: (models: [MeshResource.Model], instances: [MeshResource.Instance], count: Int)] = [:]

        for (i, instance) in contents.instances.enumerated() {
            guard let source = contents.models[instance.model] else { continue }
            let matrix = toReference * instance.transform
            var parts: [Destination: [MeshResource.Part]] = [:]
            for part in source.parts {
                let positions = part.positions.elements
                let indices = part.triangleIndices?.elements ?? Array(0..<UInt32(positions.count))
                var sorted: [Destination: [UInt32]] = [:]
                var t = 0
                while t + 2 < indices.count {
                    let a = indices[t], b = indices[t + 1], c = indices[t + 2]
                    let centre = (positions[Int(a)] + positions[Int(b)] + positions[Int(c)]) / 3
                    let p = matrix * SIMD4(centre, 1)
                    sorted[place(SIMD3(p.x, p.y, p.z)), default: []] += [a, b, c]
                    t += 3
                }
                for (destination, kept) in sorted {
                    var p = part
                    p.triangleIndices = MeshBuffer(kept)
                    parts[destination, default: []].append(p)
                    pieces[destination, default: ([], [], 0)].count += kept.count
                }
            }
            let id = "\(instance.model)#\(i)"
            for (destination, kept) in parts {
                pieces[destination, default: ([], [], 0)].models.append(MeshResource.Model(id: id, parts: kept))
                pieces[destination, default: ([], [], 0)].instances.append(MeshResource.Instance(id: id, model: id, at: instance.transform))
            }
        }

        let elsewhere = pieces.keys.filter { $0 != .cup }
        if elsewhere.isEmpty { return false }
        if pieces.count == 1, let only = elsewhere.first, let destination = parent(for: only) {
            move(entity, to: destination)
            return true
        }
        var meshes: [Destination: MeshResource] = [:]
        for (destination, piece) in pieces {
            guard let mesh = Self.mesh(piece.models, piece.instances) else { return false }
            meshes[destination] = mesh
        }

        split.append((entity, model))
        // Every piece shows its inside now the die is open.
        let materials = model.materials.map(Self.doubleSided)
        if let cupMesh = meshes[.cup] {
            var cupModel = model
            cupModel.mesh = cupMesh
            cupModel.materials = materials
            entity.components.set(cupModel)
        } else {
            entity.components.remove(ModelComponent.self)
        }
        let opacity = inheritedOpacity(from: entity)
        for (destination, mesh) in meshes {
            guard let parent = parent(for: destination) else { continue }
            let fragment = Entity()
            fragment.name = "\(entity.name)_\(destination)"
            var piece = model
            piece.mesh = mesh
            piece.materials = materials
            fragment.components.set(piece)
            parent.addChild(fragment)
            fragment.setTransformMatrix(entity.transformMatrix(relativeTo: parent), relativeTo: parent)
            if opacity < 1 { fragment.components.set(OpacityComponent(opacity: opacity)) }
        }
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
    nonisolated static func roundedOutline(half: Float, radius: Float) -> [SIMD2<Float>] {
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
