#if GBC_CUBE
import SwiftUI
import UniformTypeIdentifiers

/// The Simulator tab with the GBC cube's controls over it: a Game Boy menu
/// (load a ROM, the demo cart, the die firmware) and, while a game runs, a
/// joypad. The die itself plays as in the experiment's simulator:
/// - lean it with two fingers to walk; a lean stays where it's left, so the
///   player keeps walking (the viewer leans up to about 26°, inside the
///   cube's 12–40° walking range);
/// - drag further and it rolls onto that side, and the map rolls with it;
/// - tap the up face for A, a side face for B, hold the up face for Start;
/// - throw it and the map rolls onto whichever face lands up.
/// The joypad is there for precise steps and for menus. It sits along the
/// bottom, clear of the die, in place of the viewer's iPhone controls
/// (switch back to the die firmware for those).
struct GbcCubeScreen: View {
    @Bindable var model: ARViewerModel
    @Bindable var session: GbcCubeSession
    @State private var picking = false

    var body: some View {
        ARViewerScreen(model: model, accessory: joypad)
            .overlay(alignment: .topTrailing) {
                menu.padding(.top, 60).padding(.trailing, 12)
            }
            .fileImporter(isPresented: $picking, allowedContentTypes: [.data]) { result in
                if case .success(let url) = result {
                    session.importROM(from: url, on: model)
                }
            }
            .alert(
                "The game didn't start",
                isPresented: Binding(get: { session.error != nil }, set: { if !$0 { session.error = nil } })
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(session.error ?? "")
            }
    }

    private var joypad: AnyView? {
        guard session.cartridge != .off else { return nil }
        return AnyView(
            Joypad(keys: $session.keys)
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
        )
    }

    private var menu: some View {
        Menu {
            Button("Load ROM…", systemImage: "square.and.arrow.down") { picking = true }
            if !session.roms.isEmpty {
                Section("Your ROMs") {
                    ForEach(session.roms, id: \.self) { url in
                        Button(url.deletingPathExtension().lastPathComponent) { session.select(.rom(url), on: model) }
                    }
                }
            }
            Button("Demo cart", systemImage: "gamecontroller") { session.select(.demo, on: model) }
            Button("Die firmware", systemImage: "die.face.5") { session.select(.off, on: model) }
            Picker("Text and menus", selection: $session.layout) {
                Text("C: front face").tag(UInt8(0))
                Text("A: pan").tag(UInt8(1))
                Text("B: spread").tag(UInt8(2))
            }
        } label: {
            Label(session.cartridge == .off ? "Game Boy" : (session.title ?? "Game Boy"), systemImage: "gamecontroller.fill")
                .font(.footnote.weight(.semibold))
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(.regularMaterial, in: Capsule())
        }
    }
}

/// D-pad on the left, A and B on the right, Start and Select between: held
/// while a finger is on them.
private struct Joypad: View {
    @Binding var keys: UInt8

    var body: some View {
        HStack(alignment: .bottom) {
            VStack(spacing: 2) {
                key("▲", 0x40)
                HStack(spacing: 2) {
                    key("◀", 0x20)
                    Color.clear.frame(width: 44, height: 44)
                    key("▶", 0x10)
                }
                key("▼", 0x80)
            }
            Spacer()
            VStack(spacing: 10) {
                HStack(spacing: 8) {
                    key("Select", 0x04, small: true)
                    key("Start", 0x08, small: true)
                }
                HStack(spacing: 10) {
                    key("B", 0x02)
                    key("A", 0x01)
                }
            }
        }
    }

    private func key(_ label: String, _ bit: UInt8, small: Bool = false) -> some View {
        let held = keys & bit != 0
        return Text(label)
            .font(small ? .caption2.weight(.semibold) : .headline)
            .frame(width: small ? 56 : 44, height: small ? 26 : 44)
            .background(held ? AnyShapeStyle(.tint) : AnyShapeStyle(.regularMaterial), in: RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in if keys & bit == 0 { keys |= bit } }
                    .onEnded { _ in keys &= ~bit }
            )
            .accessibilityLabel(label)
    }
}
#endif
