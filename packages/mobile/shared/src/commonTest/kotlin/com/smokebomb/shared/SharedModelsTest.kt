package com.smokebomb.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class SharedModelsTest {
    @Test
    fun dieKindFromWire() {
        assertEquals(DieKind.D20, DieKind.fromWire("d20"))
        assertEquals(DieKind.PASS_THE_POT, DieKind.fromWire("pass_the_pot"))
        assertNull(DieKind.fromWire("d7"))
    }

    @Test
    fun potFaces() {
        assertEquals(listOf("←", "P", "→", "•", "•", "•"), (1..6).map { PotFace.fromRaw(it).glyph })
        assertEquals(3, DieKind.PASS_THE_POT.maxCount)
    }

    @Test
    fun rollTotals() {
        val roll = SignedRoll("01235b0e00000000ee", null, 3, 1000, "d6", listOf(2, 5), "00", "00")
        assertEquals(7, roll.total)
        assertEquals("2d6", roll.notation)
        assertEquals("d20", roll.copy(die = "d20", values = listOf(4)).notation)
        assertEquals("Pass the Pot ×2", roll.copy(die = "pass_the_pot").notation)
    }

    @Test
    fun modeSets() {
        assertEquals(ModeId.PIG_TOSS, ModeId.fromWire("pig_toss"))
        assertNull(ModeId.fromWire("chess"))
        val all = ModeId.entries.toSet()
        assertEquals(0b1111, ModeId.maskOf(all))
        assertEquals(all, ModeId.setOf(0b1111))
        assertEquals(setOf(ModeId.DICE), ModeId.setOf(1 or (1 shl 15)), "unknown bits are skipped")
    }

    @Test
    fun inventoryRequestsKeepDiceAndDropUnlicensed() {
        val inv = Inventory(setOf(ModeId.DICE, ModeId.HOT_POTATO), setOf(ModeId.DICE), ModeId.DICE)
        assertEquals(setOf(ModeId.HOT_POTATO, ModeId.DICE), inv.request(setOf(ModeId.HOT_POTATO, ModeId.PIG_TOSS)))
        assertEquals(setOf(ModeId.DICE), inv.request(emptySet()))
    }
}
