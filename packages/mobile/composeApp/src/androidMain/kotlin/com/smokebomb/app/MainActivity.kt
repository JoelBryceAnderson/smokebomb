package com.smokebomb.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.smokebomb.app.ble.createBleManager

class MainActivity : ComponentActivity() {
    private val ble by lazy { createBleManager(applicationContext) }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent { App(ble) }
    }
}
