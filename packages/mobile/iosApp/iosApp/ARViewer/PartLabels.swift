import Foundation

/// What to call a part when it's tapped, read from `PartLabels.json`.
struct PartLabels: Sendable {
    private let exact: [String: String]
    /// Prefix patterns, longest first.
    private let prefixes: [(prefix: String, label: String)]

    init(json: Data) throws {
        let entries = try JSONDecoder().decode([String: String].self, from: json)
            .filter { !$0.key.hasPrefix("_") }
        exact = entries.filter { !$0.key.hasSuffix("*") }
        prefixes = entries.filter { $0.key.hasSuffix("*") }
            .map { (String($0.key.dropLast()), $0.value) }
            .sorted { $0.prefix.count > $1.prefix.count }
    }

    /// The labels bundled with the app; none if the file is missing or malformed.
    static func load(bundle: Bundle) -> PartLabels {
        guard let url = bundle.url(forResource: "PartLabels", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let labels = try? PartLabels(json: data)
        else {
            assertionFailure("PartLabels.json is missing or malformed")
            return try! PartLabels(json: Data("{}".utf8))
        }
        return labels
    }

    func label(for name: String) -> String? {
        if let label = exact[name] { return label }
        return prefixes.first { name.hasPrefix($0.prefix) }?.label
    }
}
