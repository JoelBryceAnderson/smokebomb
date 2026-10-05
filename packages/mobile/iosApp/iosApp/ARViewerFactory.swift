import ComposeApp
import SwiftUI
import UIKit

/// Makes the AR tab's screen for the Compose shell, which declares the
/// `ArViewControllerFactory` protocol (composeApp/src/iosMain). RealityKit is
/// Swift-only, so the viewer lives here and Compose embeds it.
@MainActor
final class ARViewerFactory: NSObject, @preconcurrency ArViewControllerFactory {
    func makeArViewController() -> UIViewController {
        let model = ARViewerModel.shared
        // Live screens: the real firmware, from the Rust library.
        model.makeFirmware = { RustDieFirmware(panel: $0) }
        return UIHostingController(rootView: ARViewerScreen(model: model))
    }
}
