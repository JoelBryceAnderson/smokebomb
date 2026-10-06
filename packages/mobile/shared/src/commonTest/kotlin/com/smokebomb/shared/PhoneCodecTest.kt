package com.smokebomb.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/** JSON shapes pinned by `phone.rs` tests in the simulator. */
class PhoneCodecTest {
    @Test
    fun variants() {
        assertEquals(listOf("Dice", "PassThePot", "HotPotato", "PigToss", "SugarRush"), ModeId.entries.map { it.variant })
        assertEquals("D100", DieKind.D100.variant)
        assertEquals(DieKind.PASS_THE_POT, DieKind.fromVariant("PassThePot"))
    }

    @Test
    fun encodes() {
        assertEquals("\"GetInventory\"", PhoneCodec.getInventory())
        assertEquals("""{"SetEnabledModes":5}""", PhoneCodec.setEnabledModes(setOf(ModeId.DICE, ModeId.HOT_POTATO)))
        assertEquals("""{"SetDie":{"kind":"D6","count":3}}""", PhoneCodec.setDie(DieKind.D6, 3))
    }

    @Test
    fun decodesHelloAndInventory() {
        assertEquals(
            DieMessage.Hello("0.1.0", 78),
            PhoneCodec.decode("""{"Hello":{"firmware_version":[0,1,0],"battery_percent":78}}"""),
        )
        assertEquals(
            DieMessage.Inventory(Inventory(ModeId.entries.toSet(), setOf(ModeId.DICE, ModeId.HOT_POTATO), ModeId.DICE)),
            PhoneCodec.decode("""{"Inventory":{"licensed":31,"enabled":5,"active":"Dice"}}"""),
        )
    }

    @Test
    fun decodesARoll() {
        val zeros = List(32) { 0 }.joinToString(",")
        val sig = List(64) { 255 }.joinToString(",")
        val json = """{"Roll":{"record":{"device":[1,35,91,14,0,0,0,0,238],"session":[${List(16) { 0 }.joinToString(",")}],""" +
            """"counter":4,"uptime_ms":24184,"die":"D6","values":[3,5],"prev_hash":[$zeros]},"signature":[$sig]}}"""
        val roll = (PhoneCodec.decode(json) as DieMessage.Roll).roll
        assertEquals("01235b0e00000000ee", roll.deviceSerial)
        assertNull(roll.sessionId)
        assertEquals(4L, roll.counter)
        assertEquals("2d6", roll.notation)
        assertEquals(8, roll.total)
        assertEquals("ff".repeat(64), roll.signature)
    }

    @Test
    fun historySync() {
        assertEquals("""{"SyncHistory":{"since_counter":0}}""", PhoneCodec.syncHistory(0))
        assertEquals(DieMessage.HistoryItem(null), PhoneCodec.decode("""{"HistoryItem":null}"""))
        val zeros = List(32) { 0 }.joinToString(",")
        val json = """{"HistoryItem":{"record":{"device":[1,1,1,1,1,1,1,1,1],"session":[${List(16) { 0 }.joinToString(",")}],""" +
            """"counter":2,"uptime_ms":5,"die":"D20","values":[17],"prev_hash":[$zeros]},"signature":[${List(64) { 1 }.joinToString(",")}]}}"""
        val item = PhoneCodec.decode(json) as DieMessage.HistoryItem
        assertEquals(17, item.roll?.total)
    }

    @Test
    fun ignoresWhatItDoesNotKnow() {
        assertNull(PhoneCodec.decode("""{"PublicKey":[1,2]}"""))
        assertNull(PhoneCodec.decode("not json"))
        assertNull(PhoneCodec.decode("""{"Inventory":{"licensed":15,"enabled":5,"active":"Chess"}}"""))
    }
}
