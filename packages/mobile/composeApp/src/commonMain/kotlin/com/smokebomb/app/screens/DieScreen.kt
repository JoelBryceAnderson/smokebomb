package com.smokebomb.app.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.smokebomb.app.ble.BleManager
import com.smokebomb.app.ble.BleState
import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import kotlinx.coroutines.launch

/** Find, connect to, and show the status of the user's die. */
@Composable
fun DieScreen(ble: BleManager, modifier: Modifier = Modifier) {
    val state by ble.state.collectAsState()
    val inventory by ble.inventory.collectAsState()
    val scope = rememberCoroutineScope()

    ScreenScaffold("My die", modifier) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            when (val s = state) {
                BleState.Off -> Text("Turn on Bluetooth to find your die.")
                BleState.Idle -> Button(onClick = ble::startScan) { Text("Find my Sugarcube") }
                is BleState.Scanning -> {
                    Text(if (s.found.isEmpty()) "Scanning…" else "Tap a die to connect")
                    LazyColumn {
                        items(s.found, key = { it.id }) { die ->
                            ListItem(
                                headlineContent = { Text(die.name) },
                                supportingContent = { Text("${die.rssi} dBm") },
                                trailingContent = {
                                    OutlinedButton(onClick = { scope.launch { ble.connect(die) } }) {
                                        Text("Connect")
                                    }
                                },
                            )
                        }
                    }
                    OutlinedButton(onClick = ble::stopScan) { Text("Stop") }
                }
                is BleState.Connecting -> Text("Connecting to ${s.die.name}…")
                is BleState.Connected -> {
                    Text("Connected to ${s.die.name}")
                    Text("Battery: ${s.batteryPercent?.let { "$it%" } ?: "—"}")
                    inventory?.let { inv ->
                        ModesSection(inv) { enabled -> scope.launch { ble.setEnabledModes(enabled) } }
                    }
                    OutlinedButton(onClick = { scope.launch { ble.disconnect() } }) { Text("Disconnect") }
                }
                is BleState.Error -> {
                    Text(s.message)
                    Button(onClick = ble::startScan) { Text("Try again") }
                }
            }
        }
    }
}

/**
 * Which modes the die's Mode page offers. Licensed modes have a switch;
 * the rest are locked until bought in the store. Dice is always on.
 */
@Composable
private fun ModesSection(inventory: Inventory, onChange: (Set<ModeId>) -> Unit) {
    Column {
        Text("Modes", style = MaterialTheme.typography.titleMedium)
        ModeId.entries.forEach { mode ->
            val licensed = mode in inventory.licensed
            ListItem(
                headlineContent = { Text(mode.label) },
                supportingContent = {
                    Text(
                        when {
                            mode == inventory.active -> "Playing now"
                            mode == ModeId.DICE -> "Always on"
                            !licensed -> "Get it in the store"
                            else -> ""
                        },
                    )
                },
                trailingContent = {
                    if (licensed) {
                        Switch(
                            checked = mode in inventory.enabled,
                            enabled = mode != ModeId.DICE,
                            onCheckedChange = { on ->
                                val next = if (on) inventory.enabled + mode else inventory.enabled - mode
                                onChange(inventory.request(next))
                            },
                        )
                    } else {
                        Text("🔒")
                    }
                },
            )
        }
    }
}
