package com.smokebomb.app.ar

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * The platform's AR viewer, shown in the AR tab. iOS supplies one (RealityKit,
 * in Swift); where there's none, the tab is hidden.
 */
interface ArViewer {
    @Composable
    fun Content(modifier: Modifier)
}
