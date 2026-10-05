import ComposeApp
import SwiftUI
import UIKit

/// Hosts the shared Compose UI.
struct ComposeView: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> UIViewController {
        MainViewControllerKt.MainViewController(arViewer: ARViewerFactory(), arDie: ARDiePort.shared)
    }

    func updateUIViewController(_ uiViewController: UIViewController, context: Context) {}
}

struct ContentView: View {
    var body: some View {
        ComposeView().ignoresSafeArea()
    }
}
