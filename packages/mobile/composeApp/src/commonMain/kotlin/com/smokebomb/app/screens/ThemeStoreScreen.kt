package com.smokebomb.app.screens

import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.smokebomb.app.network.ApiClient
import com.smokebomb.app.network.ThemeSummary

/** Browse smoke styles and animation packs. Purchase + BLE transfer TODO. */
@Composable
fun ThemeStoreScreen(api: ApiClient, modifier: Modifier = Modifier) {
    var themes by remember { mutableStateOf(emptyList<ThemeSummary>()) }
    LaunchedEffect(api) { themes = api.themes() }

    ScreenScaffold("Theme store", modifier) {
        LazyColumn {
            items(themes, key = { it.slug }) { theme ->
                ListItem(
                    headlineContent = { Text(theme.name) },
                    supportingContent = { Text(theme.description) },
                    trailingContent = {
                        Text(if (theme.priceCents == 0) "Free" else "$${theme.priceCents / 100}.${(theme.priceCents % 100).toString().padStart(2, '0')}")
                    },
                )
            }
        }
    }
}
