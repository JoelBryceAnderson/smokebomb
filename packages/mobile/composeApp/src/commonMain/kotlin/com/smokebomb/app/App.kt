package com.smokebomb.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import com.smokebomb.app.ble.BleManager
import com.smokebomb.app.ble.DieLinks
import com.smokebomb.app.navigation.AppShell
import com.smokebomb.app.network.ApiClient
import com.smokebomb.app.network.StubApiClient
import com.smokebomb.app.screens.OnboardingScreen
import com.smokebomb.app.theme.SmokebombTheme

/** Root composable shared by Android and iOS. */
@Composable
fun App(ble: BleManager, api: ApiClient = remember { StubApiClient() }) {
    val scope = rememberCoroutineScope()
    // Bluetooth, or the desktop simulator when the Die tab picks it.
    val links = remember(ble) { DieLinks(ble, scope) }
    SmokebombTheme {
        // TODO: persist onboarding completion (DataStore / NSUserDefaults).
        var onboarded by remember { mutableStateOf(false) }
        if (onboarded) {
            AppShell(links = links, api = api)
        } else {
            OnboardingScreen(onFinished = { onboarded = true })
        }
    }
}
