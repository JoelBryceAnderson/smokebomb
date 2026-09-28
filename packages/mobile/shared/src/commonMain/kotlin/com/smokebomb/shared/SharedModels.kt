package com.smokebomb.shared

/**
 * Kotlin mirror of the Rust `smokebomb-shared` crate. Keep field names and
 * semantics in sync with `packages/shared/src` and `docs/API.md`.
 */

/** BLE GATT service advertised by every Smokebomb (placeholder UUID). */
const val SMOKEBOMB_SERVICE_UUID = "5b0e0000-5b0e-4d1e-9a5e-736d6f6b6562"

const val MAX_DICE = 10
const val MAX_POT_DICE = 3

/** [wire] matches `DieKind::wire_name` in Rust and the REST API's `die` field. */
enum class DieKind(val wire: String, val sides: Int) {
    D4("d4", 4), D6("d6", 6), D8("d8", 8), D10("d10", 10), D12("d12", 12), D20("d20", 20), D100("d100", 100),

    /** Rolled and signed as a d6; see [PotFace]. */
    PASS_THE_POT("pass_the_pot", 6);

    val isNumeric: Boolean get() = this != PASS_THE_POT
    val maxCount: Int get() = if (isNumeric) MAX_DICE else MAX_POT_DICE
    val label: String get() = if (isNumeric) wire else "Pass the Pot"

    companion object {
        fun fromWire(wire: String): DieKind? = entries.firstOrNull { it.wire == wire }
    }
}

enum class PotFace(val glyph: String) {
    LEFT("←"), POT("P"), RIGHT("→"), KEEP("•");

    companion object {
        /** 1 → ←, 2 → P, 3 → →, 4–6 → •. */
        fun fromRaw(value: Int): PotFace = when (value) {
            1 -> LEFT
            2 -> POT
            3 -> RIGHT
            else -> KEEP
        }
    }
}

/** A roll exactly as signed by the die's ATECC608. Byte fields are lowercase hex. */
data class SignedRoll(
    val deviceSerial: String,
    val sessionId: String?,
    val counter: Long,
    val uptimeMs: Long,
    /** [DieKind.wire] name. */
    val die: String,
    /** Raw values; d6 values for Pass the Pot. */
    val values: List<Int>,
    val prevHash: String,
    val signature: String,
) {
    val dieKind: DieKind? get() = DieKind.fromWire(die)
    val total: Int get() = values.sum()

    /** Setup label as the die shows it: `d20`, `3d6`, `Pass the Pot ×2`. */
    val notation: String
        get() = when {
            dieKind == DieKind.PASS_THE_POT -> if (values.size > 1) "Pass the Pot ×${values.size}" else "Pass the Pot"
            values.size > 1 -> "${values.size}$die"
            else -> die
        }
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
