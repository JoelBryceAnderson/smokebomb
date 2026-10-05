package com.smokebomb.app

import androidx.compose.runtime.remember
import androidx.compose.ui.window.ComposeUIViewController
import com.smokebomb.app.ar.ArViewControllerFactory
import com.smokebomb.app.ar.UIKitArViewer
import com.smokebomb.app.ble.PlatformContext
import com.smokebomb.app.ble.createBleManager
import com.smokebomb.app.settings.createAppSettings

/** Entry point called from SwiftUI (`iosApp/iosApp/ContentView.swift`), which also supplies the AR viewer. */
fun MainViewController(arViewer: ArViewControllerFactory) = ComposeUIViewController {
    val ble = remember { createBleManager(PlatformContext.INSTANCE) }
    val settings = remember { createAppSettings(PlatformContext.INSTANCE) }
    val ar = remember(arViewer) { UIKitArViewer(arViewer) }
    App(ble, settings, arViewer = ar)
}
