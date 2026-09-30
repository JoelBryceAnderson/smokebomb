package com.smokebomb.app.history

import com.smokebomb.app.ble.BleManager
import com.smokebomb.app.ble.BleState
import com.smokebomb.shared.SignedRoll
import com.smokebomb.shared.mergeRolls
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Every roll the app has seen, newest first: the die's kept rolls, synced
 * each time it connects (`PhoneToDie::SyncHistory`), plus rolls pushed live
 * while connected. Kept in memory for now, so a relaunch starts empty until
 * the die connects again.
 */
class RollHistory(private val die: BleManager, scope: CoroutineScope) {
    private val _rolls = MutableStateFlow<List<SignedRoll>>(emptyList())
    val rolls: StateFlow<List<SignedRoll>> = _rolls.asStateFlow()

    private val _syncing = MutableStateFlow(false)
    val syncing: StateFlow<Boolean> = _syncing.asStateFlow()

    init {
        scope.launch { die.rolls.collect { roll -> add(listOf(roll)) } }
        scope.launch {
            die.state
                .map { it is BleState.Connected }
                .distinctUntilChanged()
                .collect { connected -> if (connected) sync() }
        }
    }

    /**
     * Pull everything the die kept. Asking from counter 0 each time (not
     * from the newest counter seen) keeps it right when the counter restarts,
     * as it does when the simulator restarts; the merge drops what we have.
     */
    suspend fun sync() {
        _syncing.value = true
        try {
            add(die.syncHistory(sinceCounter = 0))
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // Lost the connection mid-sync: the next connect syncs again.
        } finally {
            _syncing.value = false
        }
    }

    private fun add(rolls: Collection<SignedRoll>) = _rolls.update { mergeRolls(it, rolls) }
}
