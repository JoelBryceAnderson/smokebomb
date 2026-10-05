import CoreText
import RealityKit
import UIKit

/// The laser etching round the charging face's window, on the lid (`ny`), as
/// the desktop simulator draws it (`packages/simulator/web-ui/src/shell.ts`):
/// the wordmark and the tagline in the script, the model and serial, and the
/// regulatory line with CE and the crossed-out bin. Tone-on-tone marking: a
/// 55 % grey decal over the border band, so it never covers the window.
@MainActor
enum Etching {
    /// Where the etching goes on one size of die, in millimetres.
    struct Layout: Equatable {
        /// The flat face: the die less two edge radii. The texture covers it.
        var flatMm: Float
        /// The window's half-size, where the border band starts.
        var windowHalfMm: Float
        /// The band's centre line, from the face centre.
        var bandMm: Float
        /// Script lines centred in the band (30 mm) or hugging the window (34 mm).
        var centreScripts: Bool

        func scaled(_ k: Float) -> Layout {
            Layout(flatMm: flatMm * k, windowHalfMm: windowHalfMm * k, bandMm: bandMm * k, centreScripts: centreScripts)
        }
    }

    /// The simulator's layouts (`GEOMETRY` in `web-ui/src/geometry.ts`).
    static let layout34 = Layout(flatMm: 29, windowHalfMm: 12, bandMm: 13.25, centreScripts: false)
    static let layout30 = Layout(flatMm: 25, windowHalfMm: 8.75, bandMm: (8.75 + 12.5) / 2, centreScripts: true)

    /// The serial the simulator etches before a roll has carried the die's own.
    static let serial = "000042"

    static let wordmark = "Sugarcube"
    static let tagline = "Designed in Williamsburg, BK · Shake well before serving"
    static let regulatory = "REGULATORY INFO IN SETTINGS"

    /// How far below the lid the decal sits, in metres.
    static let lift: Float = 0.00005
    static let opacity: Float = 0.55
    static let textureSide = 1024

    /// The layout for a model, or nil if it shows no etching: only whole dice
    /// do (the x-rays, the line-up and the exploded view don't). The 40 mm die
    /// has no layout in the simulator yet, so it takes the 30 mm one scaled up.
    static func layout(for model: SugarcubeModel) -> Layout? {
        guard model.kind == .die || model.kind == .fixture else { return nil }
        switch Int((model.bounds.x * 1000).rounded()) {
        case 34: return layout34
        case 30: return layout30
        case 40: return layout30.scaled(40.0 / 30.0)
        default: return nil
        }
    }

    /// Adds the etching under `root`, flat on the lid. `reference` is the die's
    /// own metres with the origin at the bottom centre. Returns nil if the
    /// model shows none.
    static func attach(to root: Entity, reference: Entity, model: SugarcubeModel) -> Entity? {
        guard let layout = Self.layout(for: model),
              let image = Self.image(layout),
              let texture = try? TextureResource.generate(from: image, options: textureOptions)
        else { return nil }

        var material = PhysicallyBasedMaterial()
        material.baseColor = .init(tint: UIColor(red: 150 / 255, green: 152 / 255, blue: 156 / 255, alpha: 1))
        material.roughness = 0.7
        material.metallic = 0.3
        material.blending = .transparent(opacity: .init(scale: opacity, texture: .init(texture)))

        let entity = ModelEntity(mesh: band(layout), materials: [material])
        entity.name = "Etching"
        root.addChild(entity)
        // Face out of the lid, along the firmware's screen axes for it, as the
        // simulator's decal does.
        let face = DieFace.ny
        let (right, up) = face.screenAxes
        let rotation = simd_quatf(simd_float3x3(columns: (right, up, face.normal)))
        entity.setTransformMatrix(Transform(rotation: rotation, translation: face.normal * lift).matrix, relativeTo: reference)
        return entity
    }

    // MARK: Building

    private static var textureOptions: TextureResource.CreateOptions {
        var options = TextureResource.CreateOptions(semantic: .raw)
        options.mipmapsMode = .allocateAndGenerateAll
        return options
    }

    /// The border band as a square frame in the XY plane facing +Z, its UVs
    /// spanning the flat face so the texture lands as drawn.
    static func band(_ layout: Layout) -> MeshResource {
        let s = layout.flatMm / 2 / 1000, w = layout.windowHalfMm / 1000
        let corners: [SIMD2<Float>] = [[-1, -1], [1, -1], [1, 1], [-1, 1]]
        let points = corners.map { $0 * s } + corners.map { $0 * w }
        var mesh = MeshDescriptor(name: "Etching")
        mesh.positions = MeshBuffer(points.map { SIMD3($0.x, $0.y, 0) })
        mesh.normals = MeshBuffer(Array(repeating: SIMD3<Float>(0, 0, 1), count: points.count))
        mesh.textureCoordinates = MeshBuffer(points.map { p -> SIMD2<Float> in
            let v = p.y / (2 * s) + 0.5
            return SIMD2(p.x / (2 * s) + 0.5, LiveScreens.flipVertically ? 1 - v : v)
        })
        // Outer 0–3, inner 4–7: two triangles per side, counter-clockwise.
        mesh.primitives = .triangles((0..<4).flatMap { k -> [UInt32] in
            let o = UInt32(k), o1 = UInt32((k + 1) % 4)
            return [o, o1, o1 + 4, o, o1 + 4, o + 4]
        })
        return (try? MeshResource.generate(from: [mesh]))
            ?? .generatePlane(width: layout.flatMm / 1000, height: layout.flatMm / 1000)
    }

    /// The etching's ink as white on clear (so every channel is its coverage,
    /// whichever one the opacity reads), top row toward the face's screen up,
    /// the way `drawEtching` in the simulator lays it out.
    static func image(_ layout: Layout) -> CGImage? {
        let C = CGFloat(textureSide)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = false
        let rendered = UIGraphicsImageRenderer(size: CGSize(width: C, height: C), format: format).image { context in
            let cg = context.cgContext
            cg.setFillColor(UIColor.white.cgColor)
            cg.setStrokeColor(UIColor.white.cgColor)
            cg.textMatrix = .identity
            draw(layout, side: C, in: cg)
        }
        return rendered.cgImage
    }

    /// Script sizes, and the script ink's clearance from the window, in mm.
    private static let wordmarkMm: CGFloat = 0.95
    private static let taglineMm: CGFloat = 0.7
    private static let scriptClearMm: CGFloat = 0.8

    private static func draw(_ layout: Layout, side C: CGFloat, in cg: CGContext) {
        let px = C / CGFloat(layout.flatMm)
        // The band's centre line, above the face centre once rotated to a side.
        let bandLine = -CGFloat(layout.bandMm) * px
        let windowHalf = CGFloat(layout.windowHalfMm)

        func onSide(_ rotation: CGFloat, _ body: () -> Void) {
            cg.saveGState()
            cg.translateBy(x: C / 2, y: C / 2)
            cg.rotate(by: rotation)
            body()
            cg.restoreGState()
        }
        // A plain line, its em box centred on the band.
        func plain(_ text: String, mm: CGFloat, y: CGFloat) {
            let font = Fonts.grotesk(size: mm * px)
            draw(line(text, font), baseline: y + (CTFontGetAscent(font) - CTFontGetDescent(font)) / 2, in: cg)
        }
        // Script lines. Their tails hang well below the caps' box, so place
        // them by their ink: centred in the band on the 30 mm die, the lowest
        // tail `scriptClearMm` off the window on the 34 mm one.
        func script(_ text: String, mm: CGFloat) {
            let ctLine = line(text, Fonts.script(size: mm * px))
            let ink = CTLineGetImageBounds(ctLine, cg) // y up from the baseline
            let baseline = layout.centreScripts
                ? bandLine + (ink.minY + ink.maxY) / 2
                : -(windowHalf + scriptClearMm) * px + ink.minY
            draw(ctLine, baseline: baseline, in: cg)
        }

        onSide(0) { script(wordmark, mm: wordmarkMm) }
        onSide(.pi / 2) { plain("SC-1  ·  S/N \(serial)", mm: 0.7, y: bandLine) }
        onSide(.pi) { script(tagline, mm: taglineMm) }
        // Left: CE, a crossed-out wheelie bin, and where the rest lives.
        onSide(-.pi / 2) {
            cg.translateBy(x: 0, y: bandLine)
            plain("CE      " + regulatory, mm: 0.62, y: 0)
            let bx = -5.2 * px, s = 0.36 * px
            cg.setLineWidth(0.07 * px)
            cg.stroke(CGRect(x: bx - s * 0.6, y: -s * 0.8, width: s * 1.2, height: s * 1.6))
            cg.move(to: CGPoint(x: bx - s, y: -s))
            cg.addLine(to: CGPoint(x: bx + s, y: s))
            cg.move(to: CGPoint(x: bx + s, y: -s))
            cg.addLine(to: CGPoint(x: bx - s, y: s))
            cg.strokePath()
        }
    }

    private static func line(_ text: String, _ font: CTFont) -> CTLine {
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font,
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ]
        return CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
    }

    /// Draws a line centred on x = 0 with its baseline at `baseline`, in the
    /// image's y-down coordinates.
    private static func draw(_ line: CTLine, baseline: CGFloat, in cg: CGContext) {
        let width = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
        cg.saveGState()
        cg.translateBy(x: -width / 2, y: baseline)
        cg.scaleBy(x: 1, y: -1)
        cg.textPosition = .zero
        CTLineDraw(line, cg)
        cg.restoreGState()
    }

    /// The simulator's two faces, bundled from `packages/firmware/assets/fonts`
    /// as TrueType (`Fonts/`, OFL). Space Grotesk is the bold the firmware
    /// ships; the simulator asks for semibold.
    private enum Fonts {
        private final class Token {}

        private static let scriptFace = descriptor("Pacifico-Regular")
        private static let groteskFace = descriptor("SpaceGrotesk-Bold")

        static func script(size: CGFloat) -> CTFont {
            if let scriptFace { return CTFontCreateWithFontDescriptor(scriptFace, size, nil) }
            return (UIFont(name: "SnellRoundhand-Bold", size: size) ?? .italicSystemFont(ofSize: size)) as CTFont
        }

        static func grotesk(size: CGFloat) -> CTFont {
            if let groteskFace { return CTFontCreateWithFontDescriptor(groteskFace, size, nil) }
            return UIFont.systemFont(ofSize: size, weight: .semibold) as CTFont
        }

        private static func descriptor(_ name: String) -> CTFontDescriptor? {
            guard let url = Bundle(for: Token.self).url(forResource: name, withExtension: "ttf"),
                  let data = try? Data(contentsOf: url)
            else { return nil }
            return CTFontManagerCreateFontDescriptorFromData(data as CFData)
        }
    }
}
