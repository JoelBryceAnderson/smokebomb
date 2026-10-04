import RealityKit
import SwiftUI

/// The AR tab. On iPhone (or any compact width) the controls float over the
/// camera; on iPad they sit in a side panel next to it, with a control pad for
/// turning and throwing the die while you watch it on the table.
struct ARViewerScreen: View {
    @Bindable var model: ARViewerModel
    @Environment(\.horizontalSizeClass) private var widthClass
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            switch model.availability {
            case .checking:
                ProgressView()
            case .unsupported:
                Unavailable(
                    title: "AR isn't available here",
                    message: "The AR viewer needs a device with an A12 chip or later and a rear camera. It doesn't run in the Simulator.")
            case .cameraDenied:
                Unavailable(
                    title: "Camera access is off",
                    message: "The AR viewer shows the die on your table through the camera. Turn on camera access for Sugarcube in Settings.",
                    action: ("Open Settings", {
                        if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                    }))
            case .ready:
                // One structure for both widths, so resizing in Split View keeps the AR session and the placed die.
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
        .task { await model.checkAvailability() }
        .onChange(of: scenePhase) { _, phase in
            // Coming back from Settings with camera access changed.
            if phase == .active, model.availability == .cameraDenied {
                Task { await model.checkAvailability() }
            }
        }
    }

    private var stage: some View {
        ZStack(alignment: .top) {
            ARViewContainer(model: model)
                .ignoresSafeArea(edges: .top)
            StatusOverlay(model: model)
                .padding(.top, 8)
        }
    }
}

// MARK: - AR view

private struct ARViewContainer: UIViewRepresentable {
    let model: ARViewerModel

    final class Coordinator {
        var scene: ARSceneController?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> ARView {
        let scene = ARSceneController(model: model)
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

// MARK: - Status: caption, hints, part label, result

private struct StatusOverlay: View {
    let model: ARViewerModel

    var body: some View {
        VStack(spacing: 8) {
            if !model.isCoaching {
                VStack(spacing: 2) {
                    Text(model.caption)
                        .font(.subheadline.weight(.semibold))
                    #if DEBUG
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
            if let part = model.selectedPart {
                Text(part).font(.callout.weight(.medium)).pill()
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
        .animation(.default, value: model.selectedPart)
        .animation(.default, value: model.faceUp)
    }
}

// MARK: - iPhone controls

private struct CompactControls: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        VStack(spacing: 10) {
            if model.canExplode {
                ExplodeSlider(model: model)
                    .padding(.horizontal, 12)
                    .pill()
            }
            HStack(spacing: 8) {
                ViewButtons(model: model)
                Spacer(minLength: 0)
                SizeLock(model: model)
            }
            .padding(.horizontal, 12)
            HStack(spacing: 8) {
                RollButtons(model: model)
            }
            ModelCarousel(model: model)
        }
        .padding(.bottom, 12)
    }
}

private struct ModelCarousel: View {
    let model: ARViewerModel

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(model.models) { item in
                        Button { model.select(item) } label: {
                            VStack(spacing: 1) {
                                Text(item.size).font(.caption.weight(.semibold))
                                Text(item.variant).font(.caption2)
                            }
                            .padding(.horizontal, 12)
                            .padding(.vertical, 6)
                        }
                        .buttonStyle(.plain)
                        .background(item == model.selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.regularMaterial), in: Capsule())
                        .foregroundStyle(item == model.selected ? Color.white : Color.primary)
                        .id(item.id)
                    }
                }
                .padding(.horizontal, 12)
            }
            .onChange(of: model.selected) { _, selected in
                guard let id = selected?.id else { return }
                withAnimation { proxy.scrollTo(id, anchor: .center) }
            }
        }
    }
}

// MARK: - iPad side panel

private struct SidePanel: View {
    @Bindable var model: ARViewerModel

    var body: some View {
        Form {
            Section("Model") {
                ForEach(model.models) { item in
                    Button { model.select(item) } label: {
                        HStack {
                            VStack(alignment: .leading) {
                                Text(item.title)
                                Text(item.sizeLabel).font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if item == model.selected { Image(systemName: "checkmark").foregroundStyle(.tint) }
                        }
                    }
                    .foregroundStyle(.primary)
                }
            }
            Section("Roll") {
                ControlPad(model: model)
                    .listRowInsets(EdgeInsets(top: 8, leading: 8, bottom: 8, trailing: 8))
                HStack {
                    RollButtons(model: model)
                }
                if let face = model.faceUp {
                    LabeledContent("Face up", value: face.label)
                }
            }
            Section("View") {
                if model.canToggleXray {
                    Toggle("X-ray", isOn: Binding(get: { model.isXray }, set: { _ in model.toggleXray() }))
                }
                if let other = model.explodedCounterpart {
                    Button(other.kind == .explodedXray ? "Show pre-exploded model" : "Show assembled x-ray") {
                        model.toggleExplodedFile()
                    }
                }
                if model.canExplode {
                    ExplodeSlider(model: model)
                }
                if let part = model.selectedPart {
                    LabeledContent("Part", value: part)
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

// MARK: - Shared controls

private struct ViewButtons: View {
    let model: ARViewerModel

    var body: some View {
        if model.canToggleXray {
            Button { model.toggleXray() } label: {
                Label("X-ray", systemImage: model.isXray ? "cube.transparent.fill" : "cube.transparent")
            }
            .buttonStyle(.bordered)
            .tint(model.isXray ? .accentColor : .primary)
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

    var body: some View {
        ContentUnavailableView {
            Label(title, systemImage: "arkit")
        } description: {
            Text(message)
        } actions: {
            if let action {
                Button(action.0, action: action.1).buttonStyle(.borderedProminent)
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
