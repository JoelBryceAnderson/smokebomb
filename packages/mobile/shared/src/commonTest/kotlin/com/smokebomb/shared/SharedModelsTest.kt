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

    @Test
    fun mergingRollsKeepsEachOnceNewestFirst() {
        fun roll(counter: Long, sig: String = "s$counter") =
            SignedRoll("aa", null, counter, counter * 100, "d6", listOf(1), "00", sig)
        val live = listOf(roll(3))
        val synced = listOf(roll(1), roll(2), roll(3))
        val merged = mergeRolls(live, synced)
        assertEquals(listOf(3L, 2L, 1L), merged.map { it.counter })
        assertEquals(merged, mergeRolls(merged, synced), "nothing new")
        // A restarted simulator counts from 0 again: a different roll, kept.
        assertEquals(4, mergeRolls(merged, listOf(roll(1, sig = "other"))).size)
    }
}
