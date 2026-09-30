package com.smokebomb.app.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * First run: owner name, then NFC-tap pairing (TODO) and BLE bonding.
 */
@Composable
fun OnboardingScreen(onFinished: () -> Unit) {
    var name by remember { mutableStateOf("") }
    Surface(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        ) {
            Text("Sugarcube", style = MaterialTheme.typography.displaySmall)
            Text("Name your die. It shows on the faces and on verified rolls.")
            OutlinedTextField(
                value = name,
                onValueChange = { name = it.take(24) },
                label = { Text("Owner name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Text(
                "Next: tap the die to the back of your phone to pair.",
                style = MaterialTheme.typography.bodySmall,
            )
            Button(onClick = onFinished, enabled = name.isNotBlank(), modifier = Modifier.fillMaxWidth()) {
                Text("Continue")
            }
        }
    }
}
