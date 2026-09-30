package com.smokebomb.app.screens

import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

@Composable
fun SettingsScreen(modifier: Modifier = Modifier) {
    ScreenScaffold("Settings", modifier) {
        ListItem(headlineContent = { Text("Firmware update") }, supportingContent = { Text("DFU over BLE — coming soon") })
        ListItem(headlineContent = { Text("Account") }, supportingContent = { Text("Sign in to sync roll history") })
        ListItem(headlineContent = { Text("About") }, supportingContent = { Text("Sugarcube 0.1.0") })
    }
}
