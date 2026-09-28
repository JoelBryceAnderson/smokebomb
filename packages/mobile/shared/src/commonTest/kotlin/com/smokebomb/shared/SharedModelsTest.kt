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
}
