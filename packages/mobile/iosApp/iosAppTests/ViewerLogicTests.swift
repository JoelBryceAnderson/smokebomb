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

    // MARK: Lid

    func testLidCutFollowsTheSeam() {
        let g = DieLid.Geometry(side: 0.034)
        XCTAssertEqual(g.edgeRadius, 0.0025, accuracy: 1e-7)
        XCTAssertEqual(g.seamHeight, 0.0025 * (1 - sin(Float.pi / 4)), accuracy: 1e-7)
        // The lid face, and the bottom edge below the seam.
        XCTAssertTrue(g.isLid([0, 0, 0]))
        XCTAssertTrue(g.isLid([0.0165, 0.0005, 0]))
        // The lid's flat inside, away from the walls.
        XCTAssertTrue(g.isLid([0.005, 0.002, -0.005]))
        // The cup's lower edge above the seam, its walls and its top.
        XCTAssertFalse(g.isLid([0.0168, 0.0015, 0]))
        XCTAssertFalse(g.isLid([0.017, 0.01, 0]))
        XCTAssertFalse(g.isLid([0, 0.034, 0]))
        // Scaled with the die.
        XCTAssertEqual(DieLid.Geometry(side: 0.030).edgeRadius, 0.0025 * 30 / 34, accuracy: 1e-7)
    }

    func testLidPartsByName() {
        XCTAssertEqual(DieLid.rule(for: "Screen_ny"), .lid)
        // The board and what's on it come off together.
        for name in ["Internal_board", "Internal_ic", "Internal_lra", "Internal_ind", "Internal_w_charge"] {
            XCTAssertEqual(DieLid.rule(for: name), .lid, name)
        }
        // Pieces on every face: the lid takes its own.
        for name in ["SapphireWindows", "Internal_fpc", "Internal_panel_glass"] {
            XCTAssertEqual(DieLid.rule(for: name), .byFace, name)
        }
        // The screws come out on their own.
        for name in ["LidScrews", "ScrewSleevesAndSlots", "Screw_2"] {
            XCTAssertEqual(DieLid.rule(for: name), .screws, name)
        }
        // The cup keeps its cell and frame; the wiring into the board is unplugged.
        for name in ["Pillar_0", "Internal_battery", "Internal_frame", "Internal_pillar"] {
            XCTAssertEqual(DieLid.rule(for: name), .cup, name)
        }
        for name in ["LidSeam", "Internal_w_spi_a", "Internal_w_batt"] {
            XCTAssertEqual(DieLid.rule(for: name), .hidden, name)
        }
        XCTAssertNil(DieLid.rule(for: "Shell"))
        XCTAssertNil(DieLid.rule(for: "Screen_py"))
    }

    func testNearestFace() {
        let g = DieLid.Geometry(side: 0.030)
        XCTAssertEqual(g.nearestFace([0, 0.002, 0]), .ny)
        XCTAssertEqual(g.nearestFace([0.012, 0.002, 0]), .ny)
        XCTAssertEqual(g.nearestFace([0.0134, 0.01, 0]), .px)
        XCTAssertEqual(g.nearestFace([0, 0.028, 0]), .py)
    }

    func testLidPlateMeetsTheTaper() {
        let g = DieLid.Geometry(side: 0.030)
        XCTAssertLessThan(g.tunnel.half, g.flatHalf)
        XCTAssertGreaterThan(g.tunnel.half, g.window.half)
        // The outlines line up point for point, a corner's middle on the diagonal.
        let outline = DieLid.roundedOutline(half: 0.01, radius: 0.002)
        XCTAssertEqual(outline.count, 36)
        XCTAssertEqual(outline[4].x, outline[4].y, accuracy: 1e-7)
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
        ARViewerModel(catalog: ModelCatalog(source: EverythingSource()))
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
