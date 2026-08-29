//! Run-history integration: matching the game's displayed run onto the
//! profiler's records, the structured view the panel renders, and the run
//! close lifecycle.
//!
//! # Screen wiring
//!
//! The game's `NRunHistory` screen shows one `RunHistory` at a time. The
//! shim postfixes its `DisplayRun` to forward the displayed identity (seed,
//! `StartTime`, and profile) and to attach the panel; it PREFIXES
//! `OnSubmenuOpened` with a clear (the body's initial `DisplayRun` runs
//! inside the original method, so a postfix clear would blank the
//! just-selected view), and it postfixes the screen's close with another
//! clear, filtered to `NRunHistory` so a submenu closing on top never
//! blanks a still-displayed run.
//!
//! This module maps the identity onto the cached store and hands the panel
//! a structured view in-process; no match is a typed [`RunSelection::Empty`],
//! never a crash. The panel renders the selected view as the shared
//! two-section chart (its chrome-less build) under a title/header/meta
//! band, or the empty-state notice when the selection is Empty.
//!
//! # The join key
//!
//! The game's run id IS `StartTime` (Unix seconds): its history store
//! names one file per run `{StartTime}.run`, and a resumed run keeps the
//! ORIGINAL run's start. The shim reads the same `RunManager._startTime`
//! field and forwards it; the core stamps it verbatim as the record's
//! `started_at`. Matching is exact equality on seed + time, nothing else:
//!
//! 1. Same seed AND `started_at == start_time` — the only runs.jsonl match. Both values are
//!    identical by provenance (the seed comes from RunRngSet.StringSeed on both sides), so equality
//!    can never pair the wrong run and any fuzz could. The seed disambiguates the same-second
//!    collision the game's own `{StartTime}.run` storage loses to. When the shim's reflection read
//!    fails it sends 0 and the core falls back to its clock; such a record selects Empty — matching
//!    never guesses.
//! 2. Combats fallback: a run with seed-stamped combats but no runs.jsonl entry (save & quit, a
//!    crash, or a build whose close hook never fired) selects a synthesized view instead of the
//!    empty state. The seed-matching combats group by run seq, and the group whose earliest combat
//!    start is closest to the displayed `StartTime` within [`COMBAT_GROUP_WINDOW_SECS`] (±300 s)
//!    wins. The window is unavoidable — a combat's timestamp is its own start, never the run's —
//!    but only ever picks among same-seed replays. The view's result reads "Unfinished": victory is
//!    unknown, never a false "Defeat".
//! 3. Neither entry nor combats: [`RunSelection::Empty`].
//!
//! The profile id pre-filters the `runs.jsonl` match when one is known
//! (≥ 0) — the game's screen is per-profile; combats carry no profile,
//! so the fallback matches on seed alone.
//!
//! # The run close lifecycle
//!
//! * Close on `RunManager.OnEnded` — victory, all-dead defeat, and abandon all funnel through it,
//!   so the record closes for every finished run regardless of upload preferences, the full-screen
//!   setting, or Steam mode.
//! * Suspend on save & exit: `RunManager.CleanUp` forwards `spire_profiler_run_suspended`, so a
//!   suspended run writes no record and the next continue no longer closes a spurious defeat. A
//!   combat interrupted by save+quit is never persisted.
//! * Suspend on disconnect: the host-quit path (`RunManager.LocalPlayerDisconnected`) suspends
//!   instead of closing — the reason cannot distinguish save&quit from quit-without-save, and a
//!   resume rejoins the same run id by seed.
//! * Runs that still never closed select the synthesized "Unfinished" view.
//!
//! # The store behind the view
//!
//! The cache parses `runs.jsonl` and the whole combat store once per
//! data-dir path pair per process; selections then match in memory. A
//! combat or run written mid-session invalidates the cache, so the next
//! selection re-reads. The view's roll-ups recompute from the combat store
//! every time — never stored aggregates — TEAM-merged so two players'
//! same-id cards fold into one row, with per-player roll-ups beside them
//! for the panel's avatar toggle.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::data::persistence::{
    CardStatKey, card_stat_from_rec, load_combat_docs_from, parse_combat_docs, upsert_card_stat,
};
use crate::data::records::{CombatRec, PlayerRec};
use crate::data::state::{CardStat, PlayerFilter, RunOutcome, STATE, TEAM_SLOT};

/// Generous for the run-start → first-combat gap, yet two same-seed
/// replays played further apart never merge.
const COMBAT_GROUP_WINDOW_SECS: i64 = 300;

/// Roll-ups are undeclared on purpose: the view recomputes them.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RunEntry {
    pub run_id: u32,
    pub profile: i32,
    pub character: String,
    pub ascension: i32,
    pub game_mode: String,
    /// The view's result label derives from it.
    pub outcome: RunOutcome,
    pub seed: String,
    /// Epoch seconds; matching is direct integer arithmetic.
    pub started_at: i64,
    pub ended_at: i64,
    /// Empty on pre-roster records.
    pub players: Vec<PlayerRec>,
}

impl Default for RunEntry {
    /// -1 means "never reported".
    fn default() -> Self {
        RunEntry {
            run_id: 0,
            profile: -1,
            character: String::new(),
            ascension: -1,
            game_mode: String::new(),
            outcome: RunOutcome::Defeat,
            seed: String::new(),
            started_at: 0,
            ended_at: 0,
            players: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatView {
    pub seq: u32,
    pub encounter: String,
    pub result: String,
    pub damage_dealt: i64,
    pub damage_taken: i64,
    pub turns: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerRollup {
    pub slot: u8,
    pub character: String,
    pub cards: Vec<CardStat>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunSummaryView {
    pub run_id: u32,
    pub profile: i32,
    pub character: String,
    pub ascension: i32,
    pub game_mode: String,
    /// None on the combats-only fallback.
    pub outcome: Option<RunOutcome>,
    /// "Unfinished" for the fallback: the truth is unknown.
    pub result: String,
    pub seed: String,
    pub started_at: i64,
    pub ended_at: i64,
    /// Empty on pre-roster records.
    pub players: Vec<PlayerRec>,
    pub combats: Vec<CombatView>,
    /// TEAM-merged; rows keep first-seen order.
    pub rollup: Vec<CardStat>,
    /// Keyed by roster slot.
    pub player_rollups: Vec<PlayerRollup>,
}

#[derive(Clone, Debug)]
pub enum RunSelection {
    Selected(Box<RunSummaryView>),
    Empty,
}

struct Cache {
    runs_path: PathBuf,
    runs_dir: PathBuf,
    runs: Vec<RunEntry>,
    combats: Vec<CombatRec>,
}

thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
    static SELECTION: RefCell<Option<RunSummaryView>> = const { RefCell::new(None) };
    /// Distinct from `SELECTION`: an open screen with no record must still
    /// render its empty-state notice.
    static SCREEN_OPEN: Cell<bool> = const { Cell::new(false) };
    /// Deliberately separate from the live `State::player_filter`: persists
    /// across selections so one player can be compared across runs, heals
    /// against the selected view's roster, and never touches live state.
    static RUN_FILTER: Cell<PlayerFilter> = const { Cell::new(PlayerFilter::All) };
}

fn store_paths() -> (PathBuf, PathBuf) {
    STATE.with(|s| {
        let st = s.borrow();
        (st.runs_path_full.clone(), st.runs_dir_full.clone())
    })
}

/// One JSON object per line; one bad line never hides the rest.
fn load_runs(path: &Path) -> Vec<RunEntry> {
    let Some(content) = crate::data::persistence::read_file(path) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RunEntry>(line) {
            Ok(run) => runs.push(run),
            Err(err) => crate::fail!("cannot parse a runs.jsonl line: {err}"),
        }
    }
    runs
}

fn load_combats(dir: &Path) -> Vec<CombatRec> {
    parse_combat_docs(&load_combat_docs_from(dir))
}

/// The `seq` of the most recent combat record carrying `seed`; O(history)
/// — the seed join has no index.
pub(crate) fn continued_run_id(runs_dir: &Path, seed: &str) -> Option<u32> {
    if seed.is_empty() {
        return None;
    }
    load_combats(runs_dir)
        .iter()
        .filter_map(|combat| combat.run.as_ref())
        .filter(|run| run.seed == seed)
        .map(|run| run.seq)
        .next_back()
}

/// An abandoned run leaves its directory but no entry, so the directory
/// name reserves the id; `runs/0/` never counts.
pub(crate) fn next_run_id(runs_path: &Path, runs_dir: &Path) -> u32 {
    let runs_max = load_runs(runs_path)
        .iter()
        .map(|entry| entry.run_id)
        .max()
        .unwrap_or(0);
    let dirs_max = std::fs::read_dir(runs_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    runs_max.max(dirs_max).saturating_add(1)
}

fn ensure_loaded() {
    let (runs_path, runs_dir) = store_paths();
    let hit = CACHE.with(|cell| {
        let cache = cell.borrow();
        cache
            .as_ref()
            .is_some_and(|c| c.runs_path == runs_path && c.runs_dir == runs_dir)
    });
    if hit {
        return;
    }
    let runs = load_runs(&runs_path);
    let combats = load_combats(&runs_dir);
    CACHE.with(|cell| {
        *cell.borrow_mut() = Some(Cache {
            runs_path,
            runs_dir,
            runs,
            combats,
        });
    });
}

pub fn invalidate() {
    CACHE.with(|cell| *cell.borrow_mut() = None);
}

/// Same seed AND `started_at == start_time`, profile pre-filtered when
/// known.
fn match_run<'a>(
    runs: &'a [RunEntry],
    seed: &str,
    start_time: i64,
    profile: i32,
) -> Option<&'a RunEntry> {
    runs.iter().find(|r| {
        (profile < 0 || r.profile == profile) && r.seed == seed && r.started_at == start_time
    })
}

fn build_view(entry: &RunEntry, combats: &[CombatRec]) -> RunSummaryView {
    let mut view = RunSummaryView {
        run_id: entry.run_id,
        profile: entry.profile,
        character: entry.character.clone(),
        ascension: entry.ascension,
        game_mode: entry.game_mode.clone(),
        outcome: Some(entry.outcome),
        result: match entry.outcome {
            RunOutcome::Victory => "Victory",
            RunOutcome::Abandoned => "Abandoned",
            RunOutcome::Defeat => "Defeat",
        }
        .to_owned(),
        seed: entry.seed.clone(),
        started_at: entry.started_at,
        ended_at: entry.ended_at,
        players: entry.players.clone(),
        ..RunSummaryView::default()
    };
    for combat in combats {
        let Some(run) = &combat.run else { continue };
        if run.seq != entry.run_id {
            continue;
        }
        view.combats.push(CombatView {
            seq: combat.combat_id,
            encounter: combat.encounter_id.clone(),
            result: combat.result.clone(),
            damage_dealt: combat.cards.iter().map(|c| c.damage_dealt).sum(),
            damage_taken: combat.damage_received,
            turns: combat.turns,
        });
    }
    view.rollup = roll_up_cards(combats, entry.run_id);
    view.player_rollups = build_player_rollups(entry, combats);
    view
}

/// TEAM-merged; rows keep first-seen order.
fn roll_up_cards(combats: &[CombatRec], run_id: u32) -> Vec<CardStat> {
    let mut rollup: Vec<CardStat> = Vec::new();
    for combat in combats {
        let Some(run) = &combat.run else { continue };
        if run.seq != run_id {
            continue;
        }
        for rec in &combat.cards {
            let mut row = card_stat_from_rec(rec);
            row.player = TEAM_SLOT;
            upsert_card_stat(&mut rollup, &row, CardStatKey::TeamMerged);
        }
    }
    rollup
}

/// Merging same-id rows within that slot only.
fn roll_up_cards_for_slot(combats: &[CombatRec], run_id: u32, slot: u8) -> Vec<CardStat> {
    let mut rollup: Vec<CardStat> = Vec::new();
    for combat in combats {
        let Some(run) = &combat.run else { continue };
        if run.seq != run_id {
            continue;
        }
        for rec in &combat.cards {
            let src = card_stat_from_rec(rec);
            if src.player != slot {
                continue;
            }
            upsert_card_stat(&mut rollup, &src, CardStatKey::TeamMerged);
        }
    }
    rollup
}

/// Players with no rows still get an empty entry.
fn build_player_rollups(entry: &RunEntry, combats: &[CombatRec]) -> Vec<PlayerRollup> {
    entry
        .players
        .iter()
        .map(|player| PlayerRollup {
            slot: player.slot,
            character: player.character.clone(),
            cards: roll_up_cards_for_slot(combats, entry.run_id, player.slot),
        })
        .collect()
}

fn fallback_from_combats(cache: &Cache, seed: &str, profile: i32, start_time: i64) -> RunSelection {
    // An empty seed would merge unrelated legacy records.
    if seed.is_empty() {
        return RunSelection::Empty;
    }
    let mut groups: Vec<(u32, i64)> = Vec::new();
    for combat in cache
        .combats
        .iter()
        .filter(|combat| combat.run.as_ref().is_some_and(|run| run.seed == seed))
    {
        let seq = combat
            .run
            .as_ref()
            .expect("filtered on a present run record")
            .seq;
        match groups.iter_mut().find(|(group_seq, _)| *group_seq == seq) {
            Some((_, group_start)) => *group_start = (*group_start).min(combat.started_at),
            None => groups.push((seq, combat.started_at)),
        }
    }
    // Closest group start within the window; two replays never merge.
    let Some((run_seq, group_start)) = groups
        .into_iter()
        .filter(|(_, start)| (start - start_time).abs() <= COMBAT_GROUP_WINDOW_SECS)
        .min_by_key(|(_, start)| (start - start_time).abs())
    else {
        return RunSelection::Empty;
    };
    let latest = cache
        .combats
        .iter()
        .rfind(|combat| combat.run.as_ref().is_some_and(|run| run.seq == run_seq))
        .expect("the group came from a combat record");
    let run = latest
        .run
        .as_ref()
        .expect("filtered on a present run record");
    let group_end = cache
        .combats
        .iter()
        .filter(|combat| combat.run.as_ref().is_some_and(|run| run.seq == run_seq))
        .map(|combat| combat.started_at)
        .max()
        .unwrap_or(group_start);
    let entry = RunEntry {
        run_id: run_seq,
        profile,
        character: run.character.clone(),
        ascension: run.ascension,
        game_mode: run.game_mode.clone(),
        outcome: RunOutcome::Defeat,
        seed: seed.to_owned(),
        started_at: group_start,
        ended_at: group_end,
        // Combat records carry no roster.
        players: Vec::new(),
    };
    let mut view = build_view(&entry, &cache.combats);
    // The combats-only fallback has no run record: the terminal state is
    // unknown, never a false "Defeat".
    view.outcome = None;
    view.result = "Unfinished".to_owned();
    RunSelection::Selected(Box::new(view))
}

/// Exact runs.jsonl match, else the combats fallback, else Empty.
pub fn select_run(seed: &str, start_time: i64, profile: i32) -> RunSelection {
    ensure_loaded();
    CACHE.with(|cell| {
        let cache = cell.borrow();
        let cache = cache
            .as_ref()
            .expect("ensure_loaded just populated the cache");
        let Some(entry) = match_run(&cache.runs, seed, start_time, profile) else {
            return fallback_from_combats(cache, seed, profile, start_time);
        };
        RunSelection::Selected(Box::new(build_view(entry, &cache.combats)))
    })
}

/// Marks the screen open; returns whether a profiled run matched.
pub fn select(seed: &str, start_time: i64, profile: i32) -> bool {
    let selection = select_run(seed, start_time, profile);
    let matched = matches!(selection, RunSelection::Selected(_));
    match selection {
        RunSelection::Selected(view) => SELECTION.with(|cell| *cell.borrow_mut() = Some(*view)),
        RunSelection::Empty => SELECTION.with(|cell| *cell.borrow_mut() = None),
    }
    SCREEN_OPEN.with(|cell| cell.set(true));
    matched
}

pub fn selected_view() -> Option<RunSummaryView> {
    SELECTION.with(|cell| cell.borrow().clone())
}

/// A `&RunSummaryView` cannot escape the `RefCell` guard, so the per-frame
/// dirty check receives a u64 instead.
pub fn selected_view_fingerprint() -> Option<u64> {
    SELECTION.with(|cell| cell.borrow().as_ref().map(view_fingerprint))
}

/// Compared as one u64 instead of a deep walk.
fn view_fingerprint(view: &RunSummaryView) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    view.run_id.hash(&mut h);
    view.profile.hash(&mut h);
    view.character.hash(&mut h);
    view.ascension.hash(&mut h);
    view.game_mode.hash(&mut h);
    view.outcome.hash(&mut h);
    view.result.hash(&mut h);
    view.seed.hash(&mut h);
    view.started_at.hash(&mut h);
    view.ended_at.hash(&mut h);
    view.players.len().hash(&mut h);
    for player in &view.players {
        player.slot.hash(&mut h);
        player.character.hash(&mut h);
    }
    view.combats.len().hash(&mut h);
    for combat in &view.combats {
        combat.seq.hash(&mut h);
        combat.encounter.hash(&mut h);
        combat.result.hash(&mut h);
        combat.damage_dealt.hash(&mut h);
        combat.damage_taken.hash(&mut h);
        combat.turns.hash(&mut h);
    }
    view.rollup.len().hash(&mut h);
    for card in &view.rollup {
        card.hash(&mut h);
    }
    view.player_rollups.len().hash(&mut h);
    for player in &view.player_rollups {
        player.slot.hash(&mut h);
        player.character.hash(&mut h);
        player.cards.len().hash(&mut h);
        for card in &player.cards {
            card.hash(&mut h);
        }
    }
    h.finish()
}

pub(crate) fn screen_open() -> bool {
    SCREEN_OPEN.with(|cell| cell.get())
}

pub fn run_filter() -> PlayerFilter {
    RUN_FILTER.with(|cell| cell.get())
}

pub fn run_filter_fingerprint() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    run_filter().hash(&mut h);
    h.finish()
}

/// The selected run's roll-up for the current filter; falls back to the
/// All roll-up when the requested player has no entry.
pub fn filtered_rollup(view: &RunSummaryView) -> &[CardStat] {
    match run_filter() {
        PlayerFilter::All => &view.rollup,
        PlayerFilter::Player(slot) => view
            .player_rollups
            .iter()
            .find(|p| p.slot == slot)
            .map_or(&view.rollup[..], |p| &p.cards[..]),
    }
}

pub fn toggle_run_filter(slot: u8) {
    RUN_FILTER.with(|cell| cell.set(cell.get().toggle(slot)));
}

/// The avatar row only renders slots present in the displayed run; a
/// stale filter (a different run selected mid-screen) self-heals to All.
pub fn heal_run_filter() {
    let mut slots: Vec<u8> = SELECTION.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|view| view.players.iter().map(|p| p.slot).collect())
            .unwrap_or_default()
    });
    slots.sort_unstable();
    slots.dedup();
    RUN_FILTER.with(|cell| {
        if let PlayerFilter::Player(s) = cell.get()
            && !slots.contains(&s)
        {
            cell.set(PlayerFilter::All);
        }
    });
}

pub fn clear() {
    SELECTION.with(|cell| *cell.borrow_mut() = None);
    SCREEN_OPEN.with(|cell| cell.set(false));
    RUN_FILTER.with(|cell| cell.set(PlayerFilter::All));
}

#[cfg(test)]
mod tests;
