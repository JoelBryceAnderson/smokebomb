import RealityKit
import XCTest

/// Explode, the shell fade and taking the lid off, against the per-part contract fixture.
@MainActor
final class DieRigTests: XCTestCase {
    private func loadFixture() async throws -> (rig: DieRig, pivot: Entity) {
        let bundle = Bundle(for: Self.self)
        let catalog = ModelCatalog(source: BundleModelSource(bundle: bundle))
        let model = try XCTUnwrap(catalog.model(id: "sugarcube_contract_fixture"))
        let entity = try await catalog.load(model)
        let pivot = Entity()
        pivot.addChild(entity)
        return (DieRig(model: model, root: entity, reference: pivot), pivot)
    }

    func testFindsContractParts() async throws {
        let (rig, _) = try await loadFixture()
        XCTAssertTrue(rig.canExplode)
        XCTAssertTrue(rig.canFadeShell)
    }

    func testExplodeMovesWindowsAndModulesAlongTheirNormals() async throws {
        let (rig, pivot) = try await loadFixture()
        let root = rig.root
        func position(_ name: String) throws -> SIMD3<Float> {
            try XCTUnwrap(root.findEntity(named: name)).position(relativeTo: pivot)
        }
        let before = try DieFace.allCases.flatMap { [try position("Window_\($0.rawValue)"), try position("Module_\($0.rawValue)")] }
        let boardBefore = try position("Board")

        rig.setExplode(1)
        for (i, face) in DieFace.allCases.enumerated() {
            let window = try position("Window_\(face.rawValue)") - before[2 * i]
            let module = try position("Module_\(face.rawValue)") - before[2 * i + 1]
            XCTAssertLessThan(simd_distance(window, face.normal * 0.026), 1e-6, "Window_\(face.rawValue)")
            XCTAssertLessThan(simd_distance(module, face.normal * 0.014), 1e-6, "Module_\(face.rawValue)")
        }
        XCTAssertEqual(try position("Board"), boardBefore, "parts outside the contract's explode list stay still")

        rig.setExplode(0)
        let after = try DieFace.allCases.flatMap { [try position("Window_\($0.rawValue)"), try position("Module_\($0.rawValue)")] }
        for (a, b) in zip(before, after) { XCTAssertLessThan(simd_distance(a, b), 1e-6) }
    }

    func testShellFade() async throws {
        let (rig, _) = try await loadFixture()
        let shell = try XCTUnwrap(rig.shell)
        rig.setShellFaded(true)
        XCTAssertEqual(shell.components[OpacityComponent.self]?.opacity, DieRig.xrayShellOpacity)
        rig.setShellFaded(false)
        XCTAssertNil(shell.components[OpacityComponent.self])
    }

    func testMenuPanelStandsUprightFacingTheViewer() {
        let parent = Entity()
        let panel = MenuPanel(side: 0.03)
        panel.show(at: [0.05, 0.1, 0], rotation: simd_quatf(angle: 0, axis: [0, 1, 0]), in: parent)
        XCTAssertEqual(panel.root.children.count, 7, "the pane and six keys")
        XCTAssertTrue(panel.root.parent === parent)
        // Its face (+Z) toward the viewer, its up still up.
        let q = panel.root.orientation
        XCTAssertEqual(simd_distance(q.act([0, 0, 1]), [0, 0, 1]), 0, accuracy: 1e-5)
        XCTAssertEqual(simd_distance(q.act([0, 1, 0]), [0, 1, 0]), 0, accuracy: 1e-5)
        XCTAssertGreaterThan(panel.width, 0.03)
        panel.hide()
        XCTAssertNil(panel.root.parent)
    }

    func testLidComesOffAndGoesBackOn() async throws {
        let (rig, pivot) = try await loadFixture()
        let root = rig.root
        let lidPrim = try XCTUnwrap(root.findEntity(named: "Lid"))
        // The loaded tree has the default prim's entity between the root and the parts.
        let lidParent = try XCTUnwrap(lidPrim.parent)
        let screen = try XCTUnwrap(root.findEntity(named: "Screen_ny"))
        let before = screen.position(relativeTo: pivot)

        let lid = try XCTUnwrap(DieLid(dieRoot: root, reference: pivot, side: 0.030))
        XCTAssertTrue(lidPrim.parent === lid.root, "the per-part lid moves over whole")
        XCTAssertTrue(lid.root.findEntity(named: "Window_ny") != nil)
        XCTAssertTrue(lid.root.findEntity(named: "Module_ny") != nil)
        XCTAssertNil(lid.root.findEntity(named: "Pillar_0"), "the pillars stay in the cup")
        XCTAssertNil(lid.root.findEntity(named: "Module_py"))
        // Each screw on an entity of its own, in its corner.
        XCTAssertEqual(lid.screws.count, 4)
        XCTAssertTrue(lid.screws.allSatisfy { $0.children.count == 1 })
        XCTAssertNil(lid.root.findEntity(named: "Screw_0"), "the screws aren't the lid's")
        XCTAssertLessThan(simd_distance(screen.position(relativeTo: pivot), before), 1e-6, "nothing moves until the lid does")

        lid.root.position = [0.05, 0, 0]
        XCTAssertGreaterThan(simd_distance(screen.position(relativeTo: pivot), before), 0.04)

        lid.restore()
        XCTAssertNil(lid.root.parent)
        XCTAssertTrue(lidPrim.parent === lidParent, "back where it was")
        XCTAssertLessThan(simd_distance(screen.position(relativeTo: pivot), before), 1e-6)
        XCTAssertEqual(rig.measuredSize().x, 0.030, accuracy: 0.0001)
    }
}
