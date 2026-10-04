package com.smokebomb.app

import androidx.compose.runtime.remember
import androidx.compose.ui.window.ComposeUIViewController
import com.smokebomb.app.ble.PlatformContext
import com.smokebomb.app.ble.createBleManager
import com.smokebomb.app.settings.createAppSettings

/** Entry point called from SwiftUI (`iosApp/iosApp/ContentView.swift`). */
fun MainViewController() = ComposeUIViewController {
    val ble = remember { createBleManager(PlatformContext.INSTANCE) }
    val settings = remember { createAppSettings(PlatformContext.INSTANCE) }
    App(ble, settings)
}
