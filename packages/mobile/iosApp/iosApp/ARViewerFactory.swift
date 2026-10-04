import ComposeApp
import SwiftUI
import UIKit

/// Makes the AR tab's screen for the Compose shell, which declares the
/// `ArViewControllerFactory` protocol (composeApp/src/iosMain). RealityKit is
/// Swift-only, so the viewer lives here and Compose embeds it.
@MainActor
final class ARViewerFactory: NSObject, @preconcurrency ArViewControllerFactory {
    func makeArViewController() -> UIViewController {
        UIHostingController(rootView: ARViewerScreen(model: .shared))
    }
}
