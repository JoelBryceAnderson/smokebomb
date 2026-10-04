package com.smokebomb.app.ar

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.UIKitInteropInteractionMode
import androidx.compose.ui.viewinterop.UIKitInteropProperties
import androidx.compose.ui.viewinterop.UIKitViewController
import platform.UIKit.UIViewController

/**
 * Makes the AR viewer's view controller. Implemented in Swift
 * (`iosApp/iosApp/ARViewerFactory.swift`) because RealityKit is Swift-only.
 */
interface ArViewControllerFactory {
    fun makeArViewController(): UIViewController
}

/** Embeds the Swift AR viewer in the Compose tab. */
class UIKitArViewer(private val factory: ArViewControllerFactory) : ArViewer {
    @OptIn(ExperimentalComposeUiApi::class)
    @Composable
    override fun Content(modifier: Modifier) {
        UIKitViewController(
            factory = { factory.makeArViewController() },
            modifier = modifier.fillMaxSize(),
            // The AR view takes every touch at once: taps, drags, flicks, twists and pinches.
            properties = UIKitInteropProperties(
                interactionMode = UIKitInteropInteractionMode.NonCooperative,
                isNativeAccessibilityEnabled = true,
            ),
        )
    }
}
