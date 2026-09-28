package com.smokebomb.app.screens

import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.smokebomb.app.network.ApiClient
import com.smokebomb.shared.SignedRoll

/** Simulator die serial; replaced by the paired device once onboarding stores it. */
private const val DEMO_SERIAL = "01235b0e00000000ee"

/**
 * Roll history merged from the die (BLE sync) and the server. Currently reads
 * the stub API only.
 */
@Composable
fun RollHistoryScreen(api: ApiClient, modifier: Modifier = Modifier) {
    var rolls by remember { mutableStateOf<List<SignedRoll>?>(null) }
    LaunchedEffect(api) { rolls = api.rollHistory(DEMO_SERIAL) }

    ScreenScaffold("Roll history", modifier) {
        val list = rolls
        if (list == null) {
            Text("Loading…")
            return@ScreenScaffold
        }
        LazyColumn {
            items(list, key = { it.counter }) { roll ->
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
