//! The data layer — combat facts, attribution, and the persisted JSON
//! model. Engine-free: nothing here knows the engine exists. All mutable
//! state sits in the thread-local [`state::STATE`] `RefCell<State>`;
//! [`events`] holds the borrow once while mutating and emitting the
//! gameplay event trace; the `profiler.log` sink owns its destination and
//! never re-enters [`state::STATE`].
//!
//! [`records`] — the persisted record types; [`events`] — the export
//! bodies; [`ledger`] — attribution mechanics; [`persistence`] — the JSON
//! files; [`run_history`] — run matching; [`state`] — state types and caps.
//!
//! # The attribution model
//!
//! Every ledger row carries a [`crate::source_kind::SourceKind`] (also the stored
//! `kind: u8`):
//!
//! | discriminant | kind   | entries                              |
//! |--------------|--------|--------------------------------------|
//! | 0            | Card   | deck plays, explicit card sources    |
//! | 1            | Relic  | relic-hook contexts                  |
//! | 2            | Power  | power-hook contexts, power appliers  |
//! | 3            | Potion | potion usage                         |
//! | 4            | Osty   | the multiplayer boss's own entries   |
//!
//! [`crate::source_kind::SourceKind::from_c`] clamps an unchecked C `c_int`
//! to card/relic/power (the shim only sends catalogued kinds for contexts),
//! while serde's numeric representation (`From<u8>`/`Into<u8>`) round-trips
//! all five stored bytes so potion/osty survive the JSON. Rows carry
//! `(slot, id, kind)` but the
//! ledger's lookups key `(slot, id)` — an id is unique per slot, so two
//! players' same-id cards stay separate rows; the slot vocabulary is the
//! player-slot model in [`state`].
//!
//! ## Source resolution
//!
//! Two "first non-empty wins" chains resolve a source and create the ledger
//! row when it is missing (the exact branch orders live in [`ledger`]):
//!
//! * [`ledger::resolve_card_in`] — block, forge, self-damage, Osty, and mitigation: explicit card
//!   id → the slot's play source → the innermost context → the orb fallback → the potion fallback →
//!   `last_source`, else `None` (the event still counts in the combat totals). The Doom kill
//!   catch-all opts out of `last_source` so an unrelated earlier hook is never guessed.
//! * [`ledger::resolve_damage_source_in`] — damage, additionally recording how the source resolved
//!   so the chart splits direct from indirect: explicit card id → direct; the orb fallback →
//!   indirect (orbs trigger during a play, so their damage belongs to the channeling source); the
//!   play source → direct; the innermost context → indirect iff a power; the potion fallback →
//!   direct; then poison layers claim the hit and, failing that, `last_source` catches async
//!   continuations.
//!
//! ## Contexts and the async fallback rule
//!
//! The shim wraps every catalogued relic/power hook in a begin/end pair. A
//! bounded stack keeps nested hooks correct; [`events::context_begin`]
//! also writes `last_source`, remembered after the context pops, because
//! game hooks fire across an `await` (Crimson Mantle's turn-start block).
//! The orb and potion fallbacks are set on entry and cleared the moment
//! any other attribution source appears; [`events::turn_started`] clears
//! both and `last_source` — a new turn is a natural boundary.
//!
//! ## Orb and potion attribution
//!
//! Orbs are async, so their channeling source is recorded up front
//! ([`events::orb_channeled`]: innermost context > play source >
//! `last_source`); [`events::orb_context_begin`] activates the orb
//! fallback and clears the potion fallback. During a play only the FIRST
//! orb trigger credits the channeling source — later triggers credit the
//! evoking card. Potions prefix `OnUseWrapper` so the fallback exists
//! before the effects run (a postfix would be too late for FlexPotion's
//! Strength); each slot owns its potion source directly, outside the orb
//! table.
//!
//! ## Generated cards
//!
//! A copy generated during combat is credited to its top-level
//! non-generated ancestor (a deck card, relic, power, or potion) — no
//! sub-rows. A row's `plays` counts the source's OWN triggers, so
//! `contribution / plays` is the expected value per trigger:
//! `plays + generation_triggers == Σ cards[].plays + generated_plays`.
//! [`events::card_generated`] records the instance→generator mapping;
//! the play then overrides to that `(id, kind)` for everything during
//! the play.
//!
//! ## Block pool and damage modifiers
//!
//! Block is a FIFO pool of chunks; a chunk is a base amount plus up to 4
//! modifier slices, consumed cumulatively-proportionally (residue on the
//! base). Proc block gains arrive with an empty id (the game's `GainBlock`
//! contract) and resolve through the chain or count toward `block_total`
//! only. Queued damage-modifier contributions are carved out of the
//! attacker's direct damage into the modifier sources. The shim re-runs the
//! game's own resolution step-marginally (an additive owns its raw delta, a
//! multiplicative `m` owns `running·(m−1)`); Vulnerable's nested composite
//! splits linearly in multiplier space, falling back to the whole marginal
//! if the re-run cannot reproduce it.
//!
//! ## Damage segments and records
//!
//! `damage_dealt == dmg_direct + dmg_attributed + dmg_modifier`
//! is re-checked after every mutation. Per-power records (`power_sources`,
//! `debuff_layers`, `doom_layers`, `osty_stack`, `str_reductions`) remember
//! appliers so proportional splits credit the right source; their caps and
//! one-line rationales live in [`state::caps`].

pub mod events;
pub mod ledger;
pub mod persistence;
pub mod records;
pub mod run_history;
pub mod state;
