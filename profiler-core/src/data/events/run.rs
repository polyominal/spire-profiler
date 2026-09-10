//! Run lifecycle and meta: start/end (with the save+quit resume contract),
//! profile stamping, and the run-history screen selection.

use std::fmt;

use crate::data::persistence::{event_log, now_seconds, write_run_record};
use crate::data::state::{
    self, EndedRun, Label, ModelId, NetId, RunCharacter, RunOutcome, RunPlayer, RunSnapshot, STATE,
    Seed, caps,
};
use crate::{fail, marker};

pub fn set_run_meta(profile_id: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.ready() {
            return;
        }
        let profile_id = if profile_id < -1 {
            fail!("invalid run profile {profile_id}; clamping to unknown");
            -1
        } else {
            profile_id
        };
        state.run_profile = profile_id;
        event_log!("run meta: profile {profile_id}");
    });
}

/// `start_time` is the game's own `StartTime`; 0 means the read failed and
/// the identity stays unknown.
#[allow(
    clippy::too_many_lines,
    reason = "keep metadata admission before lifecycle mutation and persistence adapters"
)]
pub fn run_started(
    character_ids: &str,
    ascension: i32,
    game_mode: &str,
    seed: &str,
    continued: i32,
    net_ids: &str,
    start_time: i64,
) {
    if !STATE.with(|cell| cell.borrow().ready()) {
        fail!("run_started called before init");
        return;
    }
    let (Ok(character), Ok(game_mode_text), Ok(seed_text)) = (
        RunCharacter::try_from(character_ids),
        Label::try_from(game_mode),
        Seed::try_from(seed),
    ) else {
        fail!("run metadata exceeds text limits or contains NUL; run not started");
        return;
    };
    if character_ids.split(',').count() > caps::MAX_PLAYERS
        || (!net_ids.is_empty() && net_ids.split(',').count() > caps::MAX_PLAYERS)
        || character_ids
            .split(',')
            .any(|id| ModelId::try_from(id).is_err())
        || net_ids.split(',').any(|id| NetId::try_from(id).is_err())
        || (net_ids.is_empty() && character_ids.contains(','))
    {
        fail!("run roster exceeds identity limits; run not started");
        return;
    }
    if STATE.with(|cell| cell.borrow().finish_run.is_none()) {
        fail!("run finish staging is leased; run not started");
        return;
    }
    let mut players: [RunPlayer; caps::MAX_PLAYERS] = std::array::from_fn(|_| RunPlayer::default());
    let mut player_count = 0;
    if net_ids.is_empty() {
        if !character_ids.is_empty() {
            players[0] = RunPlayer {
                slot: 0,
                net_id: NetId::default(),
                character: ModelId::try_from(character_ids)
                    .expect("single-player identity was validated"),
            };
            player_count = 1;
        }
    } else {
        for (slot, (character, net_id)) in
            character_ids.split(',').zip(net_ids.split(',')).enumerate()
        {
            players[slot] = RunPlayer {
                slot: slot as u8,
                character: ModelId::try_from(character).expect("roster identities were validated"),
                net_id: NetId::try_from(net_id).expect("network identities were validated"),
            };
            player_count = slot + 1;
        }
    }
    if start_time < 0 {
        fail!("invalid run start time {start_time}; clamping to unknown");
    }
    let start_time = start_time.max(0);
    if let Some(ended) = FinishedRun::take(RunOutcome::Defeat) {
        // Closing with 0 would fabricate a win.
        record_ended_run(&ended);
    }
    let Some((seq, resumed_seq)) = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let paths = state.store_paths.as_ref()?;
        let resumed = (continued != 0)
            .then(|| {
                crate::data::run_history::continued_run_id(
                    &paths.runs_path,
                    &paths.runs_dir,
                    seed,
                    start_time,
                    state.run_profile,
                )
            })
            .flatten();
        let seq = resumed
            .or_else(|| crate::data::run_history::next_run_id(&paths.runs_path, &paths.runs_dir));
        let Some(seq) = seq else {
            state.run_cards.clear();
            state.run_turns = 0;
            state.run_combats = 0;
            state.player_filter = state::PlayerFilter::All;
            state.discard_combat();
            fail!("run ID allocation failed; run not started");
            return None;
        };
        let profile = state.run_profile;
        if !state.replace_run(
            &RunSnapshot {
                seq,
                character,
                ascension,
                game_mode: game_mode_text,
                seed: seed_text,
                profile,
                started_at: start_time,
            },
            &players[..player_count],
        ) {
            fail!("run storage unavailable; run not started");
            return None;
        }
        Some((seq, resumed))
    }) else {
        return;
    };
    // The accumulator rebuild re-borrows STATE, so the start line waits.
    if let Some(seq) = resumed_seq {
        let (combats, turns) = crate::data::persistence::rebuild_run_accumulator(seq);
        event_log!(
            "run {seq} resumed: {combats} combats ({turns} turns) merged from earlier sessions"
        );
    }
    event_log!(
        "run {seq} started: {} (ascension {ascension}, {game_mode}{})",
        character_ids,
        if continued != 0 { ", continued" } else { "" }
    );
    STATE.with(|cell| {
        if let Some(run) = cell.borrow().run_ctx.as_ref()
            && run.players.len() > 1
        {
            event_log!("{}", RosterLog(&run.players));
        }
    });
}

pub struct FinishedRun(Option<EndedRun>);

impl std::ops::Deref for FinishedRun {
    type Target = EndedRun;
    fn deref(&self) -> &EndedRun {
        self.0
            .as_ref()
            .expect("the lease owns run staging until Drop")
    }
}

impl Drop for FinishedRun {
    fn drop(&mut self) {
        STATE.with(|cell| cell.borrow_mut().finish_run = self.0.take());
    }
}

impl FinishedRun {
    pub(crate) fn take(outcome: RunOutcome) -> Option<Self> {
        STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            if !state.ready() {
                return None;
            }
            let state = &mut *state;
            let context = state.run_ctx.as_ref()?;
            let staged = state.finish_run.as_mut()?;
            staged.context.clone_from(context);
            staged.outcome = outcome;
            staged.ended_at = now_seconds();
            state.run_ctx.clear();
            state.finish_run.take().map(|value| Self(Some(value)))
        })
    }
}

fn record_ended_run(ended: &EndedRun) {
    if write_run_record(ended) {
        marker!(
            "run {} recorded ({})",
            ended.context.run.seq,
            ended.outcome.name()
        );
    }
}

struct RosterLog<'a>(&'a [RunPlayer]);

impl fmt::Display for RosterLog<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "  roster: {} players (", self.0.len())?;
        for (i, player) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(
                f,
                "slot {} = {} ({})",
                player.slot, player.net_id, player.character
            )?;
        }
        f.write_str(")")
    }
}

/// No record is written; taking the active run makes the next close_previous
/// a no-op instead of a spurious defeat.
pub fn run_suspended() {
    let suspended_seq = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.ready() {
            return None;
        }
        // A save+quit mid-combat discards that combat.
        // otherwise the restart's combat_started would flush it as
        // "interrupted".
        state.discard_combat();
        // Without a combat there is no avatar row, so a selected filter
        // would strand the run tab with no way back to All.
        state.player_filter = state::PlayerFilter::All;
        let seq = state.run_ctx.as_ref().map(|run| run.run.seq);
        state.run_ctx.clear();
        seq
    });
    if let Some(seq) = suspended_seq {
        // A stale screen-open flag would keep F8 routed to the run panel on
        // the main menu after the transition; only a real suspend drops it.
        crate::data::run_history::clear();
        event_log!("run {seq} suspended (save & quit); no record written");
    }
}

pub fn run_ended(outcome: RunOutcome) {
    if let Some(ended) = FinishedRun::take(outcome) {
        record_ended_run(&ended);
    }
}

/// The shim forwards the displayed run's seed, `StartTime`, and profile.
pub fn run_history_select(seed: &str, start_time: i64, profile: i32) {
    let initialized = STATE.with(|cell| cell.borrow().ready());
    let matched = if initialized {
        crate::data::run_history::select(seed, start_time, profile)
    } else {
        crate::data::run_history::clear();
        false
    };
    event_log!(
        "run history select: seed '{}' start {start_time} ({})",
        if seed.is_empty() { "(none)" } else { seed },
        if matched { "matched" } else { "no match" }
    );
}

pub fn run_history_clear() {
    crate::data::run_history::clear();
    event_log!("run history selection cleared");
}
