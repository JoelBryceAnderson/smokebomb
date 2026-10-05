import RealityKit
import UIKit

/// The turn pad as a pane of glass floating beside the held die: the menu's
/// tips (SIM_SPEC C3) as keys in space. It lies in the plane of the die's
/// menu screen, as if that screen carried on past the die's edge, and stays
/// put in the room after that, like a real object.
@MainActor
final class MenuPanel {
    /// Keys answer ray casts in this group only, so they never touch the physics.
    static let keyGroup = CollisionGroup(rawValue: 1 << 20)

    private struct Key {
        let axis: TurnAxis
        let direction: Int
        let symbol: String
        /// Position on the pane, in key spacings.
        let at: SIMD2<Float>
    }

    // up: −90° about your right; left: −90° about vertical (the firmware's table).
    private static let keys: [Key] = [
        Key(axis: .pitch, direction: -1, symbol: "chevron.up", at: [0, 1.5]),
        Key(axis: .yaw, direction: -1, symbol: "chevron.left", at: [-0.6, 0.5]),
        Key(axis: .yaw, direction: 1, symbol: "chevron.right", at: [0.6, 0.5]),
        Key(axis: .pitch, direction: 1, symbol: "chevron.down", at: [0, -0.5]),
        Key(axis: .roll, direction: 1, symbol: "arrow.counterclockwise", at: [-0.6, -1.7]),
        Key(axis: .roll, direction: -1, symbol: "arrow.clockwise", at: [0.6, -1.7]),
    ]

    let root = Entity()
    /// Pane width and height, in metres.
    let width: Float
    let height: Float
    private var keyEntities: [ObjectIdentifier: (entity: Entity, key: Key)] = [:]

    /// A pane sized to a die of side `side` metres.
    init(side: Float) {
        let keySize = side * 0.55
        let spacing = keySize * 1.25
        width = spacing * 2.6
        height = spacing * 4.6
        root.name = "MenuPanel"

        var glass = PhysicallyBasedMaterial()
        glass.baseColor = .init(tint: UIColor(white: 0.92, alpha: 1))
        glass.roughness = .init(floatLiteral: 0.15)
        glass.metallic = .init(floatLiteral: 0)
        glass.blending = .transparent(opacity: .init(floatLiteral: 0.22))
        let pane = ModelEntity(
            mesh: .generatePlane(width: width, height: height, cornerRadius: keySize * 0.35),
            materials: [glass])
        pane.position = [0, -spacing * 0.1, 0]
        root.addChild(pane)

        let shape = ShapeResource.generateBox(width: keySize, height: keySize, depth: 0.002)
        for key in Self.keys {
            guard let image = Self.icon(key.symbol),
                  let texture = try? TextureResource.generate(from: image, options: .init(semantic: .color))
            else { continue }
            var face = UnlitMaterial()
            face.color = .init(tint: .white, texture: .init(texture))
            face.blending = .transparent(opacity: .init(floatLiteral: 0.9))
            let tile = ModelEntity(
                mesh: .generatePlane(width: keySize, height: keySize, cornerRadius: keySize * 0.3),
                materials: [face])
            tile.name = "TurnKey"
            tile.position = [key.at.x * spacing, key.at.y * spacing, 0.001]
            tile.components.set(CollisionComponent(
                shapes: [shape], mode: .trigger,
                filter: CollisionFilter(group: Self.keyGroup, mask: Self.keyGroup)))
            root.addChild(tile)
            keyEntities[ObjectIdentifier(tile)] = (tile, key)
        }
    }

    /// Stands the pane at `position` (in `parent`'s space), its face along
    /// `rotation`'s +Z and its up along +Y; it grows in from a little smaller.
    func show(at position: SIMD3<Float>, rotation: simd_quatf, in parent: Entity) {
        parent.addChild(root)
        root.transform = Transform(scale: SIMD3(repeating: 0.7), rotation: rotation, translation: position)
        root.move(
            to: Transform(scale: .one, rotation: rotation, translation: position),
            relativeTo: parent, duration: 0.25, timingFunction: .easeOut)
    }

    func hide() {
        root.removeFromParent()
    }

    /// The key under a screen point, if any; it gives a little press.
    func press(at point: CGPoint, in view: ARView) -> (axis: TurnAxis, direction: Int)? {
        guard root.parent != nil else { return nil }
        for hit in view.hitTest(point, query: .nearest, mask: Self.keyGroup) {
            guard let found = keyEntities[ObjectIdentifier(hit.entity)] else { continue }
            let tile = found.entity
            let rest = Transform(scale: .one, rotation: tile.orientation, translation: tile.position)
            tile.scale = SIMD3(repeating: 0.85)
            tile.move(to: rest, relativeTo: tile.parent, duration: 0.15, timingFunction: .easeOut)
            return (found.key.axis, found.key.direction)
        }
        return nil
    }

    /// Whether a screen point is on a key (so the touch isn't also the die's).
    func contains(_ point: CGPoint, in view: ARView) -> Bool {
        guard root.parent != nil else { return false }
        return !view.hitTest(point, query: .nearest, mask: Self.keyGroup).isEmpty
    }

    /// A key face: a white symbol on dark glass.
    private static func icon(_ symbol: String) -> CGImage? {
        let size = CGSize(width: 128, height: 128)
        let config = UIImage.SymbolConfiguration(pointSize: 56, weight: .semibold)
        guard let glyph = UIImage(systemName: symbol, withConfiguration: config)?
            .withTintColor(.white, renderingMode: .alwaysOriginal) else { return nil }
        let image = UIGraphicsImageRenderer(size: size).image { _ in
            UIColor(white: 0.12, alpha: 1).setFill()
            UIRectFill(CGRect(origin: .zero, size: size))
            let g = glyph.size
            glyph.draw(in: CGRect(x: (size.width - g.width) / 2, y: (size.height - g.height) / 2, width: g.width, height: g.height))
        }
        return image.cgImage
    }
}
