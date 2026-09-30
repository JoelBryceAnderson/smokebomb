package com.smokebomb.app.screens

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.smokebomb.app.ble.BleState
import com.smokebomb.app.ble.DieLinks
import com.smokebomb.shared.DieKind
import com.smokebomb.shared.Inventory
import com.smokebomb.shared.ModeId
import com.smokebomb.shared.SignedRoll
import kotlinx.coroutines.launch

/** Find, connect to, and set up the user's die, or the desktop simulator. */
@Composable
fun DieScreen(links: DieLinks, modifier: Modifier = Modifier) {
    val state by links.state.collectAsState()
    val inventory by links.inventory.collectAsState()
    val simulator by links.simulator.collectAsState()
    val scope = rememberCoroutineScope()
    var lastRoll by remember { mutableStateOf<SignedRoll?>(null) }
    LaunchedEffect(links) { links.rolls.collect { lastRoll = it } }

    ScreenScaffold("My die", modifier) {
        Column(
            Modifier.verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            when (val s = state) {
                BleState.Off -> Text("Turn on Bluetooth to find your die.")
                BleState.Idle -> Button(onClick = links::startScan) { Text("Find my Sugarcube") }
                is BleState.Scanning -> {
                    Text(if (s.found.isEmpty()) "Scanning…" else "Tap a die to connect")
                    s.found.forEach { die ->
                        ListItem(
                            headlineContent = { Text(die.name) },
                            supportingContent = { Text("${die.rssi} dBm") },
                            trailingContent = {
                                OutlinedButton(onClick = { scope.launch { links.connect(die) } }) {
                                    Text("Connect")
                                }
                            },
                        )
                    }
                    OutlinedButton(onClick = links::stopScan) { Text("Stop") }
                }
                is BleState.Connecting -> Text("Connecting to ${s.die.name}…")
                is BleState.Connected -> {
                    Text("Connected to ${s.die.name}")
                    Text("Battery: ${s.batteryPercent?.let { "$it%" } ?: "—"}")
                    lastRoll?.let { LastRoll(it) }
                    DiceSection { kind, count -> scope.launch { links.setDie(kind, count) } }
                    inventory?.let { inv ->
                        ModesSection(inv) { enabled -> scope.launch { links.setEnabledModes(enabled) } }
                    }
                    OutlinedButton(onClick = { scope.launch { links.disconnect() } }) { Text("Disconnect") }
                }
                is BleState.Error -> {
                    Text(s.message)
                    Button(onClick = links::startScan) { Text("Try again") }
                }
            }

            HorizontalDivider()
            if (simulator == null) {
                SimulatorSection { address -> scope.launch { links.useSimulator(address) } }
            } else {
                TextButton(onClick = { scope.launch { links.useBluetooth() } }) {
                    Text("Use Bluetooth instead of the simulator")
                }
            }
        }
    }
}

@Composable
private fun LastRoll(roll: SignedRoll) {
    Column {
        Text("Last roll", style = MaterialTheme.typography.titleMedium)
        Text("${roll.total}", style = MaterialTheme.typography.displayMedium)
        Text("${roll.notation}  ${roll.values.joinToString()}  ·  #${roll.counter}")
    }
}

/**
 * What a throw rolls. Each change goes to the die straight away; the die
 * doesn't report its setup yet, so this starts from a single d20 (its default).
 */
@Composable
private fun DiceSection(onChange: (DieKind, Int) -> Unit) {
    var kind by rememberSaveable { mutableStateOf(DieKind.D20) }
    var count by rememberSaveable { mutableIntStateOf(1) }
    fun set(k: DieKind, c: Int) {
        kind = k
        count = c.coerceIn(1, k.maxCount)
        onChange(kind, count)
    }
    Column {
        Text("Dice", style = MaterialTheme.typography.titleMedium)
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            DieKind.entries.forEach { k ->
                FilterChip(selected = k == kind, onClick = { set(k, count) }, label = { Text(k.label) })
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = { set(kind, count - 1) }, enabled = count > 1) { Text("−") }
            Text("$count × ${kind.label}")
            OutlinedButton(onClick = { set(kind, count + 1) }, enabled = count < kind.maxCount) { Text("+") }
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

/**
 * Connect to the desktop simulator instead of a real die. From the iOS
 * simulator on the same Mac `localhost:3000` works; from a phone, use the
 * Mac's address and start the simulator with `HOST=0.0.0.0`.
 */
@Composable
private fun SimulatorSection(onConnect: (String) -> Unit) {
    var address by rememberSaveable { mutableStateOf("localhost:3000") }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("No die? Use the simulator", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(
            value = address,
            onValueChange = { address = it.trim() },
            label = { Text("Simulator address") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
        )
        OutlinedButton(onClick = { onConnect(address) }, enabled = address.isNotEmpty()) {
            Text("Connect to simulator")
        }
    }
}
