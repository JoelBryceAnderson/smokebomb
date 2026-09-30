package com.smokebomb.app.network

import com.smokebomb.shared.ChainStatus
import com.smokebomb.shared.SignedRoll
import com.smokebomb.shared.VerifyResult

/**
 * Sugarcube REST API (`docs/API.md`). Stubbed with canned data until the
 * HTTP client (Ktor) is added.
 */
interface ApiClient {
    suspend fun verifyRoll(roll: SignedRoll): VerifyResult
    suspend fun rollHistory(deviceSerial: String): List<SignedRoll>
    suspend fun themes(): List<ThemeSummary>
}

data class ThemeSummary(val slug: String, val name: String, val description: String, val priceCents: Int)

class StubApiClient : ApiClient {
    override suspend fun verifyRoll(roll: SignedRoll) =
        VerifyResult(valid = true, digest = "", chain = ChainStatus.Unknown, reason = "stub")

    override suspend fun rollHistory(deviceSerial: String) = List(8) { i ->
        SignedRoll(
            deviceSerial = deviceSerial,
            sessionId = null,
            counter = 8L - i,
            uptimeMs = 60_000L * (8 - i),
            die = "d20",
            values = listOf((i * 7) % 20 + 1),
            prevHash = "00".repeat(32),
            signature = "00".repeat(64),
        )
    }

    override suspend fun themes() = listOf(
        ThemeSummary("classic-smoke", "Classic Smoke", "The default grey plume.", 0),
        ThemeSummary("ember", "Ember", "Warm sparks that burst on a max roll.", 299),
        ThemeSummary("void", "Void", "Ink-black smoke with a violet rim.", 299),
    )
}
