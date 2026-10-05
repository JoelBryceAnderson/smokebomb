import ARKit
import Combine
import RealityKit
import UIKit
import UIKit.UIGestureRecognizerSubclass

/// Owns the ARView: the AR session, plane coaching, placing the die, gestures and physics.
/// With AR off (`augmented` false) the same scene sits on a virtual table in a
/// studio, seen from a fixed camera like the desktop simulator's, and needs
/// neither a camera nor ARKit.
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
    /// The camera and the real table (true), or a virtual table in a studio.
    let augmented: Bool
    private unowned let model: ARViewerModel
    /// The studio's camera, with AR off.
    private var studioCamera: PerspectiveCamera?

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
    /// Invisible walls around a throw, so the die stays close.
    private var corral: Entity?

    /// An eased move of the die about its centre: lifting it into the hand
    /// for the menu, a quarter turn from the turn pad, setting it down.
    private struct Glide {
        let start: TimeInterval
        let duration: TimeInterval
        let from: (centre: SIMD3<Float>, rotation: simd_quatf)
        let to: (centre: SIMD3<Float>, rotation: simd_quatf)
        let easeOut: Bool
    }
    private var glide: Glide?
    /// Held in the air for the menu.
    private var held = false
    /// Where a drag wants the die's centre (x, z), which it eases toward.
    private var dragTarget: SIMD2<Float>?
    /// The turn pad: a pane of glass beside the die while the menu is open.
    private var menuPanel: MenuPanel?
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

    init(model: ARViewerModel, augmented: Bool) {
        self.model = model
        self.augmented = augmented
        arView = ARView(frame: .zero, cameraMode: augmented ? .ar : .nonAR, automaticallyConfigureSession: false)
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

        installGestures()
        guard augmented else { return }
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
    }

    // MARK: Session

    func start() {
        updates = arView.scene.subscribe(to: SceneEvents.Update.self) { [weak self] event in
            self?.update(dt: event.deltaTime)
        }
        guard augmented else {
            model.sceneDidStart()
            buildStudio()
            return
        }
        let config = ARWorldTrackingConfiguration()
        config.planeDetection = [.horizontal]
        config.environmentTexturing = .automatic
        if DiePhysics.useSceneReconstruction, ARWorldTrackingConfiguration.supportsSceneReconstruction(.mesh) {
            config.sceneReconstruction = .mesh
            arView.environment.sceneUnderstanding.options.formUnion([.collision, .physics])
        }
        arView.session.run(config, options: [.resetTracking, .removeExistingAnchors])
        model.sceneDidStart()
    }

    func stop() {
        loadTask?.cancel()
        dropFirmware()
        updates?.cancel()
        updates = nil
        if augmented { arView.session.pause() }
    }

    private func update(dt: TimeInterval) {
        if augmented {
            if model.isCoaching != coaching.isActive { model.isCoaching = coaching.isActive }
            if !model.planeFound, arView.session.currentFrame?.anchors.contains(where: { $0 is ARPlaneAnchor }) == true {
                model.planeFound = true
            }
        }
        time += dt
        if windup != nil { stepWindup() } else if model.isRolling { trackRoll(dt: dt) }
        stepGlide()
        stepDrag(dt: Float(dt))
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
        guard augmented else { return false }  // the studio places the die itself
        let hits = arView.raycast(from: point, allowing: .existingPlaneGeometry, alignment: .horizontal)
        guard let hit = hits.first ?? arView.raycast(from: point, allowing: .estimatedPlane, alignment: .horizontal).first
        else { return false }
        place(on: hit.worldTransform)
        return true
    }

    /// Puts the table's origin (and the die) at `spot`, a world transform with Y up.
    private func place(on spot: simd_float4x4) {
        placement?.removeFromParent()
        let anchor = AnchorEntity(world: spot)
        anchor.addChild(table)
        anchor.addChild(pivot)
        arView.scene.addAnchor(anchor)
        placement = anchor

        // Turn the +Z face towards the camera.
        let toCamera = anchor.convert(position: cameraTransform.translation, from: nil)
        let yaw = atan2(toCamera.x, toCamera.z)
        pivot.transform = Transform(scale: SIMD3(repeating: model.scale), rotation: simd_quatf(angle: yaw, axis: [0, 1, 0]), translation: .zero)
        restTransform = pivot.transform
        model.isPlaced = true
        imu.reset()
        ticks.reset()
        syncFirmware()
    }

    /// Takes the die off the table so the next tap places it again. In the
    /// studio it goes straight back to the middle of the table.
    func unplace() {
        reset()
        guard augmented else {
            place(on: matrix_identity_float4x4)
            return
        }
        placement?.removeFromParent()
        placement = nil
        model.isPlaced = false
        syncFirmware()
    }

    /// Where the view's camera is: the phone in AR, the studio's camera otherwise.
    private var cameraTransform: Transform {
        studioCamera.map { Transform(matrix: $0.transformMatrix(relativeTo: nil)) } ?? arView.cameraTransform
    }

    /// Where a screen point's ray meets the table (y = 0 of the placement), in world space.
    private func tablePoint(at point: CGPoint) -> SIMD3<Float>? {
        guard let placement, let ray = arView.ray(through: point) else { return nil }
        let origin = placement.convert(position: ray.origin, from: nil)
        let direction = placement.convert(direction: ray.direction, from: nil)
        guard direction.y < -1e-4 else { return nil }
        let t = -origin.y / direction.y
        return placement.convert(position: origin + direction * t, to: nil)
    }

    // MARK: Studio (AR off)

    /// A dark studio: a matte table, a key light with shadows and a fixed
    /// camera looking down at the die from the front right, as the desktop
    /// simulator frames it (SIM_SPEC A5: 32° field of view, from (0.55, 0.62, 1)).
    private func buildStudio() {
        arView.environment.background = .color(UIColor(white: 0.07, alpha: 1))
        let studio = AnchorEntity(world: matrix_identity_float4x4)

        let top = ModelEntity(
            mesh: .generatePlane(width: 1.2, depth: 1.2, cornerRadius: 0.05),
            materials: [SimpleMaterial(color: UIColor(white: 0.16, alpha: 1), roughness: 0.85, isMetallic: false)])
        studio.addChild(top)

        let light = DirectionalLight()
        light.light.intensity = 2500
        light.shadow = DirectionalLightComponent.Shadow(maximumDistance: 1.5, depthBias: 2)
        light.look(at: .zero, from: [0.4, 1.2, 0.6], relativeTo: nil)
        studio.addChild(light)

        let camera = PerspectiveCamera()
        camera.camera.fieldOfViewInDegrees = DiePhysics.studioFieldOfView
        let from = simd_normalize(SIMD3<Float>(0.55, 0.62, 1)) * DiePhysics.studioCameraDistance
        camera.look(at: [0, 0.02, 0], from: from, relativeTo: nil)
        studio.addChild(camera)
        studioCamera = camera

        arView.scene.addAnchor(studio)
        place(on: matrix_identity_float4x4)
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
        if let key = menuPanel?.press(at: point, in: arView) {
            lightHaptic.impactOccurred()
            tip(key.axis, key.direction)
            return
        }
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
        guard model.isPlaced, !model.isRolling, !held, glide == nil, let rig, let placement else { return }
        let point = g.location(in: arView)
        switch g.state {
        case .began:
            let onDie = arView.ray(through: point).map { rig.contains(origin: $0.origin, direction: $0.direction) } ?? false
            panMode = onDie ? .move : .turn
            lastPanX = g.translation(in: arView).x
        case .changed:
            if panMode == .move {
                // Slide along the table: where the finger meets the plane, in the anchor's space.
                if let world = tablePoint(at: point) {
                    let local = placement.convert(position: world, from: nil)
                    dragTarget = SIMD2(local.x, local.z)
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
                dragTarget = nil
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
        guard model.isPlaced, !model.isRolling, !held, glide == nil else { return }
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
        guard model.isPlaced, !model.isRolling, !held, glide == nil else { return }
        pivot.orientation = simd_quatf(angle: radians, axis: [0, 1, 0]) * pivot.orientation
        restTransform = pivot.transform
    }

    // MARK: Throwing

    /// Throws the die across the table. `screenDirection` is in view points
    /// (right, down); it's mapped onto the table as seen from the camera.
    func throwDie(screenDirection: CGVector, flickSpeed: CGFloat) {
        guard model.isPlaced, !model.isRolling, !held, glide == nil, let rig, rig.model.isThrowable, let placement else { return }
        if model.explode > 0 { model.explode = 0 }
        dragTarget = nil

        // Camera right and forward, flattened onto the table, in the anchor's space.
        let camera = cameraTransform.matrix
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
        buildCorral(around: w.base.translation)
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
            removeCorral()
            model.rollDidSettle(faceUp: DieFace.faceUp(orientation: orientation))
        }
    }

    /// Puts the die back on its placed spot.
    func reset() {
        windup = nil
        glide = nil
        held = false
        model.isHeld = false
        hideMenuPanel()
        dragTarget = nil
        removeCorral()
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
        // No firmware, no menu: a held die goes back down.
        if held { putDown() }
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
        // Slid or turned on the table, a finger's jitter mustn't read as a
        // shake; a throw reads everything.
        let cap: Float? = model.isRolling ? nil : DiePhysics.handlingMaxLinearMg
        for tickPose in ticks.advance(to: time, pose: pose) {
            seq = firmware.tick(imu.reading(at: tickPose, maxLinearMg: cap), touchMask: touchMask)
        }
        if seq != frameSeq {
            frameSeq = seq
            redraw()
        }
        while let effect = firmware.nextHaptic() { play(haptic: effect) }
        let mode = firmware.mode
        if model.firmwareMode != mode { model.firmwareMode = mode }

        // The menu opened: pick the die up with its front toward you. Closed:
        // set it back down.
        let front = firmware.menuFront
        if let front, !held, !model.isRolling, glide == nil {
            pickUp(showing: front)
        } else if front == nil, held, glide == nil {
            putDown()
        }
    }

    // MARK: Held and turned

    private var halfHeight: Float { (rig?.model.bounds.y ?? 0) / 2 * pivot.scale.y }

    /// The die's centre, in the anchor's space.
    private var centre: SIMD3<Float> {
        pivot.position + pivot.orientation.act(SIMD3(0, halfHeight, 0))
    }

    private func setPose(centre c: SIMD3<Float>, rotation: simd_quatf) {
        pivot.orientation = rotation
        pivot.position = c - rotation.act(SIMD3(0, halfHeight, 0))
    }

    private func glide(to c: SIMD3<Float>, rotation: simd_quatf, duration: TimeInterval, easeOut: Bool) {
        glide = Glide(start: time, duration: duration, from: (centre, pivot.orientation), to: (c, rotation), easeOut: easeOut)
    }

    private func stepGlide() {
        guard let g = glide else { return }
        let u = Float(min((time - g.start) / g.duration, 1))
        let e = g.easeOut ? 1 - (1 - u) * (1 - u) * (1 - u) : u * u * (3 - 2 * u)
        setPose(
            centre: simd_mix(g.from.centre, g.to.centre, SIMD3(repeating: e)),
            rotation: simd_slerp(g.from.rotation, g.to.rotation, e))
        if u >= 1 {
            glide = nil
            if !held { restTransform = pivot.transform }
        }
    }

    private func stepDrag(dt: Float) {
        guard let target = dragTarget else { return }
        let c = centre
        let k = 1 - exp(-DiePhysics.dragFollowRate * dt)
        let step = (target - SIMD2(c.x, c.z)) * k
        pivot.position += SIMD3(step.x, 0, step.y)
        if simd_length(target - SIMD2(c.x, c.z)) < 0.0005, panMode == nil {
            dragTarget = nil
            restTransform = pivot.transform
        }
    }

    private var cameraInAnchor: (position: SIMD3<Float>, right: SIMD3<Float>)? {
        guard let placement else { return nil }
        let camera = cameraTransform.matrix
        let right = SIMD3(camera.columns.0.x, camera.columns.0.y, camera.columns.0.z)
        return (placement.convert(position: cameraTransform.translation, from: nil),
                placement.convert(direction: right, from: nil))
    }

    /// Lifts the die off the table, the menu's face toward the camera.
    private func pickUp(showing front: DieFace) {
        guard let cam = cameraInAnchor else { return }
        held = true
        model.isHeld = true
        dragTarget = nil
        let c = centre
        let target = SIMD3<Float>(c.x, DiePhysics.heldHeight + halfHeight * 1.5, c.z)
        let rotation = DiePhysics.heldOrientation(front: front, current: pivot.orientation, toCamera: cam.position - target)
        glide(to: target, rotation: rotation, duration: DiePhysics.heldMoveTime, easeOut: false)
        showMenuPanel(dieCentre: target, dieRotation: rotation, front: front)
    }

    /// Stands the glass turn pad in the plane of the held die's menu screen,
    /// beside it on the screen's right: as if the screen carried on past the
    /// die's edge. It stays put after.
    private func showMenuPanel(dieCentre: SIMD3<Float>, dieRotation: simd_quatf, front: DieFace) {
        guard let placement, let rig else { return }
        menuPanel?.hide()
        let side = max(rig.model.bounds.x, rig.model.bounds.z) * pivot.scale.x
        let panel = MenuPanel(side: side)
        let rotation = DiePhysics.menuPanelRotation(dieRotation: dieRotation, front: front)
        let normal = rotation.act([0, 0, 1])
        let right = rotation.act([1, 0, 0])
        let position = dieCentre
            + normal * (side / 2 + DiePhysics.menuPanelLift)
            + right * (side / 2 + DiePhysics.menuPanelGap + panel.width / 2)
        panel.show(at: position, rotation: rotation, in: placement)
        menuPanel = panel
    }

    private func hideMenuPanel() {
        menuPanel?.hide()
        menuPanel = nil
    }

    /// Sets the die back down where it was, flat on its lowest face.
    private func putDown() {
        held = false
        model.isHeld = false
        hideMenuPanel()
        let rotation = DiePhysics.setDownOrientation(current: pivot.orientation)
        // Straight down over the spot it was lifted from; resting on a face,
        // the centre is half a side up.
        let c = centre
        let target = SIMD3<Float>(c.x, halfHeight, c.z)
        glide(to: target, rotation: rotation, duration: DiePhysics.heldMoveTime, easeOut: false)
    }

    /// A quarter turn from the turn pad, about the die's centre: `direction`
    /// is +1 or −1 (radians' sign) about the viewer's axis. On the table and
    /// in the hand alike, it turns about the die's own axis nearest that one,
    /// so it ends square.
    func tip(_ axis: TurnAxis, _ direction: Int) {
        guard model.isPlaced, !model.isRolling, glide == nil, let cam = cameraInAnchor else { return }
        dragTarget = nil
        let c = centre
        var toward = cam.position - c
        toward.y = 0
        toward = simd_length(toward) < 1e-5 ? [0, 0, 1] : simd_normalize(toward)
        var right = cam.right
        right.y = 0
        right = simd_length(right) < 1e-5 ? [1, 0, 0] : simd_normalize(right)
        let wanted: SIMD3<Float> = switch axis {
        case .pitch: right
        case .yaw: [0, 1, 0]
        case .roll: toward
        }
        let pivotAxis = DiePhysics.nearestDieAxis(to: wanted, orientation: pivot.orientation)
        let rotation = simd_quatf(angle: Float(direction) * .pi / 2, axis: pivotAxis) * pivot.orientation
        glide(to: c, rotation: rotation, duration: DiePhysics.tipTime, easeOut: true)
    }

    // MARK: Corral

    private func buildCorral(around spot: SIMD3<Float>) {
        removeCorral()
        guard let radius = DiePhysics.corralRadius, let placement else { return }
        let walls = Entity()
        walls.name = "Corral"
        let sides = 8
        let width = 2 * radius * tan(.pi / Float(sides)) * 1.1
        let shape = ShapeResource.generateBox(size: [width, DiePhysics.corralHeight, 0.01])
        let material = PhysicsMaterialResource.generate(staticFriction: 0.3, dynamicFriction: 0.3, restitution: DiePhysics.tableRestitution)
        for i in 0..<sides {
            let angle = Float(i) * 2 * .pi / Float(sides)
            let outward = SIMD3<Float>(sin(angle), 0, cos(angle))
            let wall = Entity()
            wall.components.set(CollisionComponent(shapes: [shape]))
            wall.components.set(PhysicsBodyComponent(shapes: [shape], mass: 0, material: material, mode: .static))
            wall.position = SIMD3(spot.x, DiePhysics.corralHeight / 2, spot.z) + outward * (radius + 0.005)
            wall.orientation = simd_quatf(angle: angle, axis: [0, 1, 0])
            walls.addChild(wall)
        }
        placement.addChild(walls)
        corral = walls
    }

    private func removeCorral() {
        corral?.removeFromParent()
        corral = nil
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
        if let point, menuPanel?.contains(point, in: arView) != true, firmware != nil, model.isPlaced, !model.isRolling, !model.isXray,
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
