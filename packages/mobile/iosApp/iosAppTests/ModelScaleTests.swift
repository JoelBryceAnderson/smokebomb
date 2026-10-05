import RealityKit
import XCTest

/// Real-world scale is the point of the viewer: every bundled model's visual
/// bounds must match the size table (ModelCatalog.all) to ±0.1 mm.
/// A model that isn't bundled yet is skipped, not failed.
@MainActor
final class ModelScaleTests: XCTestCase {
    private static let toleranceMetres: Float = 0.0001

    private var catalog: ModelCatalog!

    override func setUp() async throws {
        catalog = ModelCatalog(source: BundleModelSource(bundle: Bundle(for: Self.self)))
    }

    func testSugarcube30() async throws { try await check("sugarcube_30") }
    func testSugarcube30Rainbow() async throws { try await check("sugarcube_30_rainbow") }
    func testSugarcube34() async throws { try await check("sugarcube_34") }
    func testSugarcube34OneBit() async throws { try await check("sugarcube_34_1bit") }
    func testSugarcube40() async throws { try await check("sugarcube_40") }
    func testSugarcube30Xray() async throws { try await check("sugarcube_30_xray") }
    func testSugarcube34Xray() async throws { try await check("sugarcube_34_xray") }
    func testSugarcube40Xray() async throws { try await check("sugarcube_40_xray") }
    func testSugarcube30XrayExploded() async throws { try await check("sugarcube_30_xray_exploded") }
    func testLineup() async throws { try await check("sugarcube_lineup") }
    func testContractFixture() async throws { try await check("sugarcube_contract_fixture") }

    func testEveryCatalogModelHasATest() {
        let tested: Set = [
            "sugarcube_30", "sugarcube_30_rainbow", "sugarcube_34", "sugarcube_34_1bit", "sugarcube_40",
            "sugarcube_30_xray", "sugarcube_34_xray", "sugarcube_40_xray", "sugarcube_30_xray_exploded",
            "sugarcube_lineup", "sugarcube_contract_fixture",
        ]
        XCTAssertEqual(Set(ModelCatalog.all.map(\.id)), tested, "add a test above for each new model")
    }

    private func check(_ id: String) async throws {
        let model = try XCTUnwrap(catalog.model(id: id))
        guard catalog.source.isAvailable(model) else {
            throw XCTSkip("\(id).usdz isn't bundled yet")
        }
        let entity = try await catalog.load(model)
        let bounds = entity.visualBounds(relativeTo: nil)
        let size = bounds.extents
        for axis in 0..<3 {
            XCTAssertEqual(size[axis], model.bounds[axis], accuracy: Self.toleranceMetres,
                           "\(id): \("xyz".map(String.init)[axis]) is \(size[axis] * 1000) mm, expected \(model.bounds[axis] * 1000) mm")
        }
        // Origin at the bottom centre, so the model sits on the table where it's placed.
        XCTAssertEqual(bounds.min.y, 0, accuracy: Self.toleranceMetres, "\(id): bottom isn't at the origin")
        XCTAssertEqual(bounds.center.x, 0, accuracy: Self.toleranceMetres, "\(id): not centred on x")
        XCTAssertEqual(bounds.center.z, 0, accuracy: Self.toleranceMetres, "\(id): not centred on z")
    }
}
