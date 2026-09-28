package com.smokebomb.app.navigation

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.smokebomb.app.ble.BleManager
import com.smokebomb.app.network.ApiClient
import com.smokebomb.app.screens.DieScreen
import com.smokebomb.app.screens.RollHistoryScreen
import com.smokebomb.app.screens.SettingsScreen
import com.smokebomb.app.screens.ThemeStoreScreen

/** Top-level destinations in the bottom bar. */
enum class Destination(val label: String, val glyph: String) {
    Die("Die", "⚅"),
    History("History", "☰"),
    Store("Store", "◈"),
    Settings("Settings", "⚙"),
}

/**
 * Navigation shell: a bottom bar over four tabs. Deeper stacks (roll detail,
 * theme detail, DFU progress) will move this to navigation-compose.
 */
@Composable
fun AppShell(ble: BleManager, api: ApiClient) {
    var current by rememberSaveable { mutableStateOf(Destination.Die) }

    Scaffold(
        bottomBar = {
            NavigationBar {
                Destination.entries.forEach { dest ->
                    NavigationBarItem(
                        selected = dest == current,
                        onClick = { current = dest },
                        icon = { Text(dest.glyph) },
                        label = { Text(dest.label) },
                    )
                }
            }
        },
    ) { padding ->
        val modifier = Modifier.padding(padding)
        when (current) {
            Destination.Die -> DieScreen(ble, modifier)
            Destination.History -> RollHistoryScreen(api, modifier)
            Destination.Store -> ThemeStoreScreen(api, modifier)
            Destination.Settings -> SettingsScreen(modifier)
        }
    }
}
