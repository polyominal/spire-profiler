//! Run-history integration: matching the game's displayed run onto the
//! profiler's records, the structured view the panel renders, and the run
//! close lifecycle.
//!
//! # Screen wiring
//!
//! The shim postfixes the `NRunHistory` screen's `DisplayRun` to forward
//! the displayed identity (seed, `StartTime`, profile), PREFIXES
//! `OnSubmenuOpened` with a clear (a postfix would blank the just-selected
//! view, whose initial `DisplayRun` runs inside the original method), and
//! postfixes the close with a clear filtered to `NRunHistory` so a submenu
//! closing on top never blanks a still-displayed run.
//!
//! # The join key
//!
//! The game's history store names runs `{StartTime}.run`; resumed runs keep
//! the original `RunManager._startTime`. The shim forwards that time, the
//! profile, and `RunRngSet.StringSeed` at both run start and history selection.
//! The persisted identity contract lives in [`crate::data::persistence`].
//!
//! A unique exact identity selects its runs.jsonl entry, or synthesizes a
//! combat-only view for an unclosed run (save & quit, crash, unfired close
//! hook). The latter reads "Unfinished": its terminal outcome is unknown.
//! Missing identity components or multiple matching run IDs select Empty;
//! combat timestamps never decide run membership.
//!
//! # The run close lifecycle
//!
//! * Close on `RunManager.OnEnded` — victory, all-dead defeat, and abandon all funnel through it,
//!   so every finished run closes its record regardless of upload or display settings.
//! * Suspend on save & exit: `RunManager.CleanUp` forwards `spire_profiler_run_suspended`, so a
//!   suspended run writes no record and the next continue closes no spurious defeat. A combat
//!   interrupted by save+quit is never persisted.
//! * Suspend on disconnect: the host-quit path (`RunManager.LocalPlayerDisconnected`) suspends
//!   instead of closing (the reason cannot distinguish save&quit from quit-without-save).
//!
//! The cache parses `runs.jsonl` and the combat store once per data-dir
//! path pair per process, invalidated by successful mid-session combat or
//! run writes.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use serde::Deserialize;

use crate::data::persistence::{
    CardStatKey, card_stat_from_rec, load_combat_docs_from, parse_combat_docs,
};
use crate::data::records::{CombatRec, PlayerRec, RunRec};
use crate::data::state::{CardStat, CombatResult, PlayerFilter, RunOutcome, STATE, TEAM_SLOT};

/// Roll-ups are undeclared on purpose: the view recomputes them.
#[derive(Deserialize)]
#[serde(default)]
pub struct RunEntry {
    pub run_id: u32,
    pub profile: i32,
    pub character: Box<str>,
    pub ascension: i32,
    pub game_mode: Box<str>,
    /// The view's result label derives from it.
    pub outcome: RunOutcome,
    pub seed: Box<str>,
    /// Original game StartTime in epoch seconds.
    pub started_at: i64,
    pub ended_at: i64,
    /// Empty on pre-roster records.
    pub players: Box<[PlayerRec]>,
}

impl Default for RunEntry {
    /// -1 means "never reported".
    fn default() -> Self {
        RunEntry {
            run_id: 0,
            profile: -1,
            character: Box::default(),
            ascension: -1,
            game_mode: Box::default(),
            outcome: RunOutcome::Defeat,
            seed: Box::default(),
            started_at: 0,
            ended_at: 0,
            players: Box::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombatView {
    pub seq: u32,
    pub encounter: Box<str>,
    pub result: CombatResult,
    pub damage_dealt: i64,
    pub damage_taken: i64,
    pub turns: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRollup {
    pub slot: u8,
    pub character: Box<str>,
    pub cards: Box<[CardStat]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunSummaryView {
    pub run_id: u32,
    pub profile: i32,
    pub character: Box<str>,
    pub ascension: i32,
    pub game_mode: Box<str>,
    /// None on the combats-only fallback: the truth is unknown.
    pub outcome: Option<RunOutcome>,
    pub seed: Box<str>,
    pub started_at: i64,
    pub ended_at: i64,
    /// Empty on pre-roster records.
    pub players: Box<[PlayerRec]>,
    pub combats: Box<[CombatView]>,
    /// TEAM-merged; rows keep first-seen order.
    pub rollup: Box<[CardStat]>,
    /// Keyed by roster slot.
    pub player_rollups: Box<[PlayerRollup]>,
}

impl RunSummaryView {
    fn with_combats(mut self, combats: &[CombatRec]) -> Self {
        let mut combat_views = Vec::new();
        let mut rollup = Vec::new();
        let mut player_cards: Vec<Vec<CardStat>> = vec![Vec::new(); self.players.len()];
        let mut latest_combat = None;
        for combat in combats {
            if !combat.run.as_ref().is_some_and(|run| {
                run.seq == self.run_id
                    && run.matches_identity(&self.seed, self.started_at, self.profile)
            }) {
                continue;
            }
            latest_combat = Some(
                latest_combat.map_or(combat.started_at, |time: i64| time.max(combat.started_at)),
            );
            combat_views.push(CombatView {
                seq: combat.combat_id,
                encounter: combat.encounter_id.clone(),
                result: combat.result,
                damage_dealt: combat.cards.iter().map(|card| card.damage_dealt).sum(),
                damage_taken: combat.damage_received,
                turns: combat.turns,
            });
            // Preserve each combat's transaction boundary independently in every rollup.
            for (player, cards) in self.players.iter().zip(&mut player_cards) {
                CardStat::merge_rows(
                    cards,
                    combat
                        .cards
                        .iter()
                        .filter(|row| {
                            crate::data::state::clamp_source_slot(i32::from(row.player))
                                == player.slot
                        })
                        .map(card_stat_from_rec),
                    CardStatKey::TeamMerged,
                );
            }
            CardStat::merge_rows(
                &mut rollup,
                combat.cards.iter().map(|rec| {
                    let mut row = card_stat_from_rec(rec);
                    row.player = TEAM_SLOT;
                    row
                }),
                CardStatKey::TeamMerged,
            );
        }
        if self.outcome.is_none() {
            self.ended_at = latest_combat.unwrap_or(0);
        }
        self.combats = combat_views.into_boxed_slice();
        self.rollup = rollup.into_boxed_slice();
        self.player_rollups = self
            .players
            .iter()
            .zip(player_cards)
            .map(|(player, cards)| PlayerRollup {
                slot: player.slot,
                character: player.character.clone(),
                cards: cards.into_boxed_slice(),
            })
            .collect();
        self
    }
}

pub enum RunSelection {
    Selected(Rc<RunSummaryView>),
    Empty,
}

#[derive(Clone, Copy)]
enum MatchedRun<'a> {
    Ended(&'a RunEntry),
    Unfinished(&'a RunRec),
}

impl MatchedRun<'_> {
    fn id(self) -> u32 {
        match self {
            Self::Ended(run) => run.run_id,
            Self::Unfinished(run) => run.seq,
        }
    }
}

struct Cache {
    runs_path: Box<Path>,
    runs_dir: Box<Path>,
    runs: Box<[RunEntry]>,
    combats: Box<[CombatRec]>,
}

thread_local! {
    static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) };
    static SELECTION: RefCell<Option<Rc<RunSummaryView>>> = const { RefCell::new(None) };
    /// Distinct from [`SELECTION`]: an open screen with no record must still
    /// render its empty-state notice.
    static SCREEN_OPEN: Cell<bool> = const { Cell::new(false) };
    /// Deliberately separate from the live [`State::player_filter`]:
    /// persists across selections so one player can be compared across runs, heals
    /// against the selected view's roster, and never touches live state.
    static RUN_FILTER: Cell<PlayerFilter> = const { Cell::new(PlayerFilter::All) };
}

/// One JSON object per line; one bad line never hides the rest.
fn load_runs(path: &Path) -> Option<Vec<RunEntry>> {
    use crate::data::persistence::{ReadFile, read_file};
    let content = match read_file(path) {
        ReadFile::Content(content) => content,
        ReadFile::Missing => return Some(Vec::new()),
        ReadFile::Failed => return None,
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
    Some(runs)
}

fn load_combats(dir: &Path) -> Vec<CombatRec> {
    parse_combat_docs(load_combat_docs_from(dir)).collect()
}

pub(crate) fn continued_run_id(
    runs_path: &Path,
    runs_dir: &Path,
    seed: &str,
    start_time: i64,
    profile: i32,
) -> Option<u32> {
    matching_run(
        &load_runs(runs_path)?,
        &load_combats(runs_dir),
        seed,
        start_time,
        profile,
    )
    .map(MatchedRun::id)
}

/// An abandoned run leaves its directory but no entry, so the directory
/// name reserves the id; `runs/0/` never counts.
pub(crate) fn next_run_id(runs_path: &Path, runs_dir: &Path) -> Option<u32> {
    let runs_max = load_runs(runs_path)?
        .iter()
        .map(|entry| entry.run_id)
        .max()
        .unwrap_or(0);
    let dirs_max = crate::data::persistence::read_dir(runs_dir)?
        .iter()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    runs_max.max(dirs_max).checked_add(1)
}

fn ensure_loaded() {
    let Some((runs_path, runs_dir)) = STATE.with(|s| {
        s.borrow()
            .store_paths
            .as_ref()
            .map(|paths| (paths.runs_path.clone(), paths.runs_dir.clone()))
    }) else {
        invalidate();
        return;
    };
    let hit = CACHE.with(|cell| {
        let cache = cell.borrow();
        cache
            .as_ref()
            .is_some_and(|c| c.runs_path == runs_path && c.runs_dir == runs_dir)
    });
    if hit {
        return;
    }
    let runs = load_runs(&runs_path).unwrap_or_default();
    let combats = load_combats(&runs_dir);
    CACHE.with(|cell| {
        *cell.borrow_mut() = Some(Cache {
            runs_path,
            runs_dir,
            runs: runs.into_boxed_slice(),
            combats: combats.into_boxed_slice(),
        });
    });
}

pub fn invalidate() {
    CACHE.with(|cell| *cell.borrow_mut() = None);
}

fn matching_run<'a>(
    runs: &'a [RunEntry],
    combats: &'a [CombatRec],
    seed: &str,
    start_time: i64,
    profile: i32,
) -> Option<MatchedRun<'a>> {
    if seed.is_empty() || start_time <= 0 || profile < 0 {
        return None;
    }
    let mut matches = runs
        .iter()
        .filter(|run| {
            run.run_id != 0
                && run.profile == profile
                && run.seed.as_ref() == seed
                && run.started_at == start_time
        })
        .map(MatchedRun::Ended)
        .chain(
            combats
                .iter()
                .rev()
                .filter_map(|combat| combat.run.as_ref())
                .filter(|run| run.matches_identity(seed, start_time, profile))
                .map(MatchedRun::Unfinished),
        );
    let matched = matches.next()?;
    if matches.any(|other| other.id() != matched.id()) {
        crate::fail!("multiple run IDs share profile {profile}, seed '{seed}', start {start_time}");
        return None;
    }
    Some(matched)
}

/// Exact runs.jsonl match, else the combats fallback, else Empty.
pub fn select_run(seed: &str, start_time: i64, profile: i32) -> RunSelection {
    ensure_loaded();
    CACHE.with(|cell| {
        let cache = cell.borrow();
        let Some(cache) = cache.as_ref() else {
            return RunSelection::Empty;
        };
        let Some(matched) = matching_run(&cache.runs, &cache.combats, seed, start_time, profile)
        else {
            return RunSelection::Empty;
        };
        let view = match matched {
            MatchedRun::Ended(entry) => RunSummaryView {
                run_id: entry.run_id,
                profile: entry.profile,
                character: entry.character.clone(),
                ascension: entry.ascension,
                game_mode: entry.game_mode.clone(),
                outcome: Some(entry.outcome),
                seed: entry.seed.clone(),
                started_at: entry.started_at,
                ended_at: entry.ended_at,
                players: entry.players.clone(),
                ..RunSummaryView::default()
            }
            .with_combats(&cache.combats),
            MatchedRun::Unfinished(run) => RunSummaryView {
                run_id: run.seq,
                profile: run.profile,
                character: run.character.clone(),
                ascension: run.ascension,
                game_mode: run.game_mode.clone(),
                seed: run.seed.clone(),
                started_at: run.started_at,
                ..RunSummaryView::default()
            }
            .with_combats(&cache.combats),
        };
        RunSelection::Selected(Rc::new(view))
    })
}

/// Marks the screen open; returns whether a profiled run matched.
pub fn select(seed: &str, start_time: i64, profile: i32) -> bool {
    let selection = select_run(seed, start_time, profile);
    let matched = matches!(selection, RunSelection::Selected(_));
    match selection {
        RunSelection::Selected(view) => SELECTION.with(|cell| *cell.borrow_mut() = Some(view)),
        RunSelection::Empty => SELECTION.with(|cell| *cell.borrow_mut() = None),
    }
    SCREEN_OPEN.with(|cell| cell.set(true));
    matched
}

pub fn selected_view() -> Option<Rc<RunSummaryView>> {
    SELECTION.with(|cell| cell.borrow().clone())
}

/// A `&RunSummaryView` cannot escape the `RefCell` guard, so the per-frame
/// dirty check receives a u64 instead.
pub fn selected_view_fingerprint() -> Option<u64> {
    SELECTION.with(|cell| cell.borrow().as_deref().map(view_fingerprint))
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
    let PlayerFilter::Player(slot) = run_filter() else {
        return;
    };
    let present = SELECTION.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|view| view.players.iter().any(|player| player.slot == slot))
    });
    if !present {
        RUN_FILTER.with(|cell| cell.set(PlayerFilter::All));
    }
}

pub fn clear() {
    SELECTION.with(|cell| *cell.borrow_mut() = None);
    SCREEN_OPEN.with(|cell| cell.set(false));
    RUN_FILTER.with(|cell| cell.set(PlayerFilter::All));
}

#[cfg(test)]
mod tests;
