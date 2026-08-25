//! Run lifecycle and meta: start/end (with the save+quit resume contract),
//! profile stamping, and the run-history screen selection.

use crate::data::persistence::{append_log, now_seconds, write_run_record};
use crate::data::state::{self, RunOutcome, RunPlayer, STATE, caps};
use crate::{fail, marker};

pub fn set_run_meta(profile_id: i32) {
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized {
            return Vec::new();
        }
        state.run_profile = profile_id;
        vec![format!("run meta: profile {profile_id}\n")]
    });
    for line in log_lines {
        append_log(line);
    }
}

/// `start_time` is the game's own `StartTime`; 0 means the read failed and
/// the core stamps its own clock.
pub fn run_started(
    character_ids: &str,
    ascension: i32,
    game_mode: &str,
    seed: &str,
    continued: i32,
    net_ids: &str,
    start_time: i64,
) {
    if !STATE.with(|cell| cell.borrow().initialized) {
        fail!("run_started called before init");
        return;
    }
    // A previous unclosed run is closed out as a loss.
    let close_previous = STATE.with(|cell| {
        let state = cell.borrow();
        state.run_ctx.active
    });
    if close_previous {
        // Closing with 0 would fabricate a win.
        run_ended(RunOutcome::Defeat);
    }
    let (log_lines, resumed_seq, roster) = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // A resumed run rejoins its earlier fragments by seed.
        let resumed = (continued != 0)
            .then(|| crate::data::run_history::continued_run_id(&state.runs_dir_full, seed))
            .flatten();
        // The run id comes from the store, not a session counter.
        let seq = resumed.unwrap_or_else(|| {
            crate::data::run_history::next_run_id(&state.runs_path_full, &state.runs_dir_full)
        });
        // Fresh run: the accumulator starts over. The player filter
        // resets too — the avatar row only ever lists the current
        // roster, so a slot from a previous run's roster would strand
        // an empty chart with no way back to All.
        state.run_cards.clear();
        state.run_turns = 0;
        state.run_combats = 0;
        state.run_seq_accumulated = seq;
        state.player_filter = state::PlayerFilter::All;
        let roster = parse_roster(character_ids, net_ids);
        state.run_ctx = state::RunContext {
            active: true,
            seq,
            character: character_ids.to_owned(),
            ascension,
            game_mode: game_mode.to_owned(),
            seed: seed.to_owned(),
            started_at: if start_time > 0 {
                start_time
            } else {
                // The session clock preserves the old behavior.
                now_seconds()
            },
            players: roster.clone(),
            ..state::RunContext::default()
        };
        (
            vec![format!(
                "run {seq} started: {} (ascension {ascension}, {game_mode}{})\n",
                character_ids,
                if continued != 0 { ", continued" } else { "" }
            )],
            resumed,
            roster,
        )
    });
    // The borrow above must release first.
    if let Some(seq) = resumed_seq {
        let (combats, turns) = crate::data::persistence::rebuild_run_accumulator(seq);
        append_log(format!(
            "run {seq} resumed: {combats} combats ({turns} turns) merged from earlier sessions\n"
        ));
    }
    for line in log_lines {
        append_log(line);
    }
    append_log(roster_log_line(&roster));
}

fn roster_log_line(roster: &[RunPlayer]) -> String {
    if roster.len() <= 1 {
        return String::new();
    }
    format!(
        "  roster: {} players ({})\n",
        roster.len(),
        roster
            .iter()
            .map(|p| format!("slot {} = {} ({})", p.slot, p.net_id, p.character))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// `net_ids` pairs positionally with `character_ids`; mismatches truncate.
fn parse_roster(character_ids: &str, net_ids: &str) -> Vec<RunPlayer> {
    if net_ids.is_empty() {
        if character_ids.is_empty() {
            return Vec::new();
        }
        return vec![RunPlayer {
            slot: 0,
            net_id: String::new(),
            character: character_ids.to_owned(),
        }];
    }
    character_ids
        .split(',')
        .zip(net_ids.split(','))
        .take(caps::MAX_PLAYERS)
        .enumerate()
        .map(|(slot, (character, net_id))| RunPlayer {
            slot: slot as u8,
            net_id: net_id.to_owned(),
            character: character.to_owned(),
        })
        .collect()
}

/// No record is written; clearing `active` makes the next close_previous a
/// no-op instead of a spurious defeat.
pub fn run_suspended() {
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || !state.run_ctx.active {
            return Vec::new();
        }
        // A save+quit mid-combat discards that combat.
        // otherwise the restart's combat_started would flush it as
        // "interrupted".
        state.current = None;
        // Without a combat there is no avatar row, so a selected filter
        // would strand the run tab with no way back to All.
        state.player_filter = state::PlayerFilter::All;
        let seq = state.run_ctx.seq;
        state.run_ctx.active = false;
        vec![format!(
            "run {seq} suspended (save & quit); no record written\n"
        )]
    });
    if !log_lines.is_empty() {
        // A stale screen-open flag would keep F8 routed to the run panel on
        // the main menu after the transition; only a real suspend drops it.
        crate::data::run_history::clear();
    }
    for line in log_lines {
        append_log(line);
    }
}

pub fn run_ended(outcome: RunOutcome) {
    let active = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || !state.run_ctx.active {
            return false;
        }
        state.run_ctx.outcome = outcome;
        // Abandonment carries no separate timestamp: `ended_at` IS the
        // abandon moment.
        state.run_ctx.ended_at = now_seconds();
        true
    });
    if !active {
        return;
    }
    write_run_record();
    STATE.with(|cell| cell.borrow_mut().run_ctx.active = false);
    let (seq, outcome) = STATE.with(|cell| {
        let state = cell.borrow();
        (state.run_ctx.seq, state.run_ctx.outcome)
    });
    marker!("run {seq} recorded ({})", outcome.name());
}

/// The shim forwards the displayed run's seed, `StartTime`, and profile.
pub fn run_history_select(seed: &str, start_time: i64, profile: i32) {
    let initialized = STATE.with(|cell| cell.borrow().initialized);
    let matched = if initialized {
        crate::data::run_history::select(seed, start_time, profile)
    } else {
        crate::data::run_history::clear();
        false
    };
    append_log(format!(
        "run history select: seed '{}' start {start_time} ({})\n",
        if seed.is_empty() { "(none)" } else { seed },
        if matched { "matched" } else { "no match" }
    ));
}

pub fn run_history_clear() {
    crate::data::run_history::clear();
    append_log("run history selection cleared\n".to_owned());
}
