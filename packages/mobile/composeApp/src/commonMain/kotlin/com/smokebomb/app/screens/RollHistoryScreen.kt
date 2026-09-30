package com.smokebomb.app.screens

import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import com.smokebomb.app.history.RollHistory

/**
 * Every roll the app has seen from the die, newest first: synced when the
 * die connects, then live. Server history (`GET /v1/devices/{serial}/rolls`)
 * isn't merged in yet.
 */
@Composable
fun RollHistoryScreen(history: RollHistory, modifier: Modifier = Modifier) {
    val rolls by history.rolls.collectAsState()
    val syncing by history.syncing.collectAsState()

    ScreenScaffold("Roll history", modifier) {
        if (syncing) Text("Syncing with your die…", style = MaterialTheme.typography.bodySmall)
        if (rolls.isEmpty()) {
            Text(if (syncing) "" else "No rolls yet. Connect your die on the Die tab and throw it.")
            return@ScreenScaffold
        }
        LazyColumn {
            items(rolls, key = { "${it.deviceSerial}/${it.signature}" }) { roll ->
                ListItem(
                    headlineContent = { Text("${roll.total}", style = MaterialTheme.typography.titleLarge) },
                    supportingContent = { Text("${roll.notation}  ${roll.values.joinToString()}") },
                    trailingContent = { Text("#${roll.counter}") },
                )
                HorizontalDivider()
            }
        }
    }
}
