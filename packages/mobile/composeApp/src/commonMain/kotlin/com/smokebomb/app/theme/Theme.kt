package com.smokebomb.app.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val Colors = darkColorScheme(
    primary = Color(0xFFD9774B),
    onPrimary = Color(0xFF111111),
    background = Color(0xFF0D0D10),
    surface = Color(0xFF16161B),
    surfaceVariant = Color(0xFF1F1F26),
)

@Composable
fun SmokebombTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = Colors, content = content)
}
