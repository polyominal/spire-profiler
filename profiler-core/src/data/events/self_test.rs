//! The synthetic self-test pipeline (`--spire-profiler-self-test`).

use super::*;
use crate::ui::snapshot;

/// The host verifies the bridge without a real fight.
pub fn self_test() {
    if !STATE.with(|cell| cell.borrow().initialized) {
        return;
    }
    run_started("SELF_TEST_CHAR", 0, "Standard", "SELF_TEST_SEED", 0, "", 0);
    set_run_meta(1);
    combat_started("SELF_TEST", "test");
    // The off-by-default F8 state would hide the panel and skip `_draw`.
    crate::ui::panel::enable_for_selftest();
    // The context pops before the async OrbChanneled fires, so the channel
    // must fall back to last_source.
    context_begin("CRACKED_CORE", 1, 0);
    context_end();
    orb_channeled(1002, 0);
    turn_started();
    // The orb's turn-end tick attributes to ZAP.
    card_play_started("ZAP", 0, 1, 0, 0);
    orb_channeled(1001, 0);
    card_play_finished(0);
    orb_context_begin(1001, 0);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        ..DamageDealt::default()
    });
    // New turn: the boundary clears every slot's fallbacks.
    turn_started();
    // DEFEND gains block; the enemy's next hit is absorbed by it.
    card_play_started("DEFEND", 0, 1, 0, 0);
    block_gained(5, "DEFEND", 0, 0);
    card_play_finished(0);
    damage_dealt(DamageDealt {
        total: 8,
        unblocked: 3,
        blocked: 5,
        to_player: 1,
        ..DamageDealt::default()
    });
    // Potion use becomes a fallback source.
    potion_used("FIRE_POTION", 0);
    // BASH hits an enemy.
    card_play_started("BASH", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 3,
        blocked: 3,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    // The SHIV's later play credits the generator, not a SHIV row.
    card_play_started("CLOAK_AND_DAGGER", 0, 1, 0, 0);
    card_generated(5001, "", 0, 0);
    card_play_finished(0);
    // The first trigger credits the CHANNELING source, the second the card.
    card_play_started("DUALCAST", 0, 1, 0, 0);
    orb_context_begin(1002, 0);
    damage_dealt(DamageDealt {
        total: 8,
        blocked: 8,
        ..DamageDealt::default()
    });
    orb_context_begin(1002, 0);
    damage_dealt(DamageDealt {
        total: 8,
        blocked: 8,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("SHIV", 0, 1, 5001, 0);
    card_play_finished(0);
    // FurnacePower forges 2 damage onto Sovereign Blade.
    forge("FURNACE_POWER", 2, 2, 0);
    combat_ended();
    // Wire code 0 = victory; the headless gate greps "(victory)".
    run_ended(0);
    snapshot::chart_self_test();
}
