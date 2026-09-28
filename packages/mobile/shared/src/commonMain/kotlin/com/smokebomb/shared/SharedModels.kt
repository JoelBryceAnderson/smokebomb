package com.smokebomb.shared

/**
 * Kotlin mirror of the Rust `smokebomb-shared` crate. Keep field names and
 * semantics in sync with `packages/shared/src` and `docs/API.md`.
 */

/** BLE GATT service advertised by every Smokebomb (placeholder UUID). */
const val SMOKEBOMB_SERVICE_UUID = "5b0e0000-5b0e-4d1e-9a5e-736d6f6b6562"

enum class DieKind(val sides: Int) {
    D4(4), D6(6), D8(8), D10(10), D12(12), D20(20), D100(100);

    val label: String get() = "d$sides"

    companion object {
        fun fromSides(sides: Int): DieKind? = entries.firstOrNull { it.sides == sides }
    }
}

/** A roll exactly as signed by the die's ATECC608. Byte fields are lowercase hex. */
data class SignedRoll(
    val deviceSerial: String,
    val sessionId: String?,
    val counter: Long,
    val uptimeMs: Long,
    val dieSides: Int,
    val values: List<Int>,
    val prevHash: String,
    val signature: String,
) {
    val total: Int get() = values.sum()
    val die: DieKind? get() = DieKind.fromSides(dieSides)
    val notation: String get() = "${values.size}d$dieSides"
}

data class Device(
    val serial: String,
    val publicKey: String,
    val ownerName: String?,
    val firmwareVersion: String,
)

enum class ChainStatus { Linked, Genesis, Unknown, Broken }

/** Response of `POST /v1/rolls/verify`. */
data class VerifyResult(
    val valid: Boolean,
    val digest: String,
    val chain: ChainStatus,
    val reason: String?,
)
