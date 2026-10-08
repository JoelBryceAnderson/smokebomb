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
        // The app's die links reach the running die through this.
        model.phoneLink = ARDiePort.shared
        #if GBC_CUBE
        // The GBC cube experiment (SUGARCUBE_GBC_CUBE = YES in Local.xcconfig):
        // the die runs a Game Boy game when one is picked, the firmware otherwise.
        let session = GbcCubeSession.shared
        model.makeFirmware = { session.makeFirmware(panel: $0) }
        return UIHostingController(rootView: GbcCubeScreen(model: model, session: session))
        #else
        // Live screens: the real firmware, from the Rust library.
        model.makeFirmware = { RustDieFirmware(panel: $0) }
        return UIHostingController(rootView: ARViewerScreen(model: model))
        #endif
    }
}
