import RealityKit
import XCTest

/// Explode, the shell fade and tap-to-name against the per-part contract fixture.
@MainActor
final class DieRigTests: XCTestCase {
    private func loadFixture() async throws -> (rig: DieRig, pivot: Entity) {
        let bundle = Bundle(for: Self.self)
        let catalog = ModelCatalog(source: BundleModelSource(bundle: bundle))
        let model = try XCTUnwrap(catalog.model(id: "sugarcube_contract_fixture"))
        let entity = try await catalog.load(model)
        let pivot = Entity()
        pivot.addChild(entity)
        return (DieRig(model: model, root: entity, reference: pivot, labels: .load(bundle: bundle)), pivot)
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

    func testTapNamesTheOutermostLabelledPart() async throws {
        let (rig, _) = try await loadFixture()
        // Straight down onto the top face's centre: the sapphire window is first.
        let down = rig.pick(origin: [0, 1, 0], direction: [0, -1, 0], seeThroughShell: false)
        XCTAssertEqual(down?.name, "Window_py")
        // With x-ray on, the window is looked through; next is the top display module.
        let xray = rig.pick(origin: [0, 1, 0], direction: [0, -1, 0], seeThroughShell: true)
        XCTAssertEqual(xray?.name, "Module_py")
        XCTAssertEqual(xray?.label, "Display module: panel glass, driver chip and ribbon")
        // From below, near a corner: a lid screw.
        let screw = rig.pick(origin: [0.0115, -1, 0.0115], direction: [0, 1, 0], seeThroughShell: false)
        XCTAssertEqual(screw?.label, "Lid screw: M1.0, also the charging contact")
        XCTAssertNil(rig.pick(origin: [1, 1, 1], direction: [0, 1, 0], seeThroughShell: false))
    }
}
