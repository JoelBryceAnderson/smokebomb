import simd
import XCTest

/// What the firmware is fed: IMU readings from the die's motion, ticks at
/// exactly 60 Hz, touches on the right face, frames on the right axes.
final class FirmwareLoopTests: XCTestCase {
    private let still = simd_quatf(angle: 0, axis: [0, 1, 0])

    private func readings(_ poses: [DiePose]) -> [ImuReading] {
        var synth = ImuSynth()
        return poses.map { synth.reading(at: $0) }
    }

    private func magnitude(_ r: ImuReading) -> Float {
        simd_length(SIMD3<Float>(Float(r.accelMg.x), Float(r.accelMg.y), Float(r.accelMg.z)))
    }

    // MARK: ImuSynth

    func testAtRestReadsOneGOnTheUpFace() {
        let r = readings(Array(repeating: DiePose(position: .zero, orientation: still), count: 3))
        XCTAssertEqual(r.last?.accelMg, SIMD3(0, 1000, 0))
        XCTAssertEqual(r.last?.gyroMdps, SIMD3(0, 0, 0))
    }

    func testPlusXUpReadsOnX() {
        // +90° about Z turns the die's +X to point up.
        let q = simd_quatf(angle: .pi / 2, axis: [0, 0, 1])
        let r = readings(Array(repeating: DiePose(position: .zero, orientation: q), count: 3))
        XCTAssertEqual(r.last?.accelMg, SIMD3(1000, 0, 0))
    }

    func testFreeFallReadsNearZero() {
        let dt = ImuSynth.tick
        let poses = (0..<6).map { k in
            let t = Float(k) * dt
            return DiePose(position: [0.5 * t, 0.2 + 0.4 * t - 0.5 * ImuSynth.gravity * t * t, 0], orientation: still)
        }
        let r = readings(poses)
        XCTAssertLessThan(magnitude(r.last!), 5, "a falling die reads ~0 g")
    }

    func testSpinReadsOnTheGyro() {
        let rate: Float = 2  // rad/s about world Y
        let poses = (0..<4).map { k in
            DiePose(position: .zero, orientation: simd_quatf(angle: rate * Float(k) * ImuSynth.tick, axis: [0, 1, 0]))
        }
        let gyro = readings(poses).last!.gyroMdps
        XCTAssertEqual(Float(gyro.y), rate * 180 / .pi * 1000, accuracy: 200)
        XCTAssertEqual(gyro.x, 0)
        XCTAssertEqual(gyro.z, 0)
    }

    func testTheWindupStaysBetweenFreeFallAndImpact() {
        // The firmware takes < 350 mg as free fall and > 2500 mg as an impact;
        // the windup must read as handled, then shaking, and never as either.
        let total = DiePhysics.windupLiftTime + DiePhysics.windupShakeTime
        let poses = stride(from: 0.0, through: total, by: 1.0 / ImuSynth.tickHz).map { t in
            let (offset, rock) = DiePhysics.windup(at: t)
            return DiePose(position: offset, orientation: rock)
        }
        let mags = readings(poses).dropFirst(2).map(magnitude)
        XCTAssertGreaterThan(mags.min()!, 350)
        XCTAssertLessThan(mags.max()!, 2500)
        XCTAssertGreaterThan(mags.filter { abs($0 - 1000) > 700 }.count, 3, "the shake reads as shaking")
    }

    func testTheLockedWindupStillReadsAsAPickUpAndAShake() {
        let total = DiePhysics.windupLiftTime + DiePhysics.windupShakeTime
        let poses = stride(from: 0.0, through: total, by: 1.0 / ImuSynth.tickHz).map { t in
            let (offset, rock) = DiePhysics.windup(at: t, lift: DiePhysics.lockedWindupLift)
            return DiePose(position: offset, orientation: rock)
        }
        let mags = readings(poses).dropFirst(2).map(magnitude)
        XCTAssertGreaterThan(mags.min()!, 350)
        XCTAssertLessThan(mags.max()!, 2500)
        XCTAssertTrue(mags.prefix(15).contains { abs($0 - 1000) > 80 }, "the lift reads as handled")
        XCTAssertGreaterThan(mags.filter { abs($0 - 1000) > 700 }.count, 3, "the shake reads as shaking")
    }

    // MARK: TickClock

    func testTicksAtSixtyHertzWhateverTheFrameRate() {
        var clock = TickClock()
        let pose = DiePose(position: .zero, orientation: still)
        _ = clock.advance(to: 0, pose: pose)
        var ticks = 0
        // Two seconds at a jittery 120 Hz.
        var t = 0.0
        for i in 0..<240 {
            t += i % 2 == 0 ? 1.0 / 100 : 1.0 / 150
            ticks += clock.advance(to: t, pose: pose).count
        }
        XCTAssertEqual(Double(ticks), t * 60, accuracy: 1)
    }

    func testTicksInterpolateBetweenFrames() {
        var clock = TickClock()
        _ = clock.advance(to: 0, pose: DiePose(position: .zero, orientation: still))
        // One 30 Hz frame moving 2 cm: two ticks, at 1 and 2 cm.
        let poses = clock.advance(to: 1.0 / 30, pose: DiePose(position: [0.02, 0, 0], orientation: still))
        XCTAssertEqual(poses.count, 2)
        XCTAssertEqual(poses[0].position.x, 0.01, accuracy: 1e-5)
        XCTAssertEqual(poses[1].position.x, 0.02, accuracy: 1e-5)
    }

    func testAStallDropsTicksRatherThanCatchingUp() {
        var clock = TickClock()
        _ = clock.advance(to: 0, pose: DiePose(position: .zero, orientation: still))
        XCTAssertEqual(clock.advance(to: 2, pose: DiePose(position: .zero, orientation: still)).count, TickClock.maxTicksPerFrame)
        XCTAssertEqual(clock.advance(to: 2 + 1.0 / 60, pose: DiePose(position: .zero, orientation: still)).count, 1)
    }

    // MARK: Faces

    func testTouchFindsTheFaceTheFingerLandsOn() {
        let lo = SIMD3<Float>(-0.015, 0, -0.015), hi = SIMD3<Float>(0.015, 0.03, 0.015)
        func face(_ o: SIMD3<Float>, _ d: SIMD3<Float>) -> DieFace? {
            PartPicker.entryFace(origin: o, direction: d, boxMin: lo, boxMax: hi)
        }
        XCTAssertEqual(face([0, 1, 0], [0, -1, 0]), .py)
        XCTAssertEqual(face([0, -1, 0], [0, 1, 0]), .ny)
        XCTAssertEqual(face([1, 0.015, 0], [-1, 0, 0]), .px)
        XCTAssertEqual(face([-1, 0.015, 0], [1, 0, 0]), .nx)
        XCTAssertEqual(face([0, 0.015, 1], [0, 0, -1]), .pz)
        XCTAssertEqual(face([0, 0.015, -1], [0, 0, 1]), .nz)
        // At an angle: steep enough lands on the top, shallower on the +Z side.
        XCTAssertEqual(face([0, 0.5, 0.3], simd_normalize([0, -0.5, -0.31])), .py)
        XCTAssertEqual(face([0, 0.5, 0.3], simd_normalize([0, -0.5, -0.29])), .pz)
        XCTAssertNil(face([1, 1, 1], [0, 1, 0]))
    }

    func testScreenAxesMatchTheFirmware() {
        // orientation::BASES: right × up is the face's outward normal.
        for face in DieFace.allCases {
            let (right, up) = face.screenAxes
            XCTAssertEqual(simd_cross(right, up), face.normal, "\(face)")
        }
        XCTAssertEqual(DieFace.allCases.map(\.index), [0, 1, 2, 3, 4, 5], "the firmware's face order")
    }

    func testAJitteryDragNeverReadsAsAShake() {
        // A finger sliding the die with a centimetre of jitter each tick.
        let poses = (0..<30).map { k in
            DiePose(position: [Float(k) * 0.005 + (k % 2 == 0 ? 0.01 : -0.01), 0, 0], orientation: still)
        }
        var synth = ImuSynth()
        let mags = poses.map { synth.reading(at: $0, maxLinearMg: DiePhysics.handlingMaxLinearMg) }.map(magnitude)
        // The firmware's shake: more than 0.7 g off 1 g; free fall: under 0.35 g.
        XCTAssertLessThan(mags.map { abs($0 - 1000) }.max()!, 700)
        XCTAssertGreaterThan(mags.min()!, 350)
    }

    // MARK: Held for the menu, turned, set down

    func testHeldFacesTheCameraWithItsEdgeUp() {
        let q = DiePhysics.heldOrientation(front: .pz, current: still, toCamera: [0, 0.2, 1])
        let facing = q.act(DieFace.pz.normal)
        let tilt = atan2(Float(0.2), 1)
        XCTAssertEqual(facing.y, sin(tilt), accuracy: 1e-4)
        XCTAssertEqual(facing.z, cos(tilt), accuracy: 1e-4)
        XCTAssertGreaterThan(q.act(DieFace.pz.screenAxes.up).y, 0.9, "the screen stays upright")
    }

    func testHeldTiltIsCapped() {
        let q = DiePhysics.heldOrientation(front: .px, current: still, toCamera: [1, 10, 0])
        XCTAssertEqual(asin(q.act(DieFace.px.normal).y), DiePhysics.heldMaxTilt, accuracy: 1e-4)
    }

    func testSettingDownPutsTheLowestFaceFlat() {
        // Tipped 30° forward and turned: −Y is still lowest.
        let current = simd_quatf(angle: 0.5, axis: [1, 0, 0]) * simd_quatf(angle: 0.7, axis: [0, 1, 0])
        let q = DiePhysics.setDownOrientation(current: current)
        XCTAssertEqual(q.act(DieFace.ny.normal).y, -1, accuracy: 1e-5)
        // Turned over: +Z lowest goes flat.
        let over = DiePhysics.setDownOrientation(current: simd_quatf(angle: 1.4, axis: [1, 0, 0]))
        XCTAssertEqual(over.act(DieFace.pz.normal).y, -1, accuracy: 1e-5)
    }

    func testTheTurnPadLiesInTheMenuScreensPlane() {
        let die = DiePhysics.heldOrientation(front: .px, current: still, toCamera: [1, 0.3, 0.2])
        let pad = DiePhysics.menuPanelRotation(dieRotation: die, front: .px)
        // Same facing as the menu screen, and upright as the screen reads.
        XCTAssertEqual(simd_distance(pad.act([0, 0, 1]), die.act(DieFace.px.normal)), 0, accuracy: 1e-5)
        XCTAssertGreaterThan(pad.act([0, 1, 0]).y, 0.8)
        XCTAssertEqual(simd_dot(pad.act([1, 0, 0]), die.act(DieFace.px.normal)), 0, accuracy: 1e-5, "its sideways is in the plane")
    }

    func testQuarterTurnsUseTheDiesOwnAxes() {
        let yawed = simd_quatf(angle: 0.3, axis: [0, 1, 0])
        let axis = DiePhysics.nearestDieAxis(to: [1, 0, 0], orientation: yawed)
        XCTAssertEqual(simd_length(axis - yawed.act([1, 0, 0])), 0, accuracy: 1e-6)
    }

    func testPanelsPerDie() {
        let panel = { (id: String) in ModelCatalog.all.first { $0.id == id }?.panel }
        XCTAssertEqual(panel("sugarcube_34"), .grey96)
        XCTAssertEqual(panel("sugarcube_34_1bit"), .grey96)
        XCTAssertEqual(panel("sugarcube_30_rainbow"), .rgb64)
        XCTAssertEqual(panel("sugarcube_40"), .rgb64)
        XCTAssertNil(panel("sugarcube_lineup"))
    }
}
