package com.smokebomb.app.settings

import com.smokebomb.app.ble.PlatformContext

/**
 * Small values kept across launches. Each platform backs it with its own
 * store through [createAppSettings] (SharedPreferences on Android,
 * NSUserDefaults on iOS).
 */
interface AppSettings {
    /** Owner name typed during onboarding; null until one has been saved. */
    var ownerName: String?
}

expect fun createAppSettings(context: PlatformContext): AppSettings
