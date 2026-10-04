package com.smokebomb.app.settings

import android.content.Context
import com.smokebomb.app.ble.PlatformContext

actual fun createAppSettings(context: PlatformContext): AppSettings = AndroidAppSettings(context)

private const val PREFS_NAME = "smokebomb"
private const val KEY_OWNER_NAME = "owner_name"

class AndroidAppSettings(context: Context) : AppSettings {
    private val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    override var ownerName: String?
        get() = prefs.getString(KEY_OWNER_NAME, null)
        set(value) = prefs.edit().putString(KEY_OWNER_NAME, value).apply()
}
