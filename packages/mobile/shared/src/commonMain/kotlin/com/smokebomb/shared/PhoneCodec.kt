package com.smokebomb.shared

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long

/**
 * The die's messages (`packages/shared/src/protocol.rs`) as JSON, the way
 * the simulator's `/phone` WebSocket carries them: serde's default shape,
 * e.g. `"GetInventory"`, `{"SetEnabledModes":13}`,
 * `{"Inventory":{"licensed":15,"enabled":13,"active":"Dice"}}`. Real BLE
 * will carry the same messages in postcard.
 */
object PhoneCodec {
    fun getInventory(): String = "\"GetInventory\""

    fun setEnabledModes(modes: Set<ModeId>): String = """{"SetEnabledModes":${ModeId.maskOf(modes)}}"""

    fun setDie(kind: DieKind, count: Int): String = """{"SetDie":{"kind":"${kind.variant}","count":$count}}"""

    /** Ask for the die's kept rolls from [sinceCounter] on; they come back as [DieMessage.HistoryItem]s. */
    fun syncHistory(sinceCounter: Long): String = """{"SyncHistory":{"since_counter":$sinceCounter}}"""

    /** A message from the die, or null for one this app doesn't know. */
    fun decode(text: String): DieMessage? = runCatching {
        val obj = Json.parseToJsonElement(text) as? JsonObject ?: return null
        val (name, body) = obj.entries.singleOrNull() ?: return null
        when (name) {
            "Hello" -> {
                val b = body.jsonObject
                DieMessage.Hello(
                    firmwareVersion = b.getValue("firmware_version").jsonArray.joinToString(".") { it.jsonPrimitive.content },
                    batteryPercent = b.getValue("battery_percent").jsonPrimitive.int,
                )
            }
            "Inventory" -> {
                val b = body.jsonObject
                DieMessage.Inventory(
                    Inventory(
                        licensed = ModeId.setOf(b.getValue("licensed").jsonPrimitive.int),
                        enabled = ModeId.setOf(b.getValue("enabled").jsonPrimitive.int),
                        active = ModeId.fromVariant(b.getValue("active").jsonPrimitive.content) ?: return null,
                    ),
                )
            }
            "Roll" -> DieMessage.Roll(roll(body.jsonObject) ?: return null)
            "HistoryItem" -> DieMessage.HistoryItem(
                if (body is JsonNull) null else roll(body.jsonObject) ?: return null,
            )
            else -> null
        }
    }.getOrNull()

    private fun roll(o: JsonObject): SignedRoll? {
        val r = o.getValue("record").jsonObject
        val session = hex(r.getValue("session"))
        return SignedRoll(
            deviceSerial = hex(r.getValue("device")),
            sessionId = session.takeUnless { s -> s.all { it == '0' } },
            counter = r.getValue("counter").jsonPrimitive.long,
            uptimeMs = r.getValue("uptime_ms").jsonPrimitive.long,
            die = DieKind.fromVariant(r.getValue("die").jsonPrimitive.content)?.wire ?: return null,
            values = r.getValue("values").jsonArray.map { it.jsonPrimitive.int },
            prevHash = hex(r.getValue("prev_hash")),
            signature = hex(o.getValue("signature")),
        )
    }

    /** A serde byte array (a JSON array of numbers) as lowercase hex. */
    private fun hex(e: JsonElement): String = (e as JsonArray).joinToString("") {
        (it as JsonPrimitive).int.toString(16).padStart(2, '0')
    }
}

/** A message from the die (`DieToPhone`). */
sealed interface DieMessage {
    data class Hello(val firmwareVersion: String, val batteryPercent: Int) : DieMessage
    data class Inventory(val inventory: com.smokebomb.shared.Inventory) : DieMessage
    data class Roll(val roll: SignedRoll) : DieMessage

    /** One kept roll in answer to a history sync; null marks the end. */
    data class HistoryItem(val roll: SignedRoll?) : DieMessage
}
