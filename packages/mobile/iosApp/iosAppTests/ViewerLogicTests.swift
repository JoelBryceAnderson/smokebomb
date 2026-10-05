import simd
import XCTest

final class ViewerLogicTests: XCTestCase {
    // MARK: Face up

    func testFaceUpAtRest() {
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: 0, axis: [0, 1, 0])), .py)
    }

    func testFaceUpAfterQuarterTurns() {
        // +90° about Z turns +X to point up.
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: .pi / 2, axis: [0, 0, 1])), .px)
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: -.pi / 2, axis: [0, 0, 1])), .nx)
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: .pi, axis: [1, 0, 0])), .ny)
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: .pi / 2, axis: [1, 0, 0])), .nz)
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: -.pi / 2, axis: [1, 0, 0])), .pz)
    }

    func testFaceUpOnATilt() {
        // Leaning 30° towards +X still reads as the top face.
        XCTAssertEqual(DieFace.faceUp(orientation: simd_quatf(angle: -.pi / 6, axis: [0, 0, 1])), .py)
    }

    // MARK: Settling

    func testSettlesAfterStayingStill() {
        var detector = SettleDetector()
        XCTAssertFalse(detector.update(linearSpeed: 0.5, angularSpeed: 10, dt: 0.1))
        XCTAssertFalse(detector.update(linearSpeed: 0, angularSpeed: 0, dt: 0.2))
        XCTAssertTrue(detector.update(linearSpeed: 0, angularSpeed: 0, dt: 0.2))
    }

    func testMovementRestartsTheClock() {
        var detector = SettleDetector()
        _ = detector.update(linearSpeed: 0, angularSpeed: 0, dt: 0.3)
        _ = detector.update(linearSpeed: 0.1, angularSpeed: 0, dt: 0.01)
        XCTAssertFalse(detector.update(linearSpeed: 0, angularSpeed: 0, dt: 0.3))
    }

    func testGivesUpAfterMaxRollTime() {
        var detector = SettleDetector()
        XCTAssertTrue(detector.update(linearSpeed: 1, angularSpeed: 1, dt: DiePhysics.maxRollTime))
    }

    func testThrowSpeedStaysInRange() {
        XCTAssertEqual(DiePhysics.throwSpeed(forFlick: 0), DiePhysics.throwSpeed.lowerBound)
        XCTAssertEqual(DiePhysics.throwSpeed(forFlick: 1e6), DiePhysics.throwSpeed.upperBound)
    }

    // MARK: Picking

    func testRayHitsBox() {
        let t = PartPicker.hitDistance(origin: [0, 5, 0], direction: [0, -1, 0], boxMin: [-1, -1, -1], boxMax: [1, 1, 1])
        XCTAssertEqual(t, 4)
    }

    func testRayMissesBoxBehindOrBeside() {
        XCTAssertNil(PartPicker.hitDistance(origin: [0, 5, 0], direction: [0, 1, 0], boxMin: [-1, -1, -1], boxMax: [1, 1, 1]))
        XCTAssertNil(PartPicker.hitDistance(origin: [3, 5, 0], direction: [0, -1, 0], boxMin: [-1, -1, -1], boxMax: [1, 1, 1]))
    }

    func testRayIntoScaledSpaceKeepsDistance() {
        // A box at x = 10, scaled ×2: world distance from the origin to its near face is 8.
        let worldFromLocal = simd_float4x4(diagonal: [2, 2, 2, 1]) * simd_float4x4(translation: [5, 0, 0])
        let local = PartPicker.ray(origin: .zero, direction: [1, 0, 0], into: worldFromLocal)
        let t = PartPicker.hitDistance(origin: local.origin, direction: local.direction, boxMin: [-1, -1, -1], boxMax: [1, 1, 1])
        XCTAssertEqual(t ?? .nan, 8, accuracy: 1e-5)
    }

    // MARK: Labels

    func testLabelsPreferExactThenLongestPrefix() throws {
        let labels = try PartLabels(json: Data(#"{"_about": "x", "Internal_*": "inner", "Internal_w_*": "wire", "Shell": "shell"}"#.utf8))
        XCTAssertEqual(labels.label(for: "Shell"), "shell")
        XCTAssertEqual(labels.label(for: "Internal_w_red"), "wire")
        XCTAssertEqual(labels.label(for: "Internal_board"), "inner")
        XCTAssertNil(labels.label(for: "_about"))
        XCTAssertNil(labels.label(for: "Sugarcube"))
    }

    func testBundledLabelsCoverTheContract() {
        let labels = PartLabels.load(bundle: Bundle(for: Self.self))
        let contract = ["Shell", "Lid", "Board", "Cell", "BalancePlate", "Wiring"]
            + DieFace.allCases.flatMap { ["Window_\($0.rawValue)", "Module_\($0.rawValue)"] }
            + (0..<4).flatMap { ["Screw_\($0)", "Pillar_\($0)"] }
        for name in contract { XCTAssertNotNil(labels.label(for: name), name) }
        XCTAssertEqual(labels.label(for: "BalancePlate"), "Tungsten balance plate")
    }

    // MARK: Catalog

    func testCaptions() {
        let rainbow = ModelCatalog.all.first { $0.id == "sugarcube_30_rainbow" }
        XCTAssertEqual(rainbow.map { "\($0.title) · \($0.sizeLabel)" }, "30 mm · Rainbow · 30.0 mm")
        let lineup = ModelCatalog.all.first { $0.id == "sugarcube_lineup" }
        XCTAssertEqual(lineup?.sizeLabel, "156.0 × 40.0 × 40.0 mm")
    }

    // MARK: Size and colour

    @MainActor
    func testStartsOnThe30mmRainbow() {
        XCTAssertEqual(viewer().selected?.id, "sugarcube_30_rainbow")
    }

    @MainActor
    func testSizesAndColours() {
        let viewer = viewer()
        XCTAssertEqual(viewer.sizes, ["30 mm", "34 mm", "40 mm"])
        XCTAssertEqual(viewer.colours.map(\.variant), ["Rainbow", "Colour"])
        viewer.chooseSize("34 mm")
        XCTAssertEqual(viewer.selected?.id, "sugarcube_34")
        XCTAssertEqual(viewer.colours.map(\.variant), ["16-grey", "1-bit"])
        viewer.choose(viewer.colours[1])
        XCTAssertEqual(viewer.selected?.id, "sugarcube_34_1bit")
    }

    @MainActor
    func testChangingSizeKeepsTheColourWhereItCan() {
        let viewer = viewer()
        viewer.choose(viewer.colours[1])
        XCTAssertEqual(viewer.selected?.id, "sugarcube_30")
        viewer.chooseSize("40 mm")
        XCTAssertEqual(viewer.selected?.id, "sugarcube_40")
    }

    @MainActor
    func testXrayStaysOnAcrossSizeAndColour() {
        let viewer = viewer()
        viewer.toggleXray()
        XCTAssertEqual(viewer.selected?.id, "sugarcube_30_xray")
        // The 30 mm dice share an x-ray: choosing the other colour keeps it, and x-ray off goes to that colour.
        viewer.choose(viewer.colours[1])
        XCTAssertEqual(viewer.selected?.id, "sugarcube_30_xray")
        XCTAssertEqual(viewer.die?.id, "sugarcube_30")
        viewer.chooseSize("34 mm")
        XCTAssertEqual(viewer.selected?.id, "sugarcube_34_xray")
        XCTAssertEqual(viewer.die?.id, "sugarcube_34")
        viewer.toggleXray()
        XCTAssertEqual(viewer.selected?.id, "sugarcube_34")
    }

    @MainActor
    func testExplodedXrayRemembersTheColour() {
        let viewer = viewer()
        viewer.toggleXray()
        viewer.toggleExplodedFile()
        XCTAssertEqual(viewer.selected?.id, "sugarcube_30_xray_exploded")
        viewer.choose(viewer.colours[1])
        XCTAssertEqual(viewer.selected?.id, "sugarcube_30_xray_exploded")
        viewer.toggleXray()
        XCTAssertEqual(viewer.selected?.id, "sugarcube_30")
    }

    /// A viewer with every catalog model available, whether or not it's bundled.
    @MainActor
    private func viewer() -> ARViewerModel {
        ARViewerModel(catalog: ModelCatalog(source: EverythingSource()), labels: try! PartLabels(json: Data("{}".utf8)))
    }
}

private struct EverythingSource: ModelSource {
    func isAvailable(_ model: SugarcubeModel) -> Bool { model.kind != .fixture }
    func url(for model: SugarcubeModel) async throws -> URL { throw URLError(.fileDoesNotExist) }
}

private extension simd_float4x4 {
    init(translation t: SIMD3<Float>) {
        self = matrix_identity_float4x4
        columns.3 = SIMD4(t, 1)
    }
}
