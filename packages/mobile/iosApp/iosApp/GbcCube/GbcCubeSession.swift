#if GBC_CUBE
import Foundation
import Observation

/// Which game the Simulator tab's die runs, if any, and the ROMs you've
/// loaded. ROMs are copied into the app's Documents/ROMs folder (visible in
/// the Files app when the app shares its Documents), and each game's battery
/// save sits next to it as a `.sav`.
@MainActor
@Observable
final class GbcCubeSession {
    static let shared = GbcCubeSession()

    enum Cartridge: Equatable {
        /// The die firmware, as without the experiment.
        case off
        /// The built-in demo cart (no ROM needed).
        case demo
        case rom(URL)
    }

    private(set) var cartridge: Cartridge
    /// Imported ROMs, by name.
    private(set) var roms: [URL] = []
    /// Why the last game didn't start, until the next one does.
    var error: String?
    /// The running game's title.
    private(set) var title: String?

    /// Text, menus and battles: `GC_UI_FRONT` (0), `GC_UI_PAN` (1) or
    /// `GC_UI_SPREAD` (2).
    var layout: UInt8 = 0 {
        didSet { firmware?.setLayout(layout) }
    }
    /// Joypad buttons held on screen.
    var keys: UInt8 = 0 {
        didSet { firmware?.keys = keys }
    }

    @ObservationIgnored private weak var firmware: GbcCubeFirmware?
    private static let cartridgeKey = "gbcCube.cartridge"

    let romsFolder: URL = {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        return docs.appendingPathComponent("ROMs", isDirectory: true)
    }()

    private init() {
        cartridge = .off
        refresh()
        switch UserDefaults.standard.string(forKey: Self.cartridgeKey) {
        case "demo": cartridge = .demo
        case let name? where !name.isEmpty:
            let url = romsFolder.appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: url.path) { cartridge = .rom(url) }
        default: break
        }
    }

    /// Makes what the tab runs for a die with `panel`s: the game on a 30 mm
    /// (colour) die, the die firmware otherwise.
    func makeFirmware(panel: PanelKind) -> DieFirmware? {
        let rom: URL?
        switch cartridge {
        case .off:
            return RustDieFirmware(panel: panel)
        case .demo:
            rom = nil
        case .rom(let url):
            rom = url
        }
        guard panel == .rgb64 else {
            error = "Game Boy games need the 30 mm colour die."
            return RustDieFirmware(panel: panel)
        }
        do {
            let game = try GbcCubeFirmware(rom: rom)
            game.keys = keys
            game.setLayout(layout)
            firmware = game
            title = game.title
            error = nil
            return game
        } catch let failure as GbcCubeFirmware.OpenError {
            error = failure.message
        } catch {
            self.error = error.localizedDescription
        }
        title = nil
        return RustDieFirmware(panel: panel)
    }

    /// Runs `cartridge` on the tab's die, rebooting whatever runs now.
    func select(_ cartridge: Cartridge, on model: ARViewerModel) {
        self.cartridge = cartridge
        let saved: String? = switch cartridge {
        case .off: nil
        case .demo: "demo"
        case .rom(let url): url.lastPathComponent
        }
        UserDefaults.standard.set(saved, forKey: Self.cartridgeKey)
        keys = 0
        // The tab rebuilds its firmware when live screens come back on.
        model.liveScreens = false
        model.liveScreens = true
    }

    /// Copies a ROM the file picker gave us into Documents/ROMs, replacing
    /// one of the same name, and runs it.
    func importROM(from picked: URL, on model: ARViewerModel) {
        let scoped = picked.startAccessingSecurityScopedResource()
        defer { if scoped { picked.stopAccessingSecurityScopedResource() } }
        do {
            try FileManager.default.createDirectory(at: romsFolder, withIntermediateDirectories: true)
            let destination = romsFolder.appendingPathComponent(picked.lastPathComponent)
            if FileManager.default.fileExists(atPath: destination.path) {
                try FileManager.default.removeItem(at: destination)
            }
            try FileManager.default.copyItem(at: picked, to: destination)
            refresh()
            select(.rom(destination), on: model)
        } catch {
            self.error = "Couldn't load \(picked.lastPathComponent): \(error.localizedDescription)"
        }
    }

    private func refresh() {
        let files = (try? FileManager.default.contentsOfDirectory(at: romsFolder, includingPropertiesForKeys: nil)) ?? []
        roms = files
            .filter { ["gb", "gbc"].contains($0.pathExtension.lowercased()) }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
    }
}
#endif
