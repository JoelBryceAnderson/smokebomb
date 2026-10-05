package com.smokebomb.app

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import com.smokebomb.app.ar.ArViewer
import com.smokebomb.app.ble.BleManager
import com.smokebomb.app.ble.DieLinks
import com.smokebomb.app.ble.DiePort
import com.smokebomb.app.history.RollHistory
import com.smokebomb.app.navigation.AppShell
import com.smokebomb.app.network.ApiClient
import com.smokebomb.app.network.StubApiClient
import com.smokebomb.app.screens.OnboardingScreen
import com.smokebomb.app.settings.AppSettings
import com.smokebomb.app.theme.SmokebombTheme

/** Root composable shared by Android and iOS. */
@Composable
fun App(
    ble: BleManager,
    settings: AppSettings,
    api: ApiClient = remember { StubApiClient() },
    arViewer: ArViewer? = null,
    arDie: DiePort? = null,
) {
    val scope = rememberCoroutineScope()
    // Bluetooth, or a simulator (the desktop's, or the Simulator tab's die) when the Die tab picks one.
    val links = remember(ble, arDie) { DieLinks(ble, scope, arDie) }
    val history = remember(links) { RollHistory(links, scope) }
    SmokebombTheme {
        // TODO: persist onboarding completion (DataStore / NSUserDefaults).
        var onboarded by remember { mutableStateOf(false) }
        if (onboarded) {
            AppShell(links = links, history = history, api = api, arViewer = arViewer)
        } else {
            OnboardingScreen(
                initialName = settings.ownerName.orEmpty(),
                onFinished = { name ->
                    settings.ownerName = name
                    onboarded = true
                },
            )
        }
    }
}
