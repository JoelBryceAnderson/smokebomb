import CoreGraphics
import Metal
import RealityKit
import UIKit

/// The firmware's panels drawn on the die: one quad over each `Screen_<face>`,
/// turned to that face's screen axes (`DieFace.screenAxes`, the firmware's
/// `orientation::BASES`), so frames land the way the hardware shows them,
/// whatever the model's own UVs. The model's baked screens are hidden while
/// these are up.
@MainActor
final class LiveScreens {
    /// Flip if the frames show upside down on device (RealityKit texture
    /// coordinates start bottom-left, as USD's do; this assumes so).
    static let flipVertically = false
    /// How far in front of the model's screen the quad sits, in metres.
    static let lift: Float = 0.00005

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

            let quad = ModelEntity(mesh: Self.quad(width: width, height: height), materials: [Self.material(texture)])
            quad.name = "LiveScreen_\(face.rawValue)"
            parent.addChild(quad)
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

    /// A quad in the XY plane facing +Z, with the image's top row along +Y.
    private static func quad(width: Float, height: Float) -> MeshResource {
        let w = width / 2, h = height / 2
        let (bottom, top): (Float, Float) = flipVertically ? (1, 0) : (0, 1)
        var mesh = MeshDescriptor(name: "LiveScreen")
        mesh.positions = MeshBuffer([[-w, -h, 0], [w, -h, 0], [w, h, 0], [-w, h, 0]])
        mesh.normals = MeshBuffer(Array(repeating: SIMD3<Float>(0, 0, 1), count: 4))
        mesh.textureCoordinates = MeshBuffer([[0, bottom], [1, bottom], [1, top], [0, top]])
        mesh.primitives = .triangles([0, 1, 2, 0, 2, 3])
        // A fallback that can't fail: a plain plane, if the custom mesh is refused.
        return (try? MeshResource.generate(from: [mesh])) ?? .generatePlane(width: width, height: height)
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
