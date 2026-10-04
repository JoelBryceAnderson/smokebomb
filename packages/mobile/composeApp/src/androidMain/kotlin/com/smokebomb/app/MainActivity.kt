package com.smokebomb.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.smokebomb.app.ble.createBleManager
import com.smokebomb.app.settings.createAppSettings

class MainActivity : ComponentActivity() {
    private val ble by lazy { createBleManager(applicationContext) }
    private val settings by lazy { createAppSettings(applicationContext) }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent { App(ble, settings) }
    }
}
