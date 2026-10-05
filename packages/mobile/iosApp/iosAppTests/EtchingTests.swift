import RealityKit
import XCTest

/// The charging face's laser etching: which models carry it, where it sits, and what it draws.
@MainActor
final class EtchingTests: XCTestCase {
    private func model(_ id: String) throws -> SugarcubeModel {
        try XCTUnwrap(ModelCatalog.all.first { $0.id == id })
    }

    func testOnlyWholeDiceAreEtched() throws {
        XCTAssertEqual(Etching.layout(for: try model("sugarcube_34")), Etching.layout34)
        XCTAssertEqual(Etching.layout(for: try model("sugarcube_30_rainbow")), Etching.layout30)
        XCTAssertEqual(Etching.layout(for: try model("sugarcube_40"))?.flatMm ?? 0, 25 * 4 / 3, accuracy: 1e-4)
        XCTAssertEqual(Etching.layout(for: try model("sugarcube_contract_fixture")), Etching.layout30)
        for id in ["sugarcube_30_xray", "sugarcube_30_xray_exploded", "sugarcube_lineup"] {
            XCTAssertNil(Etching.layout(for: try model(id)), id)
        }
    }

    func testTheBandStaysOffTheWindowAndOnTheFlat() {
        for layout in [Etching.layout30, Etching.layout34] {
            XCTAssertGreaterThan(layout.bandMm, layout.windowHalfMm)
            XCTAssertLessThan(layout.bandMm, layout.flatMm / 2)
        }
    }

    func testSitsJustBelowTheLidAndFadesWithTheShell() async throws {
        let bundle = Bundle(for: Self.self)
        let catalog = ModelCatalog(source: BundleModelSource(bundle: bundle))
        let fixture = try model("sugarcube_contract_fixture")
        let entity = try await catalog.load(fixture)
        let pivot = Entity()
        pivot.addChild(entity)
        let rig = DieRig(model: fixture, root: entity, reference: pivot, labels: .load(bundle: bundle))

        let etching = try XCTUnwrap(rig.etching)
        let bounds = etching.visualBounds(relativeTo: pivot)
        XCTAssertEqual(bounds.extents.x, 0.025, accuracy: 1e-6)
        XCTAssertEqual(bounds.extents.z, 0.025, accuracy: 1e-6)
        XCTAssertEqual(bounds.max.y, -Etching.lift, accuracy: 1e-7)
        XCTAssertEqual(bounds.min.y, -Etching.lift, accuracy: 1e-7)
        XCTAssertEqual(rig.measuredSize().y, 0.030, accuracy: 0.0001, "the decal doesn't count toward true size")
        // Its face points out of the lid.
        XCTAssertEqual(simd_distance(etching.orientation(relativeTo: pivot).act([0, 0, 1]), [0, -1, 0]), 0, accuracy: 1e-5)

        rig.setShellFaded(true)
        XCTAssertFalse(etching.isEnabled)
        rig.setShellFaded(false)
        XCTAssertTrue(etching.isEnabled)
    }

    func testDrawsInTheBorderOnly() throws {
        for layout in [Etching.layout30, Etching.layout34] {
            let image = try XCTUnwrap(Etching.image(layout))
            let side = Etching.textureSide
            XCTAssertEqual(image.width, side)
            let alpha = try Self.alpha(image)
            let px = Float(side) / layout.flatMm
            let window = Int(layout.windowHalfMm * px)
            var inWindow = 0, inBand = 0
            for y in 0..<side {
                for x in 0..<side where alpha[y * side + x] > 0 {
                    if abs(x - side / 2) < window && abs(y - side / 2) < window { inWindow += 1 } else { inBand += 1 }
                }
            }
            XCTAssertEqual(inWindow, 0, "nothing over the window")
            XCTAssertGreaterThan(inBand, 1000, "the border carries the etching")
            // Every side is marked: the wordmark (top), the serial (right),
            // the tagline (bottom) and the regulatory line (left).
            let band = Int(layout.bandMm * px), mid = side / 2, reach = side / 8
            func inked(_ cx: Int, _ cy: Int) -> Bool {
                let r = Int(1.5 * px)
                for y in (cy - r)...(cy + r) {
                    for x in (cx - reach)...(cx + reach) where x >= 0 && y >= 0 && x < side && y < side && alpha[y * side + x] > 0 {
                        return true
                    }
                }
                return false
            }
            XCTAssertTrue(inked(mid, mid - band), "top")
            XCTAssertTrue(inked(mid, mid + band), "bottom")
            // The side lines run vertically: sample along their length.
            XCTAssertTrue((mid - reach...mid + reach).contains { y in alpha[y * side + mid + band] > 0 || alpha[y * side + mid + band - 1] > 0 || alpha[y * side + mid + band + 1] > 0 }, "right")
            XCTAssertTrue((mid - reach...mid + reach).contains { y in alpha[y * side + mid - band] > 0 || alpha[y * side + mid - band - 1] > 0 || alpha[y * side + mid - band + 1] > 0 }, "left")
        }
    }

    /// The image's alpha, top row first.
    private static func alpha(_ image: CGImage) throws -> [UInt8] {
        let w = image.width, h = image.height
        let context = try XCTUnwrap(CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                                              space: CGColorSpaceCreateDeviceRGB(),
                                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        let data = try XCTUnwrap(context.data).bindMemory(to: UInt8.self, capacity: context.bytesPerRow * h)
        return (0..<(w * h)).map { i in data[(i / w) * context.bytesPerRow + (i % w) * 4 + 3] }
    }
}
