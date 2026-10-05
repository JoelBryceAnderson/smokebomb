import RealityKit
import SwiftUI

/// The Simulator tab: the die, with the real firmware, on your table through
/// the camera (AR on) or on a virtual table (AR off). On iPhone (or any compact
/// width) the controls float over the view; on iPad they sit in a side panel
/// next to it, with a control pad for turning and throwing the die.
struct ARViewerScreen: View {
    @Bindable var model: ARViewerModel
    @Environment(\.horizontalSizeClass) private var widthClass
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            if model.augmented, model.availability != .ready {
                arUnavailable
            } else {
                // One structure for both widths, so resizing in Split View keeps the session and the placed die.
                HStack(spacing: 0) {
                    stage
                        .overlay(alignment: .bottom) {
                            if widthClass != .regular { CompactControls(model: model) }
                        }
                    if widthClass == .regular {
                        Divider()
                        SidePanel(model: model)
                            .frame(width: 340)
                    }
                }
            }
        }
        // AR needs camera access; the studio doesn't.
        .task(id: model.augmented) {
            if model.augmented { await model.checkAvailability() }
        }
        .onChange(of: scenePhase) { _, phase in
            // Coming back from Settings with camera access changed.
            if phase == .active, model.augmented, model.availability == .cameraDenied {
                Task { await model.checkAvailability() }
            }
        }
    }

    @ViewBuilder private var arUnavailable: some View {
        let withoutAR = ("Use without AR", { model.augmented = false })
        switch model.availability {
        case .checking, .ready:
            ProgressView()
        case .unsupported:
            Unavailable(
                title: "AR isn't available here",
                message: "AR needs a device with an A12 chip or later and a rear camera. The die can still run on a virtual table.",
                action: withoutAR)
        case .cameraDenied:
            Unavailable(
                title: "Camera access is off",
                message: "AR shows the die on your table through the camera. Turn on camera access for Sugarcube in Settings, or use the die on a virtual table.",
                action: ("Open Settings", {
                    if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                }),
                secondary: withoutAR)
        }
    }

    private var stage: some View {
        ZStack(alignment: .top) {
            // A new view, session and scene when AR is switched on or off.
            ARViewContainer(model: model, augmented: model.augmented)
                .id(model.augmented)
                .ignoresSafeArea(edges: .top)
            StatusOverlay(model: model)
                .padding(.top, 8)
        }
        .overlay(alignment: .leading) {
            // On iPhone the AR toggle sits with X-ray in the controls.
            if model.arSupported, widthClass == .regular {
                ARPane(model: model)
                    .padding(.leading, 12)
            }
        }
    }
}

// MARK: - AR view

private struct ARViewContainer: UIViewRepresentable {
    let model: ARViewerModel
    let augmented: Bool

    final class Coordinator {
        var scene: ARSceneController?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> ARView {
        let scene = ARSceneController(model: model, augmented: augmented)
        context.coordinator.scene = scene
        model.scene = scene
        scene.start()
        return scene.arView
    }

    func updateUIView(_ view: ARView, context: Context) {}

    static func dismantleUIView(_ view: ARView, coordinator: Coordinator) {
        coordinator.scene?.stop()
        coordinator.scene = nil
    }
}

// MARK: - Status: caption, hints, result

private struct StatusOverlay: View {
    let model: ARViewerModel

    var body: some View {
        VStack(spacing: 8) {
            if !model.isCoaching {
                VStack(spacing: 2) {
                    Text(model.caption)
                        .font(.subheadline.weight(.semibold))
                    #if DEBUG
                    if let mode = model.firmwareMode {
                        Text("Firmware: \(mode)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.secondary)
                    }
                    if let check = model.boundsCheck {
                        Text(check.text)
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(check.ok ? Color.green : Color.red)
                    }
                    #endif
                }
                .pill()
            }
            if let hint = model.hint {
                Text(hint).font(.footnote).pill()
            }
            if let error = model.loadError {
                Text(error).font(.footnote).foregroundStyle(.red).pill()
            }
            if let face = model.faceUp {
                Text("Face up: \(face.label)").font(.callout.weight(.medium)).pill()
            }
            if model.isLoading {
                ProgressView().pill()
            }
        }
        .multilineTextAlignment(.center)
        .padding(.horizontal, 16)
        .animation(.default, value: model.faceUp)
    }
}

// MARK: - iPhone controls

private struct CompactControls: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        VStack(spacing: 10) {
            if model.canExplode, !model.isLidOff {
                ExplodeSlider(model: model)
                    .padding(.horizontal, 12)
                    .pill()
            }
            // Icons only when the labels don't fit the width.
            ViewThatFits(in: .horizontal) {
                viewRow
                viewRow.labelStyle(.iconOnly)
            }
            .padding(.horizontal, 12)
            HStack(spacing: 8) {
                RollButtons(model: model)
            }
            ModelPicker(model: model)
        }
        .padding(.bottom, 12)
    }

    private var viewRow: some View {
        HStack(spacing: 8) {
            ViewButtons(model: model)
            Spacer(minLength: 0)
            SizeLock(model: model)
        }
    }
}

/// Size, then colour: a row of chips each, over the camera view.
private struct ModelPicker: View {
    let model: ARViewerModel

    var body: some View {
        VStack(spacing: 6) {
            if model.sizes.count > 1 {
                ChipRow(items: model.sizes, id: \.self, label: { $0 }, isSelected: { $0 == model.die?.size }) {
                    model.chooseSize($0)
                }
            }
            if model.colours.count > 1 {
                ChipRow(items: model.colours, id: \.id, label: { $0.variant }, isSelected: { $0 == model.die }) {
                    model.choose($0)
                }
            }
        }
    }
}

private struct ChipRow<Item, ID: Hashable>: View {
    let items: [Item]
    let id: KeyPath<Item, ID>
    let label: (Item) -> String
    let isSelected: (Item) -> Bool
    let action: (Item) -> Void

    var body: some View {
        // Centred when it fits, scrolling when it doesn't.
        ViewThatFits(in: .horizontal) {
            chips
            ScrollView(.horizontal, showsIndicators: false) { chips }
        }
    }

    private var chips: some View {
        HStack(spacing: 8) {
            ForEach(items, id: id) { item in
                let selected = isSelected(item)
                Button { action(item) } label: {
                    Text(label(item))
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                }
                .buttonStyle(.plain)
                .background(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.regularMaterial), in: Capsule())
                .foregroundStyle(selected ? Color.white : Color.primary)
                .accessibilityAddTraits(selected ? .isSelected : [])
            }
        }
        .padding(.horizontal, 12)
    }
}

// MARK: - iPad side panel

private struct SidePanel: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        Form {
            Section("Model") {
                if model.sizes.count > 1 {
                    Picker("Size", selection: Binding(get: { model.die?.size ?? "" }, set: { model.chooseSize($0) })) {
                        ForEach(model.sizes, id: \.self) { Text($0).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }
                if model.colours.count > 1 {
                    Picker("Colour", selection: Binding(get: { model.die?.id ?? "" }, set: { id in
                        if let die = model.colours.first(where: { $0.id == id }) { model.choose(die) }
                    })) {
                        ForEach(model.colours) { Text($0.variant).tag($0.id) }
                    }
                }
                if let die = model.die {
                    LabeledContent("Measures", value: die.sizeLabel)
                }
            }
            Section("Roll") {
                ControlPad(model: model)
                    .listRowInsets(EdgeInsets(top: 8, leading: 8, bottom: 8, trailing: 8))
                // Icons only: the side panel is too narrow for their titles,
                // which wrapped. VoiceOver still reads them.
                HStack {
                    RollButtons(model: model)
                }
                .labelStyle(.iconOnly)
                if let face = model.faceUp {
                    LabeledContent("Face up", value: face.label)
                }
            }
            Section("View") {
                if model.canRunFirmware {
                    Toggle("Live screens", isOn: $model.liveScreens)
                }
                if model.canToggleXray {
                    Toggle("X-ray", isOn: Binding(get: { model.isXray }, set: { _ in model.toggleXray() }))
                }
                if let other = model.explodedCounterpart {
                    Button(other.kind == .explodedXray ? "Show pre-exploded model" : "Show assembled x-ray") {
                        model.toggleExplodedFile()
                    }
                }
                if model.canTakeLidOff {
                    Toggle("Lid off", isOn: Binding(get: { model.isLidOff }, set: { _ in model.toggleLid() }))
                        .disabled(!model.canToggleLid)
                }
                if model.canExplode, !model.isLidOff {
                    ExplodeSlider(model: model)
                }
            }
            Section("Size") {
                Toggle("True size", isOn: $model.isTrueSizeLocked)
                if !model.isTrueSizeLocked {
                    LabeledContent("Scale") {
                        Button(model.scalePercent) { model.resetScale() }
                    }
                    Text("Pinch the die to scale it. Tap the percentage to return to 100%.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// A trackpad for the die: drag sideways to turn it, flick to throw it in that
/// direction (up is away from you).
private struct ControlPad: View {
    let model: ARViewerModel
    @State private var lastX: CGFloat = 0

    var body: some View {
        RoundedRectangle(cornerRadius: 14)
            .fill(.quaternary)
            .frame(height: 160)
            .overlay {
                VStack(spacing: 6) {
                    Image(systemName: "hand.draw")
                        .font(.title2)
                    Text(model.isPlaced ? "Drag to turn the die\nFlick to throw it" : "Place the die first")
                        .font(.footnote)
                        .multilineTextAlignment(.center)
                }
                .foregroundStyle(.secondary)
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let x = value.translation.width
                        model.scene?.turn(by: Float(x - lastX) * 0.01)
                        lastX = x
                    }
                    .onEnded { value in
                        lastX = 0
                        let v = value.velocity
                        let speed = hypot(v.width, v.height)
                        if speed > DiePhysics.flickThreshold, model.canThrow {
                            model.scene?.throwDie(screenDirection: CGVector(dx: v.width, dy: v.height), flickSpeed: speed)
                        }
                    }
            )
            .disabled(!model.isPlaced)
            .accessibilityLabel("Die control pad")
            .accessibilityHint("Drag sideways to turn the die. Flick to throw it.")
    }
}

// MARK: - AR pane

/// A small pane on the left edge, on iPad: AR on (the camera and your table)
/// or off (the die locked in place on a virtual table). On iPhone it's a
/// toggle beside X-ray instead.
private struct ARPane: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        VStack(spacing: 4) {
            Image(systemName: model.augmented ? "arkit" : "cube")
                .font(.title3)
                .foregroundStyle(model.augmented ? Color.accentColor : Color.secondary)
            Text("AR").font(.caption.weight(.semibold))
            Toggle("AR", isOn: $model.augmented)
                .labelsHidden()
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 8)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("AR")
        .accessibilityValue(model.augmented ? "On" : "Off")
    }
}

// MARK: - Shared controls

private struct ViewButtons: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        if model.canRunFirmware {
            Toggle(isOn: $model.liveScreens) {
                Label("Live", systemImage: model.liveScreens ? "play.display" : "display")
            }
            .toggleStyle(.button)
            .buttonStyle(.bordered)
            .tint(.primary)
        }
        if model.canToggleXray {
            Button { model.toggleXray() } label: {
                Label("X-ray", systemImage: model.isXray ? "cube.transparent.fill" : "cube.transparent")
            }
            .buttonStyle(.bordered)
            .tint(model.isXray ? .accentColor : .primary)
        }
        if model.arSupported {
            Toggle(isOn: $model.augmented) {
                Label("AR", systemImage: "arkit")
            }
            .toggleStyle(.button)
            .buttonStyle(.bordered)
            .tint(model.augmented ? .accentColor : .primary)
        }
        if model.canTakeLidOff {
            Button { model.toggleLid() } label: {
                Label(model.isLidOff ? "Lid on" : "Lid off", systemImage: model.isLidOff ? "square.stack.3d.down.forward.fill" : "square.stack.3d.down.forward")
            }
            .buttonStyle(.bordered)
            .tint(model.isLidOff ? .accentColor : .primary)
            .disabled(!model.canToggleLid)
        }
        if let other = model.explodedCounterpart {
            Button { model.toggleExplodedFile() } label: {
                Label(other.kind == .explodedXray ? "Exploded" : "Assembled", systemImage: "square.3.layers.3d.down.right")
            }
            .buttonStyle(.bordered)
            .tint(.primary)
        }
    }
}

private struct SizeLock: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        if !model.isTrueSizeLocked {
            Button(model.scalePercent) { model.resetScale() }
                .buttonStyle(.bordered)
                .tint(.primary)
                .accessibilityHint("Returns to 100%")
        }
        Toggle(isOn: $model.isTrueSizeLocked) {
            Label("True size", systemImage: model.isTrueSizeLocked ? "lock.fill" : "lock.open")
        }
        .toggleStyle(.button)
        .buttonStyle(.bordered)
        .tint(.primary)
    }
}

private struct RollButtons: View {
    let model: ARViewerModel

    var body: some View {
        Button { model.roll() } label: { Label("Roll", systemImage: "dice") }
            .buttonStyle(.borderedProminent)
            .disabled(!model.canThrow)
        Button { model.resetDie() } label: { Label("Reset", systemImage: "arrow.uturn.backward") }
            .buttonStyle(.bordered)
            .disabled(!model.isPlaced)
        if model.isPlaced {
            Button { model.placeAgain() } label: { Label("Place", systemImage: "scope") }
                .buttonStyle(.bordered)
        }
    }
}

private struct ExplodeSlider: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        HStack {
            Image(systemName: "square.stack.3d.up")
            Slider(value: $model.explode, in: 0...1)
                .disabled(model.isRolling)
            Text("Explode").font(.caption)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Explode")
    }
}

private struct Unavailable: View {
    let title: String
    let message: String
    var action: (String, () -> Void)?
    var secondary: (String, () -> Void)?

    var body: some View {
        ContentUnavailableView {
            Label(title, systemImage: "arkit")
        } description: {
            Text(message)
        } actions: {
            if let action {
                Button(action.0, action: action.1).buttonStyle(.borderedProminent)
            }
            if let secondary {
                Button(secondary.0, action: secondary.1).buttonStyle(.bordered)
            }
        }
    }
}

private extension View {
    func pill() -> some View {
        padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(.regularMaterial, in: Capsule())
    }
}
