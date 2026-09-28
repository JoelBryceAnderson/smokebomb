package com.smokebomb.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class SharedModelsTest {
    @Test
    fun dieKindFromSides() {
        assertEquals(DieKind.D20, DieKind.fromSides(20))
        assertNull(DieKind.fromSides(7))
    }

    @Test
    fun rollTotals() {
        val roll = SignedRoll("01235b0e00000000ee", null, 3, 1000, 6, listOf(2, 5), "00", "00")
        assertEquals(7, roll.total)
        assertEquals("2d6", roll.notation)
    }
}
