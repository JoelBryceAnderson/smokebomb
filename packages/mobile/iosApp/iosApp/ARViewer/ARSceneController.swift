import ARKit
import Combine
import RealityKit
import UIKit
import UIKit.UIGestureRecognizerSubclass

/// Owns the ARView: the AR session, plane coaching, placing the die, gestures and physics.
/// UI state lives in `ARViewerModel`; this pushes changes back to it.
///
/// The scene, all under one anchor at the spot you tapped:
///
///     placement (AnchorEntity, world)
///     ├── table   static collider, top face at y = 0
///     └── pivot   position on the table, yaw, user scale; the physics body
///         └── rig.root   the loaded model (its own metres, origin at the bottom centre)
///
/// Table and die share the anchor so they share one physics simulation.
///
/// With live screens on, the real firmware runs here too: every 1/60 s it gets
/// the IMU reading the die's motion makes (`ImuSynth`) and the faces being
/// touched, and its six panels are drawn on the die (`LiveScreens`).
@MainActor
final class ARSceneController: NSObject, UIGestureRecognizerDelegate {
    let arView: ARView
    private unowned let model: ARViewerModel

    private let coaching = ARCoachingOverlayView()
    private var placement: AnchorEntity?
    private let table = Entity()
    private let pivot = Entity()
    private(set) var rig: DieRig?
    /// Where Reset puts the die: the placed spot, as last moved or turned.
    private var restTransform = Transform.identity
    private var updates: Cancellable?
    private var loadTask: Task<Void, Never>?

    // Throwing.
    private var settle = SettleDetector()
    private var lastPose: (position: SIMD3<Float>, orientation: simd_quatf)?
    /// A throw in its windup: picked up and shaken before release.
    private struct Windup {
        let start: TimeInterval
        let base: Transform
        let velocity: SIMD3<Float>
        let spin: SIMD3<Float>
    }
    private var windup: Windup?
    /// Seconds since the scene started, summed from frame times.
    private var time: TimeInterval = 0

    // The firmware, when live screens are on.
    private var firmware: DieFirmware?
    private var firmwarePanel: PanelKind?
    private var screens: LiveScreens?
    private var imu = ImuSynth()
    private var ticks = TickClock()
    private var frameSeq: UInt64 = 0
    private var touchMask: UInt8 = 0
    private var faceBuffer: [UInt8] = []
    private let lightHaptic = UIImpactFeedbackGenerator(style: .light)
    private let heavyHaptic = UIImpactFeedbackGenerator(style: .heavy)
    private let mediumHaptic = UIImpactFeedbackGenerator(style: .medium)
    private let notifyHaptic = UINotificationFeedbackGenerator()

    // Gestures.
    private enum PanMode { case move, turn }
    private var panMode: PanMode?
    private var lastPanX: CGFloat = 0
    private var pinchStartScale: Float = 1

    init(model: ARViewerModel) {
        self.model = model
        arView = ARView(frame: .zero, cameraMode: .ar, automaticallyConfigureSession: false)
        super.init()
        arView.renderOptions.formUnion([.disableMotionBlur, .disableDepthOfField])

        table.name = "Table"
        table.components.set(CollisionComponent(shapes: [Self.tableShape()]))
        table.components.set(PhysicsBodyComponent(
            shapes: [Self.tableShape()], mass: 0,
            material: .generate(staticFriction: DiePhysics.tableStaticFriction,
                                dynamicFriction: DiePhysics.tableDynamicFriction,
                                restitution: DiePhysics.tableRestitution),
            mode: .static))
        pivot.name = "Die"

        coaching.session = arView.session
        coaching.goal = .horizontalPlane
        coaching.activatesAutomatically = true
        coaching.translatesAutoresizingMaskIntoConstraints = false
        arView.addSubview(coaching)
        NSLayoutConstraint.activate([
            coaching.leadingAnchor.constraint(equalTo: arView.leadingAnchor),
            coaching.trailingAnchor.constraint(equalTo: arView.trailingAnchor),
            coaching.topAnchor.constraint(equalTo: arView.topAnchor),
            coaching.bottomAnchor.constraint(equalTo: arView.bottomAnchor),
        ])
        installGestures()
    }

    // MARK: Session

    func start() {
        let config = ARWorldTrackingConfiguration()
        config.planeDetection = [.horizontal]
        config.environmentTexturing = .automatic
        if DiePhysics.useSceneReconstruction, ARWorldTrackingConfiguration.supportsSceneReconstruction(.mesh) {
            config.sceneReconstruction = .mesh
            arView.environment.sceneUnderstanding.options.formUnion([.collision, .physics])
        }
        arView.session.run(config, options: [.resetTracking, .removeExistingAnchors])
        model.sceneDidStart()
        updates = arView.scene.subscribe(to: SceneEvents.Update.self) { [weak self] event in
            self?.update(dt: event.deltaTime)
        }
    }

    func stop() {
        loadTask?.cancel()
        dropFirmware()
        updates?.cancel()
        updates = nil
        arView.session.pause()
    }

    private func update(dt: TimeInterval) {
        if model.isCoaching != coaching.isActive { model.isCoaching = coaching.isActive }
        if !model.planeFound, arView.session.currentFrame?.anchors.contains(where: { $0 is ARPlaneAnchor }) == true {
            model.planeFound = true
        }
        time += dt
        if windup != nil { stepWindup() } else if model.isRolling { trackRoll(dt: dt) }
        if firmware != nil { stepFirmware() }
    }

    // MARK: Showing models

    /// Swaps in a model, in the same spot if one is placed.
    func show(_ next: SugarcubeModel) {
        loadTask?.cancel()
        model.isLoading = true
        loadTask = Task { [weak self] in
            guard let self else { return }
            do {
                let entity = try await self.model.catalog.load(next)
                guard !Task.isCancelled else { return }
                self.install(entity, as: next)
            } catch {
                guard !Task.isCancelled else { return }
                self.model.loadFailed(error)
            }
            self.model.isLoading = false
        }
    }

    private func install(_ entity: Entity, as next: SugarcubeModel) {
        if model.isRolling { reset() }
        rig?.highlight(nil)
        rig?.root.removeFromParent()
        pivot.addChild(entity)
        let rig = DieRig(model: next, root: entity, reference: pivot, labels: model.labels)
        self.rig = rig
        rig.setExplode(model.explode)
        rig.setShellFaded(model.shellFaded)
        updateBody()
        model.rigDidLoad(rig, measured: rig.measuredSize())
        syncFirmware()
    }

    func setExplode(_ e: Float) { rig?.setExplode(e) }
    func setShellFaded(_ faded: Bool) { rig?.setShellFaded(faded) }

    func setScale(_ scale: Float) {
        pivot.scale = SIMD3(repeating: scale)
        restTransform.scale = pivot.scale
    }

    /// The die's physics body: a box at its real size, kinematic until thrown.
    private func updateBody() {
        guard let rig else { return }
        let size = rig.model.bounds
        let shape = ShapeResource.generateBox(size: size).offsetBy(translation: [0, size.y / 2, 0])
        pivot.components.set(CollisionComponent(shapes: [shape]))
        var body = PhysicsBodyComponent(
            shapes: [shape], mass: rig.model.massKg ?? 0.1,
            material: .generate(staticFriction: DiePhysics.staticFriction,
                                dynamicFriction: DiePhysics.dynamicFriction,
                                restitution: DiePhysics.restitution),
            mode: .kinematic)
        body.isContinuousCollisionDetectionEnabled = DiePhysics.continuousCollisionDetection
        body.linearDamping = DiePhysics.linearDamping
        body.angularDamping = DiePhysics.angularDamping
        pivot.components.set(body)
    }

    private static func tableShape() -> ShapeResource {
        // Larger than any table, so a hard throw still lands; 2 cm thick against tunnelling.
        ShapeResource.generateBox(size: [6, 0.02, 6]).offsetBy(translation: [0, -0.01, 0])
    }

    // MARK: Placing

    private func place(at point: CGPoint) -> Bool {
        let hits = arView.raycast(from: point, allowing: .existingPlaneGeometry, alignment: .horizontal)
        guard let hit = hits.first ?? arView.raycast(from: point, allowing: .estimatedPlane, alignment: .horizontal).first
        else { return false }

        placement?.removeFromParent()
        let anchor = AnchorEntity(world: hit.worldTransform)
        anchor.addChild(table)
        anchor.addChild(pivot)
        arView.scene.addAnchor(anchor)
        placement = anchor

        // Turn the +Z face towards the camera.
        let toCamera = anchor.convert(position: arView.cameraTransform.translation, from: nil)
        let yaw = atan2(toCamera.x, toCamera.z)
        pivot.transform = Transform(scale: SIMD3(repeating: model.scale), rotation: simd_quatf(angle: yaw, axis: [0, 1, 0]), translation: .zero)
        restTransform = pivot.transform
        model.isPlaced = true
        imu.reset()
        ticks.reset()
        syncFirmware()
        return true
    }

    /// Takes the die off the table so the next tap places it again.
    func unplace() {
        if model.isRolling { reset() }
        placement?.removeFromParent()
        placement = nil
        model.isPlaced = false
        syncFirmware()
    }

    // MARK: Gestures

    private func installGestures() {
        let tap = UITapGestureRecognizer(target: self, action: #selector(didTap(_:)))
        let pan = UIPanGestureRecognizer(target: self, action: #selector(didPan(_:)))
        pan.maximumNumberOfTouches = 1
        let twist = UIRotationGestureRecognizer(target: self, action: #selector(didTwist(_:)))
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(didPinch(_:)))
        let touch = TouchTracker { [weak self] point in self?.touchChanged(at: point) }
        for recognizer in [tap, pan, twist, pinch, touch] as [UIGestureRecognizer] {
            recognizer.delegate = self
            arView.addGestureRecognizer(recognizer)
        }
    }

    nonisolated func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        // Twist and pinch together; a one-finger pan stays on its own.
        !(g is UIPanGestureRecognizer) && !(other is UIPanGestureRecognizer)
    }

    @objc private func didTap(_ g: UITapGestureRecognizer) {
        let point = g.location(in: arView)
        guard model.isPlaced, let rig else {
            _ = place(at: point)
            return
        }
        guard let ray = arView.ray(through: point) else { return }
        // With live screens, a tap on the die is a touch for the firmware
        // (TouchTracker sent it); parts are named in x-ray.
        if firmware != nil, !model.isXray, rig.contains(origin: ray.origin, direction: ray.direction) {
            rig.highlight(nil)
            model.selectedPart = nil
            return
        }
        let part = rig.pick(origin: ray.origin, direction: ray.direction, seeThroughShell: model.isXray)
        rig.highlight(part)
        model.selectedPart = part?.label
    }

    @objc private func didPan(_ g: UIPanGestureRecognizer) {
        guard model.isPlaced, !model.isRolling, let rig, let placement else { return }
        let point = g.location(in: arView)
        switch g.state {
        case .began:
            let onDie = arView.ray(through: point).map { rig.contains(origin: $0.origin, direction: $0.direction) } ?? false
            panMode = onDie ? .move : .turn
            lastPanX = g.translation(in: arView).x
        case .changed:
            if panMode == .move {
                // Slide along the table: where the finger meets the plane, in the anchor's space.
                if let hit = arView.raycast(from: point, allowing: .estimatedPlane, alignment: .horizontal).first {
                    let world = SIMD3(hit.worldTransform.columns.3.x, hit.worldTransform.columns.3.y, hit.worldTransform.columns.3.z)
                    let local = placement.convert(position: world, from: nil)
                    pivot.position = [local.x, 0, local.z]
                }
            } else {
                let x = g.translation(in: arView).x
                turn(by: Float(x - lastPanX) * 0.01)
                lastPanX = x
            }
        case .ended:
            let v = g.velocity(in: arView)
            let speed = hypot(v.x, v.y)
            if panMode == .move, speed > DiePhysics.flickThreshold, rig.model.isThrowable {
                throwDie(screenDirection: CGVector(dx: v.x, dy: v.y), flickSpeed: speed)
            } else {
                restTransform = pivot.transform
            }
            panMode = nil
        default:
            panMode = nil
        }
    }

    @objc private func didTwist(_ g: UIRotationGestureRecognizer) {
        guard model.isPlaced, !model.isRolling else { return }
        turn(by: -Float(g.rotation))
        g.rotation = 0
        if g.state == .ended { restTransform = pivot.transform }
    }

    @objc private func didPinch(_ g: UIPinchGestureRecognizer) {
        guard model.isPlaced, !model.isTrueSizeLocked else { return }
        if g.state == .began { pinchStartScale = model.scale }
        model.setScale(pinchStartScale * Float(g.scale))
    }

    /// Spins the die about the table's vertical.
    func turn(by radians: Float) {
        guard model.isPlaced, !model.isRolling else { return }
        pivot.orientation = simd_quatf(angle: radians, axis: [0, 1, 0]) * pivot.orientation
        restTransform = pivot.transform
    }

    // MARK: Throwing

    /// Throws the die across the table. `screenDirection` is in view points
    /// (right, down); it's mapped onto the table as seen from the camera.
    func throwDie(screenDirection: CGVector, flickSpeed: CGFloat) {
        guard model.isPlaced, !model.isRolling, let rig, rig.model.isThrowable, let placement else { return }
        if model.explode > 0 { model.explode = 0 }

        // Camera right and forward, flattened onto the table, in the anchor's space.
        let camera = arView.cameraTransform.matrix
        let right = flatten(placement.convert(direction: SIMD3(camera.columns.0.x, camera.columns.0.y, camera.columns.0.z), from: nil))
        let forward = flatten(placement.convert(direction: -SIMD3(camera.columns.2.x, camera.columns.2.y, camera.columns.2.z), from: nil))
        var direction = right * Float(screenDirection.dx) - forward * Float(screenDirection.dy)
        if simd_length(direction) < 1e-4 { direction = forward }
        direction = simd_normalize(direction)

        let speed = DiePhysics.throwSpeed(forFlick: flickSpeed)
        let rollAxis = simd_normalize(simd_cross([0, 1, 0], direction))
        let spin = Float.random(in: DiePhysics.throwSpin)
        let wobble = SIMD3<Float>(Float.random(in: -0.3...0.3), Float.random(in: -0.5...0.5), Float.random(in: -0.3...0.3))

        restTransform = pivot.transform
        windup = Windup(
            start: time, base: pivot.transform,
            velocity: direction * speed + [0, DiePhysics.throwLift, 0],
            spin: (rollAxis + wobble) * spin)
        rig.highlight(nil)
        model.selectedPart = nil
        touchMask = 0
        model.rollDidStart()
    }

    /// Picked up, then shaken, then let go: one frame of the windup.
    private func stepWindup() {
        guard let w = windup else { return }
        let t = time - w.start
        if t >= DiePhysics.windupLiftTime + DiePhysics.windupShakeTime {
            release(w)
            return
        }
        let (offset, rock) = DiePhysics.windup(at: t)
        pivot.position = w.base.translation + offset
        pivot.orientation = rock * w.base.rotation
    }

    private func release(_ w: Windup) {
        windup = nil
        setBodyMode(.dynamic)
        pivot.components.set(PhysicsMotionComponent(linearVelocity: w.velocity, angularVelocity: w.spin))
        settle.reset()
        lastPose = nil
    }

    private func trackRoll(dt: TimeInterval) {
        guard dt > 0, let placement else { return }
        let position = pivot.position(relativeTo: placement)
        let orientation = pivot.orientation(relativeTo: nil)
        defer { lastPose = (position, orientation) }
        guard let last = lastPose else { return }

        if position.y < DiePhysics.lostBelow {
            reset()
            return
        }
        let linear = simd_length(position - last.position) / Float(dt)
        // q and -q are the same turn; take the short way round.
        var turned = (orientation * last.orientation.inverse).angle
        if turned > .pi { turned = 2 * .pi - turned }
        let angular = turned / Float(dt)
        if settle.update(linearSpeed: linear, angularSpeed: angular, dt: dt) {
            setBodyMode(.kinematic)
            model.rollDidSettle(faceUp: DieFace.faceUp(orientation: orientation))
        }
    }

    /// Puts the die back on its placed spot.
    func reset() {
        windup = nil
        imu.reset()
        ticks.reset()
        setBodyMode(.kinematic)
        pivot.components.remove(PhysicsMotionComponent.self)
        pivot.transform = restTransform
        model.rollDidReset()
    }

    private func setBodyMode(_ mode: PhysicsBodyMode) {
        guard var body = pivot.components[PhysicsBodyComponent.self] else { return }
        body.mode = mode
        pivot.components.set(body)
    }

    private func flatten(_ v: SIMD3<Float>) -> SIMD3<Float> {
        let flat = SIMD3<Float>(v.x, 0, v.z)
        return simd_length(flat) < 1e-5 ? [0, 0, -1] : simd_normalize(flat)
    }

    // MARK: Firmware

    /// Starts, keeps or stops the firmware to match: running while a die with
    /// panels is placed and live screens are on. Swapping between models with
    /// the same panels keeps it running; other panels boot it afresh.
    func syncFirmware() {
        let wanted = model.liveScreens && model.isPlaced ? rig?.model.panel : nil
        if wanted != firmwarePanel {
            dropFirmware()
            if let wanted, let made = model.makeFirmware?(wanted) {
                firmware = made
                firmwarePanel = wanted
                faceBuffer = [UInt8](repeating: 0, count: made.panelSide * made.panelSide * 4)
                imu.reset()
                ticks.reset()
                lightHaptic.prepare()
            }
        }
        screens?.remove()
        screens = nil
        if let firmware, let rig {
            let live = LiveScreens(rig: rig, reference: pivot, side: firmware.panelSide)
            screens = live.isEmpty ? nil : live
            redraw()
        }
    }

    private func dropFirmware() {
        screens?.remove()
        screens = nil
        firmware = nil
        firmwarePanel = nil
        frameSeq = 0
        touchMask = 0
        model.firmwareMode = nil
    }

    /// Runs the firmware up to now: one tick per 1/60 s, each with the IMU
    /// reading of the die's pose at that instant.
    private func stepFirmware() {
        guard let firmware else { return }
        let pose = DiePose(position: pivot.position(relativeTo: nil), orientation: pivot.orientation(relativeTo: nil))
        var seq = frameSeq
        for tickPose in ticks.advance(to: time, pose: pose) {
            seq = firmware.tick(imu.reading(at: tickPose), touchMask: touchMask)
        }
        if seq != frameSeq {
            frameSeq = seq
            redraw()
        }
        while let effect = firmware.nextHaptic() { play(haptic: effect) }
        let mode = firmware.mode
        if model.firmwareMode != mode { model.firmwareMode = mode }
    }

    private func redraw() {
        guard let firmware, let screens else { return }
        for face in DieFace.allCases where firmware.faceRGBA(face, into: &faceBuffer) {
            screens.show(faceBuffer, on: face)
        }
    }

    /// The firmware's haptic effects, felt through the phone. Numbers follow
    /// `smokebomb_hal::HapticEffect`.
    private func play(haptic effect: Int) {
        switch effect {
        case 0, 6, 9: lightHaptic.impactOccurred()  // Tick, MenuTip, DockTick
        case 2, 8: heavyHaptic.impactOccurred()  // LandingThud, SeatThunk
        case 3: notifyHaptic.notificationOccurred(.success)  // MaxCelebration
        case 4, 11: notifyHaptic.notificationOccurred(.warning)  // Dud, SoftBuzz
        default: mediumHaptic.impactOccurred()
        }
    }

    /// A finger down on the die touches the face under it, until it lifts
    /// (or turns into a drag, a pinch or a twist). In x-ray, taps name parts instead.
    private func touchChanged(at point: CGPoint?) {
        var mask: UInt8 = 0
        if let point, firmware != nil, model.isPlaced, !model.isRolling, !model.isXray,
           let rig, let ray = arView.ray(through: point) {
            let size = rig.model.bounds
            let local = PartPicker.ray(origin: ray.origin, direction: ray.direction, into: pivot.transformMatrix(relativeTo: nil))
            if let face = PartPicker.entryFace(
                origin: local.origin, direction: local.direction,
                boxMin: [-size.x / 2, 0, -size.z / 2], boxMax: [size.x / 2, size.y, size.z / 2]) {
                mask = 1 << UInt8(face.index)
            }
        }
        touchMask = mask
    }
}

/// Watches one finger without claiming it: reports where it went down and
/// when it lifts. It never recognises, so taps, drags and pinches still work;
/// when one of those takes over, UIKit resets this and the touch ends.
private final class TouchTracker: UIGestureRecognizer {
    private let changed: (CGPoint?) -> Void

    init(changed: @escaping (CGPoint?) -> Void) {
        self.changed = changed
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        if let view, let touch = touches.first, (event.allTouches?.count ?? 1) == 1 {
            changed(touch.location(in: view))
        } else {
            changed(nil)
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        changed(nil)
        state = .failed
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        changed(nil)
        state = .failed
    }

    override func reset() {
        changed(nil)
    }
}
