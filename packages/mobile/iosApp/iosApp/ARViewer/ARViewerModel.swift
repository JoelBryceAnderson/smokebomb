import ARKit
import AVFoundation
import Observation
import RealityKit

/// The AR viewer's state, shared by the iPhone overlay and the iPad side panel.
/// It outlives the AR view, so the chosen model and size lock survive a tab switch.
@MainActor
@Observable
final class ARViewerModel {
    enum Availability {
        case checking, ready, cameraDenied, unsupported
    }

    /// The one the app uses; tests make their own.
    static let shared = ARViewerModel(
        catalog: ModelCatalog(source: BundleModelSource(bundle: .main)),
        labels: .load(bundle: .main))

    let catalog: ModelCatalog
    let labels: PartLabels
    @ObservationIgnored weak var scene: ARSceneController?
    /// Makes the firmware for live screens. The app sets it (it needs the Rust
    /// library); nil means baked screens only, as in the tests.
    @ObservationIgnored var makeFirmware: DieFirmwareFactory?

    /// AR's own availability (camera access, device support). It doesn't
    /// matter with AR off.
    var availability = Availability.checking

    /// AR on: the camera and your table. Off (the default): the die locked in
    /// place on a virtual table, like the desktop simulator; it needs no camera
    /// and runs anywhere. Remembered.
    var augmented: Bool {
        didSet { UserDefaults.standard.set(augmented, forKey: Self.augmentedKey) }
    }
    /// Whether this device can do AR at all.
    let arSupported: Bool
    private static let augmentedKey = "simulator.augmented"
    let models: [SugarcubeModel]
    private(set) var selected: SugarcubeModel?
    /// The die to go back to when x-ray is switched off.
    private var dieBeforeXray: SugarcubeModel?

    var isLoading = false
    var loadError: String?
    var isCoaching = false
    var planeFound = false
    var isPlaced = false

    var isTrueSizeLocked = true {
        didSet { if isTrueSizeLocked { setScale(1) } }
    }
    private(set) var scale: Float = 1

    var explode: Float = 0 {
        didSet { scene?.setExplode(explode) }
    }
    private(set) var canExplode = false
    private(set) var shellFaded = false
    private(set) var canFadeShell = false

    /// Run the real firmware and show its panels on the die.
    var liveScreens = true {
        didSet { scene?.syncFirmware() }
    }
    /// Whether live screens are possible at all: the app supplied a firmware.
    var canRunFirmware: Bool { makeFirmware != nil }
    /// The firmware's mode while it runs (debug readout).
    var firmwareMode: String?

    var selectedPart: String?
    private(set) var isRolling = false
    /// Held in the air with the menu's face toward you (the firmware's menu is open).
    var isHeld = false
    private(set) var faceUp: DieFace?
    /// Visual bounds of the loaded model, measured in metres; the debug overlay compares them with the table.
    private(set) var measured: SIMD3<Float>?

    init(catalog: ModelCatalog, labels: PartLabels) {
        self.catalog = catalog
        self.labels = labels
        models = catalog.available
        selected = models.first { $0.id == ModelCatalog.defaultID } ?? models.first { $0.kind == .die } ?? models.first
        let supported = ARWorldTrackingConfiguration.isSupported
        let saved = UserDefaults.standard.object(forKey: Self.augmentedKey) as? Bool
        arSupported = supported
        augmented = supported && (saved ?? false)
    }

    // MARK: Availability

    func checkAvailability() async {
        guard ARWorldTrackingConfiguration.isSupported else {
            availability = .unsupported
            return
        }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            availability = .ready
        case .notDetermined:
            availability = await AVCaptureDevice.requestAccess(for: .video) ? .ready : .cameraDenied
        default:
            availability = .cameraDenied
        }
    }

    // MARK: Captions

    /// "30 mm · Rainbow · 30.0 mm", with the scale when it isn't true size.
    var caption: String {
        guard let selected else { return "No models bundled" }
        var text = "\(selected.title) · \(selected.sizeLabel)"
        if abs(scale - 1) > 0.005 { text += " · \(scalePercent)" }
        return text
    }

    var scalePercent: String { "\(Int((scale * 100).rounded()))%" }

    /// Debug: measured bounds against the size table, to ±0.1 mm.
    var boundsCheck: (text: String, ok: Bool)? {
        guard let selected, let measured else { return nil }
        let mm = measured * 1000
        let ok = all(simd_abs(measured - selected.bounds) .<= SIMD3(repeating: 0.0001))
        let size = String(format: "%.2f × %.2f × %.2f mm", mm.x, mm.y, mm.z)
        return (ok ? "Bounds \(size) ✓" : "Bounds \(size) ✗ expected \(selected.sizeLabel)", ok)
    }

    var hint: String? {
        if isCoaching { return nil }
        if augmented, !isPlaced { return planeFound ? "Tap the table to place the die" : "Move your device slowly to find the table" }
        if isHeld { return "Menu: tip it with the turn pad · touch the front face" }
        return nil
    }

    // MARK: Choosing models

    func select(_ model: SugarcubeModel) {
        guard model != selected else { return }
        if model.kind == .die || model.kind == .fixture { dieBeforeXray = nil }
        selected = model
        selectedPart = nil
        faceUp = nil
        loadError = nil
        scene?.show(model)
    }

    /// The models you pick by size and colour: the dice (and, in debug builds, the per-part test model).
    var dice: [SugarcubeModel] {
        models.filter { $0.kind == .die || $0.kind == .fixture }
    }

    /// The die behind what's showing: itself, or the one an x-ray belongs to.
    var die: SugarcubeModel? {
        guard let selected else { return nil }
        switch selected.kind {
        case .die, .fixture: return selected
        case .xray, .explodedXray: return dieBeforeXray ?? catalog.model(id: selected.die)
        case .lineup: return nil
        }
    }

    /// "30 mm", "34 mm", "40 mm": the sizes there's a die for, in catalog order.
    var sizes: [String] {
        dice.map(\.size).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
    }

    /// The colours of the chosen size.
    var colours: [SugarcubeModel] {
        dice.filter { $0.size == die?.size }
    }

    /// Another size, keeping the colour where that size has it, and keeping x-ray on.
    func chooseSize(_ size: String) {
        guard size != die?.size else { return }
        let options = dice.filter { $0.size == size }
        guard let next = options.first(where: { $0.variant == die?.variant }) ?? options.first else { return }
        choose(next)
    }

    /// Another die. In x-ray it stays in x-ray: on the same file when the dice
    /// share one, or on the new die's x-ray, or back to the die if it has none.
    func choose(_ next: SugarcubeModel) {
        guard next != die, let current = selected else { return }
        guard current.kind == .xray || current.kind == .explodedXray else {
            select(next)
            return
        }
        let currentXray = current.kind == .xray ? current.id : current.exploded
        if next.xray != nil, next.xray == currentXray {
            dieBeforeXray = next
        } else if let xray = catalog.model(id: next.xray), models.contains(xray) {
            select(xray)
            dieBeforeXray = next
        } else {
            select(next)
        }
    }

    var isXray: Bool {
        shellFaded || selected?.kind == .xray || selected?.kind == .explodedXray
    }

    /// Whether the X-ray switch does anything for this model.
    var canToggleXray: Bool {
        guard let selected else { return false }
        if canFadeShell { return true }
        switch selected.kind {
        case .die: return catalog.model(id: selected.xray).map { models.contains($0) } ?? false
        case .xray, .explodedXray: return true
        case .lineup, .fixture: return false
        }
    }

    func toggleXray() {
        guard let selected else { return }
        if canFadeShell {
            shellFaded.toggle()
            scene?.setShellFaded(shellFaded)
            return
        }
        switch selected.kind {
        case .die:
            guard let xray = catalog.model(id: selected.xray), models.contains(xray) else { return }
            let die = selected
            select(xray)
            dieBeforeXray = die
        case .xray, .explodedXray:
            if let back = dieBeforeXray ?? catalog.model(id: selected.die), models.contains(back) {
                select(back)
            }
        case .lineup, .fixture:
            break
        }
    }

    /// The pre-exploded counterpart for the 30 mm x-ray, in either direction.
    var explodedCounterpart: SugarcubeModel? {
        guard let other = catalog.model(id: selected?.exploded), models.contains(other) else { return nil }
        return other
    }

    func toggleExplodedFile() {
        guard let other = explodedCounterpart else { return }
        let back = dieBeforeXray
        select(other)
        dieBeforeXray = back
    }

    // MARK: Size

    func setScale(_ value: Float) {
        scale = isTrueSizeLocked ? 1 : min(max(value, 0.25), 10)
        scene?.setScale(scale)
    }

    func resetScale() { setScale(1) }

    // MARK: Rolling

    var canThrow: Bool {
        isPlaced && !isRolling && !isHeld && !isLoading && (selected?.isThrowable ?? false)
    }

    /// The turn pad works once the die is down and not mid-throw.
    var canTurn: Bool { isPlaced && !isRolling && !isLoading }

    /// A quarter turn about one of the viewer's axes (+1 or −1): a menu tip
    /// while the die is held, or turning it over on the table.
    func tip(_ axis: TurnAxis, _ direction: Int) {
        scene?.tip(axis, direction)
    }

    /// Throws away from the camera, for the Roll button.
    func roll() {
        scene?.throwDie(screenDirection: CGVector(dx: CGFloat.random(in: -0.3...0.3), dy: -1),
                        flickSpeed: DiePhysics.flickForFullSpeed * CGFloat.random(in: 0.35...0.7))
    }

    func resetDie() { scene?.reset() }
    func placeAgain() { scene?.unplace() }

    // MARK: Called by the scene

    func sceneDidStart() {
        isPlaced = false
        planeFound = false
        isRolling = false
        if let selected { scene?.show(selected) }
    }

    func rigDidLoad(_ rig: DieRig, measured: SIMD3<Float>) {
        self.measured = measured
        canExplode = rig.canExplode
        canFadeShell = rig.canFadeShell
        if !canExplode { explode = 0 }
        if !canFadeShell { shellFaded = false }
    }

    func loadFailed(_ error: Error) {
        loadError = error.localizedDescription
        measured = nil
    }

    func rollDidStart() {
        isRolling = true
        faceUp = nil
    }

    func rollDidSettle(faceUp: DieFace) {
        isRolling = false
        self.faceUp = faceUp
    }

    func rollDidReset() {
        isRolling = false
    }
}
