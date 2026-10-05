import CoreGraphics
import Metal
import RealityKit
import UIKit

/// The firmware's panels drawn on the die: one quad over each `Screen_<face>`,
/// turned to that face's screen axes (`DieFace.screenAxes`, the firmware's
/// `orientation::BASES`), so frames land the way the hardware shows them,
/// whatever the model's own UVs. The model's baked screens are hidden while
/// these are up. The quads are rounded like the glass's mask; their corners
/// are filled black, so the square the baked screen covered stays covered.
@MainActor
final class LiveScreens {
    /// Flip if the frames show upside down on device (RealityKit texture
    /// coordinates start bottom-left, as USD's do; this assumes so).
    static let flipVertically = false
    /// How far in front of the model's screen the quad sits, in metres.
    static let lift: Float = 0.00005
    /// Segments in each rounded corner of the quad.
    static let cornerSegments = 8

    /// The glass's ink mask rounds the lit area, as the web simulator draws it
    /// (`geometry.ts` `maskTexels` / `texelsPerPx`): 14 px on the 96×96 panel,
    /// 10.75 px on the 64×64. A fraction of the side, so it fits any quad.
    static func cornerFraction(side: Int) -> Float {
        switch side {
        case 96: return 14 / 96
        case 64: return 10.75 / 64
        default: return 0.15
        }
    }

    private struct Screen {
        let quad: ModelEntity
        let texture: TextureResource
        var shown: [UInt8] = []
    }

    let side: Int
    private var screens: [DieFace: Screen] = [:]
    private var hidden: [Entity] = []

    /// Builds quads for every `Screen_<face>` in the rig. `reference` is the
    /// rig's reference entity (the die's own metres).
    init(rig: DieRig, reference: Entity, side: Int) {
        self.side = side
        let blank = [UInt8](repeating: 0, count: side * side * 4)
        guard let blankImage = Self.image(blank, side: side) else { return }
        for face in DieFace.allCases {
            guard let screen = rig.root.findEntity(named: "Screen_\(face.rawValue)"),
                  let parent = screen.parent,
                  let texture = try? TextureResource.generate(from: blankImage, options: Self.textureOptions)
            else { continue }

            let bounds = screen.visualBounds(relativeTo: reference)
            let (right, up) = face.screenAxes
            let normal = face.normal
            let width = abs(simd_dot(bounds.extents, right))
            let height = abs(simd_dot(bounds.extents, up))
            let depth = abs(simd_dot(bounds.extents, normal))
            guard width > 0, height > 0 else { continue }

            let radius = min(width, height) * Self.cornerFraction(side: side)
            let quad = ModelEntity(mesh: Self.quad(width: width, height: height, radius: radius), materials: [Self.material(texture)])
            quad.name = "LiveScreen_\(face.rawValue)"
            parent.addChild(quad)
            if let mesh = Self.corners(width: width, height: height, radius: radius) {
                quad.addChild(ModelEntity(mesh: mesh, materials: [UnlitMaterial(color: .black)]))
            }
            // Face the screen's way, on its outer surface, in the die's frame.
            let rotation = simd_quatf(simd_float3x3(columns: (right, up, normal)))
            let centre = bounds.center + normal * (depth / 2 + Self.lift)
            quad.setTransformMatrix(Transform(rotation: rotation, translation: centre).matrix, relativeTo: reference)

            screen.isEnabled = false
            hidden.append(screen)
            screens[face] = Screen(quad: quad, texture: texture)
        }
    }

    var isEmpty: Bool { screens.isEmpty }

    /// Shows a new frame on one face; skipped if it hasn't changed.
    func show(_ rgba: [UInt8], on face: DieFace) {
        guard var screen = screens[face], rgba != screen.shown, let image = Self.image(rgba, side: side) else { return }
        try? screen.texture.replace(withImage: image, options: Self.textureOptions)
        screen.shown = rgba
        screens[face] = screen
    }

    /// Takes the quads off and brings the model's baked screens back.
    func remove() {
        for screen in screens.values { screen.quad.removeFromParent() }
        for entity in hidden { entity.isEnabled = true }
        screens.removeAll()
        hidden.removeAll()
    }

    // MARK: Building

    private static var textureOptions: TextureResource.CreateOptions {
        var options = TextureResource.CreateOptions(semantic: .color)
        options.mipmapsMode = .none
        return options
    }

    /// Unlit, like an OLED: it glows the same in any light. Nearest-neighbour
    /// sampling keeps the panel's pixels crisp.
    private static func material(_ texture: TextureResource) -> UnlitMaterial {
        let sampler = MTLSamplerDescriptor()
        sampler.minFilter = .nearest
        sampler.magFilter = .nearest
        sampler.mipFilter = .notMipmapped
        var material = UnlitMaterial()
        material.color = .init(tint: .white, texture: .init(texture, sampler: .init(sampler)))
        return material
    }

    /// A rounded quad in the XY plane facing +Z, with the image's top row
    /// along +Y: a fan from the centre round the outline, each corner an arc
    /// of `radius`, UVs mapped straight from position so the panel isn't
    /// squashed, just cut at the corners like the glass's mask.
    private static func quad(width: Float, height: Float, radius: Float) -> MeshResource {
        let w = width / 2, h = height / 2
        let r = max(0, min(radius, w, h))
        let outline = roundedOutline(width: width, height: height, radius: r)
        let points = [SIMD2<Float>(0, 0)] + outline
        func uv(_ p: SIMD2<Float>) -> SIMD2<Float> {
            let v = (p.y + h) / height
            return [(p.x + w) / width, flipVertically ? 1 - v : v]
        }
        var indices: [UInt32] = []
        for i in 0..<outline.count {
            indices += [0, UInt32(1 + i), UInt32(1 + (i + 1) % outline.count)]
        }
        var mesh = MeshDescriptor(name: "LiveScreen")
        mesh.positions = MeshBuffer(points.map { SIMD3($0.x, $0.y, 0) })
        mesh.normals = MeshBuffer(Array(repeating: SIMD3<Float>(0, 0, 1), count: points.count))
        mesh.textureCoordinates = MeshBuffer(points.map(uv))
        mesh.primitives = .triangles(indices)
        // A fallback that can't fail: a plain plane, if the custom mesh is refused.
        return (try? MeshResource.generate(from: [mesh]))
            ?? .generatePlane(width: width, height: height, cornerRadius: r)
    }

    /// The four corners the rounding cuts off the quad, in the quad's plane:
    /// the square outline stitched to the rounded one, point for point.
    private static func corners(width: Float, height: Float, radius: Float) -> MeshResource? {
        let outer = roundedOutline(width: width, height: height, radius: 0)
        let inner = roundedOutline(width: width, height: height, radius: radius)
        let n = outer.count
        var indices: [UInt32] = []
        for k in 0..<n {
            let next = (k + 1) % n
            let (o0, o1, i0, i1) = (UInt32(k), UInt32(next), UInt32(n + k), UInt32(n + next))
            indices += [o0, o1, i1, o0, i1, i0]
        }
        var mesh = MeshDescriptor(name: "LiveScreenCorners")
        mesh.positions = MeshBuffer((outer + inner).map { SIMD3($0.x, $0.y, 0) })
        mesh.normals = MeshBuffer(Array(repeating: SIMD3<Float>(0, 0, 1), count: 2 * n))
        mesh.primitives = .triangles(indices)
        return try? MeshResource.generate(from: [mesh])
    }

    /// A rounded rectangle's outline about the origin, counter-clockwise from
    /// the bottom-right corner's arc, `cornerSegments + 1` points a corner, so
    /// any two outlines line up point for point.
    private static func roundedOutline(width: Float, height: Float, radius: Float) -> [SIMD2<Float>] {
        let w = width / 2, h = height / 2
        let r = max(0, min(radius, w, h))
        // Corner centres with each arc's start angle.
        let corners: [(SIMD2<Float>, Float)] = [
            ([w - r, -h + r], -.pi / 2),
            ([w - r, h - r], 0),
            ([-w + r, h - r], .pi / 2),
            ([-w + r, -h + r], .pi),
        ]
        var outline: [SIMD2<Float>] = []
        for (centre, start) in corners {
            for k in 0...cornerSegments {
                let a = start + Float(k) / Float(cornerSegments) * (.pi / 2)
                outline.append(centre + r * SIMD2(cos(a), sin(a)))
            }
        }
        return outline
    }

    private static func image(_ rgba: [UInt8], side: Int) -> CGImage? {
        guard rgba.count >= side * side * 4,
              let provider = CGDataProvider(data: Data(rgba) as CFData)
        else { return nil }
        return CGImage(
            width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: side * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}
