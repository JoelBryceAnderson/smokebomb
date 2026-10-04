package com.smokebomb.app.settings

import com.smokebomb.app.ble.PlatformContext
import platform.Foundation.NSUserDefaults

actual fun createAppSettings(context: PlatformContext): AppSettings = IosAppSettings()

private const val KEY_OWNER_NAME = "owner_name"

class IosAppSettings : AppSettings {
    private val defaults = NSUserDefaults.standardUserDefaults

    override var ownerName: String?
        get() = defaults.stringForKey(KEY_OWNER_NAME)
        set(value) = defaults.setObject(value, forKey = KEY_OWNER_NAME)
}
