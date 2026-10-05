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
///     ├── pivot   position on the table, yaw, user scale; the physics body
///     │   └── rig.root   the loaded model (its own metres, origin at the bottom centre)
///     └── lid     with the lid off: the lid cut from the die (`DieLid`), on its own
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
    /// With AR off the die stays in the middle of the table: dragging turns
    /// it rather than moving it, and a roll is tossed straight up so it lands
    /// where it was.
    private var lockedInPlace: Bool { !augmented }
    /// What the studio camera frames: a box around the die (and its turn pad,
    /// with the menu open), as half its width and height across the view.
    /// Reframed when the view changes shape.
    private var studioFraming: (centre: SIMD3<Float>, half: SIMD2<Float>)?
    private var studioAspect: CGFloat = 0

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
    /// The lid, while it's off: cut from the die and moved on its own.
    private var lid: DieLid?
    /// Turning the die lid-up, to take the lid off once it's there.
    private var lidOpening = false
    /// One stage of the lid coming off or going back on: the lid and the
    /// four screws, each from one pose to another (in the anchor's space).
    private struct LidStep {
        let duration: TimeInterval
        /// The lid's move, if it moves: swung over about `swing` (coming
        /// off), or carried back in an arc (nil, going on).
        var lid: (from: Transform, to: Transform, swing: SIMD3<Float>?)?
        var screws: [(from: Transform, to: Transform)] = []
        /// Screwing in (+) or out (−): turns about `screwAxis` as they go.
        var turns: Float = 0
        var screwAxis: SIMD3<Float> = [0, 1, 0]
        /// When this step ends the lid is on and the die whole again.
        var finishesClosing = false
        /// When this step ends the screws are on the table, loose.
        var loosensScrews = false
    }
    /// What's left of the lid coming off or going on; the first is under way.
    private var lidSteps: [LidStep] = []
    private var lidStepStart: TimeInterval = 0
    /// The screws are loose on the table (dynamic bodies), to be knocked about.
    private var screwsLoose = false
    /// Where the loose screws were laid, to go back to if one falls off the table.
    private var screwHomes: [Transform] = []
    /// A die's x-ray, loaded for its insides: the die's own model has none,
    /// so the x-ray's are shown while the lid is off.
    private var borrowedInternals: (id: String, entity: Entity)?
    private var internalsLoading = false
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
    /// The drag started on the die (a flick from there throws it).
    private var panOnDie = false
    /// The drag or twist started on the lid (with it off): it turns or moves the lid.
    private var panOnLid = false
    /// The drag started on a loose screw: a flick sends it across the table.
    private var panScrew: Entity?
    private var twistOnLid = false
    /// Where the finger took hold of the lid, from its origin, on the table (AR).
    private var lidGrab: SIMD2<Float>?
    private var lastPanY: CGFloat = 0
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
        if let framing = studioFraming, arView.bounds.height > 0,
           abs(arView.bounds.width / arView.bounds.height - studioAspect) > 0.01 {
            frameStudio(centre: framing.centre, half: framing.half, animated: false)
        }
        if windup != nil { stepWindup() } else if model.isRolling { trackRoll(dt: dt) }
        stepGlide()
        if lidOpening, glide == nil, !internalsLoading {
            lidOpening = false
            beginLidOff()
        }
        stepLid()
        if screwsLoose { recoverLostScrews() }
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
        // With the lid off, the new model's lid comes off too, where the old one lay.
        let lidAt = lidSteps.isEmpty ? lid.map { lidPose($0) } : nil
        closeLidNow()
        rig?.root.removeFromParent()
        pivot.addChild(entity)
        let rig = DieRig(model: next, root: entity, reference: pivot)
        self.rig = rig
        rig.setExplode(model.explode)
        rig.setShellFaded(model.shellFaded)
        updateBody()
        model.rigDidLoad(rig, measured: rig.measuredSize())
        syncFirmware()
        if let lidAt, detachLid(at: lidAt) {
            model.lidDidChange(off: true)
            loadInternals()
        } else if lockedInPlace, !held {
            frameDieOnTable(animated: false)
        }
    }

    func setExplode(_ e: Float) {
        guard lid == nil else { return }
        rig?.setExplode(e)
    }

    func setShellFaded(_ faded: Bool) {
        keepingLid { rig?.setShellFaded(faded) }
        // The screens square up in x-ray, and round again after.
        syncFirmware()
    }

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
        frameDieOnTable(animated: false)
    }

    /// The direction the studio camera looks from (SIM_SPEC A5).
    private static let studioViewDirection = simd_normalize(SIMD3<Float>(0.55, 0.62, 1))

    /// Frames the die on the table, with a little room around it.
    private func frameDieOnTable(animated: Bool) {
        guard lockedInPlace, let rig else {
            if lockedInPlace { frameStudio(centre: [0, 0.015, 0], half: [0.04, 0.04], animated: animated) }
            return
        }
        let side = max(rig.model.bounds.x, rig.model.bounds.z) * pivot.scale.x
        let half = side * DiePhysics.studioDieFraming
        frameStudio(centre: homeCentre, half: [half, half], animated: animated)
    }

    /// Moves the studio camera along its fixed viewing direction so a box
    /// (half width and height, across the view) fills it, whatever its shape.
    private func frameStudio(centre: SIMD3<Float>, half: SIMD2<Float>, animated: Bool) {
        guard let camera = studioCamera else { return }
        studioFraming = (centre, half)
        let size = arView.bounds.size
        let aspect = size.height > 0 ? Float(size.width / size.height) : 1
        studioAspect = size.height > 0 ? size.width / size.height : 0
        let halfV = DiePhysics.studioFieldOfView * .pi / 360
        let halfH = atan(tan(halfV) * aspect)
        let distance = max(half.x / tan(halfH), half.y / tan(halfV)) * DiePhysics.studioFramingMargin

        // Looking along −Z at the centre, Y up.
        let z = Self.studioViewDirection
        let x = simd_normalize(simd_cross([0, 1, 0], z))
        let y = simd_cross(z, x)
        let target = Transform(rotation: simd_quatf(simd_float3x3(columns: (x, y, z))), translation: centre + z * distance)
        if animated {
            camera.move(to: target, relativeTo: nil, duration: DiePhysics.studioReframeTime, timingFunction: .easeInOut)
        } else {
            camera.transform = target
        }
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
        // Once placed, a tap on the die is a touch for the firmware (TouchTracker sends it).
        if !model.isPlaced { _ = place(at: point) }
    }

    @objc private func didPan(_ g: UIPanGestureRecognizer) {
        guard model.isPlaced, !model.isRolling, !held, glide == nil, !lidBusy, let rig, let placement else { return }
        let point = g.location(in: arView)
        switch g.state {
        case .began:
            panScrew = screw(near: point)
            if panScrew != nil {
                panOnLid = false
                panOnDie = false
                panMode = nil
                return
            }
            let ray = arView.ray(through: point)
            panOnLid = ray.map { lidIsNearest(origin: $0.origin, direction: $0.direction) } ?? false
            let onDie = !panOnLid && (ray.map { rig.contains(origin: $0.origin, direction: $0.direction) } ?? false)
            panOnDie = onDie
            panMode = onDie && !lockedInPlace ? .move : .turn
            lastPanX = g.translation(in: arView).x
            lastPanY = g.translation(in: arView).y
            lidGrab = nil
            if panOnLid, !lockedInPlace, let lid, let world = tablePoint(at: point) {
                let local = placement.convert(position: world, from: nil)
                lidGrab = SIMD2(lid.root.position.x - local.x, lid.root.position.z - local.z)
            }
        case .changed where panScrew != nil:
            break
        case .ended where panScrew != nil:
            let v = g.velocity(in: arView)
            if let screw = panScrew { flick(screw, screenVelocity: CGVector(dx: v.x, dy: v.y)) }
            panScrew = nil
        case .changed where panOnLid:
            if let grab = lidGrab, let lid, let world = tablePoint(at: point) {
                // AR: slide the lid along the table.
                let local = placement.convert(position: world, from: nil)
                lid.root.position.x = local.x + grab.x
                lid.root.position.z = local.z + grab.y
            } else {
                // Studio: sideways turns it about the vertical, up and down tips it toward you.
                let t = g.translation(in: arView)
                turnLid(yaw: Float(t.x - lastPanX) * 0.01, pitch: Float(t.y - lastPanY) * 0.01)
                lastPanX = t.x
                lastPanY = t.y
            }
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
            if panOnDie, lid == nil, speed > DiePhysics.flickThreshold, rig.model.isThrowable {
                dragTarget = nil
                throwDie(screenDirection: CGVector(dx: v.x, dy: v.y), flickSpeed: speed)
            } else {
                restTransform = pivot.transform
            }
            panMode = nil
            panOnLid = false
        default:
            panMode = nil
            panOnLid = false
            panScrew = nil
        }
    }

    @objc private func didTwist(_ g: UIRotationGestureRecognizer) {
        guard model.isPlaced, !model.isRolling, !held, glide == nil, !lidBusy else { return }
        if g.state == .began {
            twistOnLid = arView.ray(through: g.location(in: arView)).map { lidIsNearest(origin: $0.origin, direction: $0.direction) } ?? false
        }
        if twistOnLid {
            turnLid(yaw: -Float(g.rotation), pitch: 0)
            g.rotation = 0
            return
        }
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
        guard model.isPlaced, !model.isRolling, !held, glide == nil, !lidBusy else { return }
        // About the die's centre, not the model's origin (its lid's centre),
        // which is off to one side whenever the die isn't lid-down.
        setPose(centre: centre, rotation: simd_quatf(angle: radians, axis: [0, 1, 0]) * pivot.orientation)
        restTransform = pivot.transform
    }

    // MARK: Throwing

    /// Throws the die across the table. `screenDirection` is in view points
    /// (right, down); it's mapped onto the table as seen from the camera.
    func throwDie(screenDirection: CGVector, flickSpeed: CGFloat) {
        guard model.isPlaced, !model.isRolling, !held, glide == nil, lid == nil, !lidOpening,
              let rig, rig.model.isThrowable, let placement else { return }
        if model.explode > 0 { model.explode = 0 }
        dragTarget = nil

        let direction = tableDirection(screenDirection, in: placement)
        let speed = DiePhysics.throwSpeed(forFlick: flickSpeed)
        let rollAxis = simd_normalize(simd_cross([0, 1, 0], direction))
        let spin = Float.random(in: DiePhysics.throwSpin)
        let wobble = SIMD3<Float>(Float.random(in: -0.3...0.3), Float.random(in: -0.5...0.5), Float.random(in: -0.3...0.3))

        restTransform = pivot.transform
        // Locked in place: straight up, tumbling, to land where it was.
        let velocity = lockedInPlace
            ? SIMD3<Float>(0, DiePhysics.lockedTossLift, 0)
            : direction * speed + [0, DiePhysics.throwLift, 0]
        windup = Windup(start: time, base: pivot.transform, velocity: velocity, spin: (rollAxis + wobble) * spin)
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
        let (offset, rock) = DiePhysics.windup(at: t, lift: lockedInPlace ? DiePhysics.lockedWindupLift : DiePhysics.windupLift)
        // Lifted, shaken and rocked about the die's centre.
        setPose(centre: dieCentre(of: w.base) + offset, rotation: rock * w.base.rotation)
    }

    private func release(_ w: Windup) {
        windup = nil
        if lockedInPlace, let rig {
            // A tight ring round the die's home: room to tumble, not to wander.
            let side = max(rig.model.bounds.x, rig.model.bounds.z) * pivot.scale.x
            buildCorral(around: homeCentre, radius: side * DiePhysics.lockedCorralFactor)
        } else if let radius = DiePhysics.corralRadius {
            buildCorral(around: dieCentre(of: w.base), radius: radius)
        }
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
            if lockedInPlace { slideBackHome() }
        }
    }

    /// Puts the die back on its placed spot.
    func reset() {
        windup = nil
        glide = nil
        closeLidNow()
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

    /// A direction on screen (right, down), as seen on the table from the
    /// camera, in the anchor's space.
    private func tableDirection(_ screen: CGVector, in placement: Entity) -> SIMD3<Float> {
        // Camera right and forward, flattened onto the table.
        let camera = cameraTransform.matrix
        let right = flatten(placement.convert(direction: SIMD3(camera.columns.0.x, camera.columns.0.y, camera.columns.0.z), from: nil))
        let forward = flatten(placement.convert(direction: -SIMD3(camera.columns.2.x, camera.columns.2.y, camera.columns.2.z), from: nil))
        let direction = right * Float(screen.dx) - forward * Float(screen.dy)
        return simd_length(direction) < 1e-4 ? forward : simd_normalize(direction)
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
        // The screens are rebuilt on the whole die, then the lid comes off again.
        keepingLid { syncFirmwareNow() }
    }

    private func syncFirmwareNow() {
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
        // The app's link follows whichever die is running.
        model.phoneLink?.attach(firmware)
        screens?.remove()
        screens = nil
        if let firmware, let rig {
            let live = LiveScreens(rig: rig, reference: pivot, side: firmware.panelSide, rounded: !model.isXray)
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
        model.phoneLink?.attach(nil)
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
        model.phoneLink?.pump()
        let mode = firmware.mode
        if model.firmwareMode != mode { model.firmwareMode = mode }

        // The menu opened: pick the die up with its front toward you. Closed:
        // set it back down.
        let front = firmware.menuFront
        if let front, !held, !model.isRolling, glide == nil, lid == nil, !lidOpening {
            pickUp(showing: front)
        } else if front == nil, held, glide == nil {
            putDown()
        }
    }

    // MARK: Held and turned

    private var halfHeight: Float { (rig?.model.bounds.y ?? 0) / 2 * pivot.scale.y }

    /// Where a pose puts the die's centre, in the anchor's space.
    private func dieCentre(of t: Transform) -> SIMD3<Float> {
        t.translation + t.rotation.act(SIMD3(0, halfHeight, 0))
    }

    /// AR off: the die's home, in the middle of the table, resting on a face.
    /// Fixed, so nothing a roll leaves behind can shift it.
    private var homeCentre: SIMD3<Float> { SIMD3(0, halfHeight, 0) }

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
        let lift = lockedInPlace ? DiePhysics.studioHeldHeight : DiePhysics.heldHeight
        let target = SIMD3<Float>(c.x, lift + halfHeight * 1.5, c.z)
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

        // In the studio, pull the camera back to take in the die and the pad:
        // from the die's far edge to the pad's.
        if lockedInPlace {
            let left = dieCentre - right * (side / 2)
            let farEdge = position + right * (panel.width / 2)
            let half = SIMD2<Float>(simd_distance(left, farEdge) / 2, max(panel.height, side) / 2)
            frameStudio(centre: (left + farEdge) / 2, half: half, animated: true)
        }
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
        frameDieOnTable(animated: true)
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
        guard model.isPlaced, !model.isRolling, glide == nil, !lidBusy, let cam = cameraInAnchor else { return }
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

    /// Locked in place: after a toss, ease the die back over its home, gently
    /// enough that the firmware still reads it as at rest. It keeps the face
    /// it landed on.
    private func slideBackHome() {
        let c = centre
        let target = SIMD3<Float>(homeCentre.x, c.y, homeCentre.z)
        let distance = simd_length(target - c)
        guard distance > 0.0005 else { return }
        // Smoothstep peaks at 6·d/T²; keep that under the limit.
        let duration = max(0.3, TimeInterval(sqrt(6 * distance / DiePhysics.lockedReturnAcceleration)))
        glide(to: target, rotation: pivot.orientation, duration: duration, easeOut: false)
    }

    // MARK: Corral

    private func buildCorral(around spot: SIMD3<Float>, radius: Float) {
        removeCorral()
        guard let placement else { return }
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
    /// (or turns into a drag, a pinch or a twist), in x-ray too. With the lid
    /// off, a finger on the lid's screen touches −Y; the cup's open end has none.
    private func touchChanged(at point: CGPoint?) {
        var mask: UInt8 = 0
        if let point, menuPanel?.contains(point, in: arView) != true, firmware != nil, model.isPlaced, !model.isRolling,
           !lidBusy, let ray = arView.ray(through: point), let face = touchedFace(origin: ray.origin, direction: ray.direction) {
            mask = 1 << UInt8(face.index)
        }
        touchMask = mask
    }

    /// The face a world-space ray first touches, on the die or the lid.
    private func touchedFace(origin: SIMD3<Float>, direction: SIMD3<Float>) -> DieFace? {
        guard let rig else { return nil }
        let size = rig.model.bounds
        let local = PartPicker.ray(origin: origin, direction: direction, into: pivot.transformMatrix(relativeTo: nil))
        let boxMin = SIMD3<Float>(-size.x / 2, 0, -size.z / 2), boxMax = SIMD3<Float>(size.x / 2, size.y, size.z / 2)
        var cup = PartPicker.hitDistance(origin: local.origin, direction: local.direction, boxMin: boxMin, boxMax: boxMax)
        var face = PartPicker.entryFace(origin: local.origin, direction: local.direction, boxMin: boxMin, boxMax: boxMax)
        guard let lid else { return face }
        if face == .ny {
            face = nil
            cup = nil
        }
        if let hit = lidHit(lid, origin: origin, direction: direction), hit.distance < cup ?? .infinity {
            return hit.face == .ny ? .ny : nil
        }
        return face
    }

    // MARK: The lid

    /// The lid is on its way off or back on: hands off until it's there.
    private var lidBusy: Bool { lidOpening || !lidSteps.isEmpty }

    /// Takes the lid off (the die turns lid-up, the lid lifts off and lies
    /// inside-up beside it) or puts it back on.
    func setLidOff(_ off: Bool) {
        guard model.isPlaced, !model.isRolling, !held, windup == nil, glide == nil, !lidBusy else { return }
        if off {
            guard lid == nil else { return }
            dragTarget = nil
            lidOpening = true
            loadInternals()
            model.lidDidChange(off: true)
            let c = centre
            glide(to: SIMD3(c.x, halfHeight, c.z), rotation: Self.lidUp(pivot.orientation),
                  duration: DiePhysics.heldMoveTime, easeOut: false)
        } else if let lid {
            // The lid back on, then the screws back over their holes and in,
            // from wherever they were knocked to.
            pinScrews()
            let laid = lid.screws.map(\.transform)
            let seated = seatedScrews(lid)
            let out = pivot.orientation.act([0, -1, 0])
            let backedOut = seated.map { Self.moved($0, by: out * screwTravel(lid)) }
            lidSteps = [
                LidStep(duration: DiePhysics.lidMoveTime, lid: (from: lid.root.transform, to: attachedLidTransform, swing: nil)),
                LidStep(duration: DiePhysics.screwLayTime, screws: zip(laid, backedOut).map { (from: $0, to: $1) }),
                LidStep(duration: DiePhysics.screwTime, screws: zip(backedOut, seated).map { (from: $0, to: $1) },
                        turns: DiePhysics.screwTurns, screwAxis: out, finishesClosing: true),
            ]
            lidStepStart = time
        }
    }

    private static func moved(_ t: Transform, by offset: SIMD3<Float>) -> Transform {
        Transform(scale: t.scale, rotation: t.rotation, translation: t.translation + offset)
    }

    /// Where each screw sits in its hole on the die as it is now, in the anchor's space.
    private func seatedScrews(_ lid: DieLid) -> [Transform] {
        lid.screwSeats.map { pivot.transform.matrix * Transform(translation: $0).matrix }.map(Transform.init(matrix:))
    }

    /// How far a screw comes out: its length and a millimetre, in the anchor's space.
    private func screwTravel(_ lid: DieLid) -> Float {
        let length = lid.screws.map { $0.visualBounds(relativeTo: $0).max.y }.max() ?? 0
        return (max(length, 0) + 0.001) * pivot.scale.x
    }

    /// The lid and its screws where they lie, to put back after the die is rebuilt.
    private func lidPose(_ lid: DieLid) -> (lid: Transform, screws: [Transform]) {
        (lid.root.transform, lid.screws.map(\.transform))
    }

    /// Lid up, keeping the die's heading.
    private static func lidUp(_ q: simd_quatf) -> simd_quatf {
        var heading = q.act([1, 0, 0])
        heading.y = 0
        if simd_length(heading) < 1e-3 {
            heading = q.act([0, 0, 1])
            heading.y = 0
        }
        heading = simd_length(heading) < 1e-5 ? [1, 0, 0] : simd_normalize(heading)
        return DiePhysics.rotation(from: DieFace.ny.normal, [1, 0, 0], to: [0, 1, 0], heading)
    }

    /// Where the lid sits on the die, in the anchor's space.
    private var attachedLidTransform: Transform { pivot.transform }

    /// The die is lid-up: the screws come out and lie in a row on the table,
    /// then the lid lifts off and swings over beside the cup.
    private func beginLidOff() {
        guard detachLid(at: nil), let lid, let rig, let cam = cameraInAnchor else {
            model.lidDidChange(off: false)
            return
        }
        let from = lid.root.transform
        var side = cam.right
        side.y = 0
        side = simd_length(side) < 1e-5 ? [1, 0, 0] : simd_normalize(side)
        // Tipping its top toward `side` turns it over, inside up.
        let axis = simd_normalize(simd_cross([0, 1, 0], side))
        let s = max(rig.model.bounds.x, rig.model.bounds.z) * pivot.scale.x
        let c = centre
        let rest = SIMD3<Float>(c.x, 0, c.z) + side * s * DiePhysics.lidOffDistance
        let to = Transform(scale: from.scale, rotation: simd_quatf(angle: .pi, axis: axis) * from.rotation, translation: rest)

        // The screws: straight out of their holes, turning, then laid in a
        // row on the table in front of the die, heads toward you.
        var toward = cam.position - c
        toward.y = 0
        toward = simd_length(toward) < 1e-5 ? [0, 0, 1] : simd_normalize(toward)
        let up = SIMD3<Float>(0, 1, 0)
        let seated = lid.screws.map(\.transform)
        let out = pivot.orientation.act([0, -1, 0])
        let backedOut = seated.map { Self.moved($0, by: out * screwTravel(lid)) }
        let lying = DiePhysics.rotation(from: [0, 1, 0], [1, 0, 0], to: -toward, up)
        // In a row in front of the die, along the view's right; a screw's
        // head radius off the table. (Step by step: as one expression it's
        // too much for the type checker.)
        let rowStart: SIMD3<Float> = SIMD3<Float>(c.x, 0, c.z) + toward * (s * DiePhysics.screwRowDistance)
        let rowStep: SIMD3<Float> = side * (s * DiePhysics.screwSpacing)
        var laid: [Transform] = []
        for (i, screw) in lid.screws.enumerated() {
            let head: Float = screw.visualBounds(relativeTo: screw).extents.x / 2 * pivot.scale.x
            let along: Float = Float(i) - 1.5
            let spot: SIMD3<Float> = rowStart + rowStep * along + up * head
            laid.append(Transform(scale: seated[i].scale, rotation: lying, translation: spot))
        }
        lidSteps = [
            LidStep(duration: DiePhysics.screwTime, screws: zip(seated, backedOut).map { (from: $0, to: $1) },
                    turns: -DiePhysics.screwTurns, screwAxis: out),
            LidStep(duration: DiePhysics.screwLayTime, screws: zip(backedOut, laid).map { (from: $0, to: $1) },
                    loosensScrews: true),
            LidStep(duration: DiePhysics.lidMoveTime, lid: (from: from, to: to, swing: axis)),
        ]
        lidStepStart = time
        if lockedInPlace {
            // Frame the cup, the lid and the screws together.
            let mid = (SIMD3(c.x, halfHeight, c.z) + rest) / 2 + toward * s * 0.2
            frameStudio(centre: mid, half: [simd_distance(c, rest) / 2 + s * 0.6, s * 0.85], animated: true)
        }
    }

    /// One frame of the lid coming off or going back on.
    private func stepLid() {
        guard let step = lidSteps.first, let lid, let rig else { return }
        let u = Float(min((time - lidStepStart) / step.duration, 1))
        let s = max(rig.model.bounds.x, rig.model.bounds.z) * pivot.scale.x
        let up = SIMD3<Float>(0, 1, 0)
        let smooth = { (x: Float) -> Float in x * x * (3 - 2 * x) }
        if let move = step.lid {
            var position: SIMD3<Float>
            let rotation: simd_quatf
            if let axis = move.swing {
                // Straight off the die first, then over and down beside the cup.
                let pull = smooth(min(u / 0.3, 1))
                let swing = smooth(max(0, (u - 0.2) / 0.8))
                position = simd_mix(move.from.translation, move.to.translation, SIMD3(repeating: swing))
                position += up * (s * DiePhysics.lidPull * pull * (1 - swing) + s * 0.5 * sin(.pi * swing))
                rotation = simd_quatf(angle: .pi * swing, axis: axis) * move.from.rotation
            } else {
                let e = smooth(u)
                position = simd_mix(move.from.translation, move.to.translation, SIMD3(repeating: e))
                position += up * s * 0.5 * sin(.pi * e)
                rotation = simd_slerp(move.from.rotation, move.to.rotation, e)
            }
            lid.root.transform = Transform(scale: move.from.scale, rotation: rotation, translation: position)
        }
        let e = smooth(u)
        for (screw, leg) in zip(lid.screws, step.screws) {
            var position = simd_mix(leg.from.translation, leg.to.translation, SIMD3(repeating: e))
            let rotation: simd_quatf
            if step.turns != 0 {
                // Turning in or out of its hole, along its own axis.
                rotation = simd_quatf(angle: step.turns * 2 * .pi * e, axis: step.screwAxis) * leg.from.rotation
            } else {
                // Carried between the die and the table.
                position += up * s * 0.25 * sin(.pi * e)
                rotation = simd_slerp(leg.from.rotation, leg.to.rotation, e)
            }
            screw.transform = Transform(scale: leg.from.scale, rotation: rotation, translation: position)
        }
        guard u >= 1 else { return }
        lidSteps.removeFirst()
        lidStepStart = time
        if step.loosensScrews { loosenScrews() }
        if step.finishesClosing { finishLidOn() }
    }

    /// The lid is back on the lid-up die: whole again, turned back over onto its lid.
    private func finishLidOn() {
        closeLidNow()
        let c = centre
        let over = simd_quatf(angle: .pi, axis: pivot.orientation.act([1, 0, 0])) * pivot.orientation
        glide(to: SIMD3(c.x, halfHeight, c.z), rotation: over, duration: DiePhysics.heldMoveTime, easeOut: false)
        frameDieOnTable(animated: true)
    }

    /// Cuts the lid off the die where it sits, or puts it and its screws
    /// where they lay (`pose`, in the anchor's space).
    private func detachLid(at pose: (lid: Transform, screws: [Transform])?) -> Bool {
        guard lid == nil, let rig, let placement, model.canTakeLidOff,
              let cut = DieLid(dieRoot: rig.root, reference: pivot, side: rig.model.bounds.x,
                               internals: internalsForRig, opacity: model.isXray ? DieRig.xrayShellOpacity : 1)
        else { return false }
        placement.addChild(cut.root, preservingWorldTransform: true)
        for screw in cut.screws { placement.addChild(screw, preservingWorldTransform: true) }
        // Kinematic, so sliding or turning it knocks the screws about.
        let bounds = cut.root.visualBounds(relativeTo: cut.root)
        let shape = ShapeResource.generateBox(size: bounds.extents).offsetBy(translation: bounds.center)
        cut.root.components.set(CollisionComponent(shapes: [shape]))
        cut.root.components.set(PhysicsBodyComponent(shapes: [shape], mass: 0, mode: .kinematic))
        lid = cut
        if let pose {
            cut.root.transform = pose.lid
            for (screw, t) in zip(cut.screws, pose.screws) { screw.transform = t }
            loosenScrews()
        }
        return true
    }

    /// The x-ray's insides for the die showing, once loaded; nil for an x-ray (it has its own).
    private var internalsForRig: Entity? {
        guard let rig, rig.model.kind == .die, let borrowed = borrowedInternals, borrowed.id == rig.model.xray else { return nil }
        return borrowed.entity
    }

    /// Loads the die's x-ray for its insides, if it isn't already. With the
    /// lid already off when it arrives, the lid is cut again to take them in.
    private func loadInternals() {
        guard let rig, rig.model.kind == .die, let id = rig.model.xray, borrowedInternals?.id != id,
              let xray = model.catalog.model(id: id), model.models.contains(xray)
        else { return }
        internalsLoading = true
        Task { [weak self] in
            let entity = try? await self?.model.catalog.load(xray)
            guard let self else { return }
            self.internalsLoading = false
            guard let entity else { return }
            self.borrowedInternals = (id, entity)
            if self.lid != nil, self.lidSteps.isEmpty, self.rig?.model.xray == id { self.keepingLid {} }
        }
    }

    /// Puts the lid back on at once, wherever it was.
    private func closeLidNow() {
        lidOpening = false
        lidSteps = []
        screwsLoose = false
        lid?.restore()
        lid = nil
        if model.isLidOff { model.lidDidChange(off: false) }
    }

    /// Runs `body` on the whole die: the lid goes back on, then comes off
    /// again where it lay. Mid-move, it just goes back on.
    private func keepingLid(_ body: () -> Void) {
        guard let lid else {
            body()
            return
        }
        let at = lidSteps.isEmpty ? lidPose(lid) : nil
        lidSteps = []
        screwsLoose = false
        self.lid?.restore()
        self.lid = nil
        body()
        if let at, detachLid(at: at) { return }
        closeLidNow()
    }

    // MARK: Loose screws

    /// Lets the laid-out screws go: each a dynamic body shaped like itself,
    /// lying on the table where it was put, for the die, the lid or a
    /// finger to knock about.
    private func loosenScrews() {
        guard let lid else { return }
        let material = PhysicsMaterialResource.generate(staticFriction: DiePhysics.screwStaticFriction,
                                                        dynamicFriction: DiePhysics.screwDynamicFriction,
                                                        restitution: DiePhysics.screwRestitution)
        for screw in lid.screws {
            let shapes = lid.collisionShapes(of: screw)
            guard !shapes.isEmpty else { continue }
            screw.components.set(CollisionComponent(shapes: shapes))
            var body = PhysicsBodyComponent(shapes: shapes, mass: DiePhysics.screwMass, material: material, mode: .dynamic)
            body.isContinuousCollisionDetectionEnabled = true
            body.linearDamping = DiePhysics.screwLinearDamping
            body.angularDamping = DiePhysics.screwAngularDamping
            screw.components.set(body)
            screw.components.set(PhysicsMotionComponent())
        }
        screwHomes = lid.screws.map(\.transform)
        screwsLoose = true
    }

    /// Holds the screws still again, to be carried back to the die.
    private func pinScrews() {
        screwsLoose = false
        for screw in lid?.screws ?? [] {
            screw.components.remove(PhysicsMotionComponent.self)
            screw.components.remove(PhysicsBodyComponent.self)
            screw.components.remove(CollisionComponent.self)
        }
    }

    /// A screw knocked off the table goes back where it was laid.
    private func recoverLostScrews() {
        guard let lid else { return }
        for (screw, home) in zip(lid.screws, screwHomes) where screw.position.y < DiePhysics.lostBelow {
            screw.transform = home
            screw.components.set(PhysicsMotionComponent())
        }
    }

    /// The loose screw nearest a point in the view, if one is close enough to touch.
    private func screw(near point: CGPoint) -> Entity? {
        guard screwsLoose, let lid else { return nil }
        let near = lid.screws.compactMap { screw -> (Entity, CGFloat)? in
            guard let p = arView.project(screw.visualBounds(relativeTo: nil).center) else { return nil }
            return (screw, hypot(p.x - point.x, p.y - point.y))
        }.min { $0.1 < $1.1 }
        guard let near, near.1 <= DiePhysics.screwTouchRadius else { return nil }
        return near.0
    }

    /// Sends a loose screw across the table the way the finger flicked,
    /// tumbling about the axis it would roll on.
    private func flick(_ screw: Entity, screenVelocity v: CGVector) {
        guard screwsLoose, let placement else { return }
        let flickSpeed = hypot(v.dx, v.dy)
        guard flickSpeed > 1 else { return }
        let direction = tableDirection(v, in: placement)
        let speed = DiePhysics.screwSpeed(forFlick: flickSpeed) * pivot.scale.x
        let roll = simd_normalize(simd_cross([0, 1, 0], direction))
        screw.components.set(PhysicsMotionComponent(linearVelocity: direction * speed + [0, speed * 0.2, 0],
                                                    angularVelocity: roll * speed * 100))
        lightHaptic.impactOccurred()
    }

    /// Whether a world-space ray meets the lid before the cup.
    private func lidIsNearest(origin: SIMD3<Float>, direction: SIMD3<Float>) -> Bool {
        guard let lid, let hit = lidHit(lid, origin: origin, direction: direction) else { return false }
        guard let rig else { return true }
        let bounds = rig.root.visualBounds(relativeTo: nil)
        let cup = PartPicker.hitDistance(origin: origin, direction: direction, boxMin: bounds.min, boxMax: bounds.max)
        return hit.distance < cup ?? .infinity
    }

    /// Where a world-space ray meets the lid's own box, and through which of the die's faces.
    private func lidHit(_ lid: DieLid, origin: SIMD3<Float>, direction: SIMD3<Float>) -> (distance: Float, face: DieFace?)? {
        let bounds = lid.root.visualBounds(relativeTo: lid.root)
        let local = PartPicker.ray(origin: origin, direction: direction, into: lid.root.transformMatrix(relativeTo: nil))
        guard let t = PartPicker.hitDistance(origin: local.origin, direction: local.direction, boxMin: bounds.min, boxMax: bounds.max)
        else { return nil }
        let face = PartPicker.entryFace(origin: local.origin, direction: local.direction, boxMin: bounds.min, boxMax: bounds.max)
        return (t, face)
    }

    /// Turns the lid about its middle (yaw about the vertical, pitch about the
    /// view's right), then sets it down on the table.
    private func turnLid(yaw: Float, pitch: Float) {
        guard let lid, let placement, let cam = cameraInAnchor else { return }
        let root = lid.root
        var right = cam.right
        right.y = 0
        right = simd_length(right) < 1e-5 ? [1, 0, 0] : simd_normalize(right)
        let middle = root.visualBounds(relativeTo: placement).center
        let q = simd_quatf(angle: pitch, axis: right) * simd_quatf(angle: yaw, axis: [0, 1, 0])
        root.orientation = q * root.orientation
        root.position = middle + q.act(root.position - middle)
        root.position.y -= root.visualBounds(relativeTo: placement).min.y
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
