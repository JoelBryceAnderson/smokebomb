import Foundation
import RealityKit

/// One model the AR viewer can show. See AR_VIEWER.md for how to add one.
struct SugarcubeModel: Identifiable, Hashable, Sendable {
    enum Kind: Sendable {
        case die, xray, explodedXray, lineup, fixture
    }

    /// The file name without `.usdz`.
    let id: String
    let size: String
    let variant: String
    let kind: Kind
    /// Real-world visual bounds in metres (x, y, z). The scale test checks these to ±0.1 mm.
    let bounds: SIMD3<Float>
    /// Approximate mass. Nil means the model can't be thrown (the line-up, the pre-exploded x-ray).
    let massKg: Float?
    /// The matching x-ray model, for a die.
    var xray: String?
    /// The die an x-ray belongs to, for the X-ray toggle's way back.
    var die: String?
    /// The pre-exploded counterpart, in either direction.
    var exploded: String?

    /// "30 mm · Rainbow"
    var title: String { "\(size) · \(variant)" }

    /// "30.0 mm" for a cube, "156.0 × 40.0 × 40.0 mm" otherwise.
    var sizeLabel: String {
        let mm = bounds * 1000
        if mm.x == mm.y && mm.y == mm.z { return String(format: "%.1f mm", mm.x) }
        return String(format: "%.1f × %.1f × %.1f mm", mm.x, mm.y, mm.z)
    }

    var isThrowable: Bool { massKg != nil }
}

/// Where model files come from. Bundled for v1; a download source can be added
/// later by implementing this and passing it to `ModelCatalog`.
protocol ModelSource: Sendable {
    /// Whether the model can be loaded now, without waiting. Hides missing files from the picker.
    func isAvailable(_ model: SugarcubeModel) -> Bool
    /// A local file URL for the model, fetching it first if the source needs to.
    func url(for model: SugarcubeModel) async throws -> URL
}

/// Models bundled in the app's `Models` folder (`iosApp/Resources/Models`).
struct BundleModelSource: ModelSource {
    let bundle: Bundle
    var subdirectory = "Models"

    func isAvailable(_ model: SugarcubeModel) -> Bool {
        fileURL(model) != nil
    }

    func url(for model: SugarcubeModel) async throws -> URL {
        guard let url = fileURL(model) else { throw ModelCatalog.Failure.notBundled(model.id) }
        return url
    }

    private func fileURL(_ model: SugarcubeModel) -> URL? {
        bundle.url(forResource: model.id, withExtension: "usdz", subdirectory: subdirectory)
    }
}

/// The models the viewer knows about, and loading them.
@MainActor
final class ModelCatalog {
    enum Failure: LocalizedError {
        case notBundled(String)

        var errorDescription: String? {
            switch self {
            case .notBundled(let id): "\(id).usdz isn't in the app. Run scripts/convert_usdz.py and rebuild."
            }
        }
    }

    // Masses: titanium shell plus internals. Keep in step with scripts/convert_usdz.py's size table.
    nonisolated static let all: [SugarcubeModel] = {
        let mm30 = SIMD3<Float>(repeating: 0.030), mm34 = SIMD3<Float>(repeating: 0.034), mm40 = SIMD3<Float>(repeating: 0.040)
        return [
            SugarcubeModel(id: "sugarcube_30_rainbow", size: "30 mm", variant: "Rainbow", kind: .die, bounds: mm30, massKg: 0.055,
                           xray: "sugarcube_30_xray"),
            SugarcubeModel(id: "sugarcube_30", size: "30 mm", variant: "Colour", kind: .die, bounds: mm30, massKg: 0.055,
                           xray: "sugarcube_30_xray"),
            SugarcubeModel(id: "sugarcube_34", size: "34 mm", variant: "16-grey", kind: .die, bounds: mm34, massKg: 0.082,
                           xray: "sugarcube_34_xray"),
            SugarcubeModel(id: "sugarcube_34_1bit", size: "34 mm", variant: "1-bit", kind: .die, bounds: mm34, massKg: 0.082,
                           xray: "sugarcube_34_xray"),
            SugarcubeModel(id: "sugarcube_40", size: "40 mm", variant: "Colour", kind: .die, bounds: mm40, massKg: 0.129,
                           xray: "sugarcube_40_xray"),
            SugarcubeModel(id: "sugarcube_30_xray", size: "30 mm", variant: "X-ray", kind: .xray, bounds: mm30, massKg: 0.055,
                           die: "sugarcube_30", exploded: "sugarcube_30_xray_exploded"),
            SugarcubeModel(id: "sugarcube_34_xray", size: "34 mm", variant: "X-ray", kind: .xray, bounds: mm34, massKg: 0.082,
                           die: "sugarcube_34"),
            SugarcubeModel(id: "sugarcube_40_xray", size: "40 mm", variant: "X-ray", kind: .xray, bounds: mm40, massKg: 0.129,
                           die: "sugarcube_40"),
            SugarcubeModel(id: "sugarcube_30_xray_exploded", size: "30 mm", variant: "X-ray, exploded", kind: .explodedXray,
                           bounds: SIMD3(repeating: 0.0687), massKg: nil, die: "sugarcube_30", exploded: "sugarcube_30_xray"),
            SugarcubeModel(id: "sugarcube_lineup", size: "Line-up", variant: "16, 30, 34, 40 mm", kind: .lineup,
                           bounds: SIMD3(0.156, 0.040, 0.040), massKg: nil),
            // Crude boxes named per the per-part contract (scripts/make_contract_fixture.py), for trying
            // explode, the shell fade and tap-to-name before the real per-part models exist. Debug builds only.
            SugarcubeModel(id: "sugarcube_contract_fixture", size: "30 mm", variant: "Per-part test", kind: .fixture,
                           bounds: mm30, massKg: 0.055),
        ]
    }()

    /// What the viewer shows first: the 30 mm rainbow die.
    nonisolated static let defaultID = "sugarcube_30_rainbow"

    let source: ModelSource
    private var loaded: [String: Entity] = [:]

    init(source: ModelSource) {
        self.source = source
    }

    /// The models that can be shown now, in picker order.
    var available: [SugarcubeModel] {
        Self.all.filter { model in
            #if !DEBUG
            if model.kind == .fixture { return false }
            #endif
            return source.isAvailable(model)
        }
    }

    func model(id: String?) -> SugarcubeModel? {
        guard let id else { return nil }
        return Self.all.first { $0.id == id }
    }

    /// A fresh copy of the model's entity tree. Files are read once and cloned after that.
    func load(_ model: SugarcubeModel) async throws -> Entity {
        if let entity = loaded[model.id] { return entity.clone(recursive: true) }
        let url = try await source.url(for: model)
        let entity = try await Entity(contentsOf: url)
        loaded[model.id] = entity
        return entity.clone(recursive: true)
    }
}
