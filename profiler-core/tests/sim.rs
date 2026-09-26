//! Deterministic randomized simulation of the profiler core, inspired by
//! TigerBeetle's VOPR:
//! https://github.com/tigerbeetle/tigerbeetle/blob/97c7a8ef385270ebe0e1b75959d3d21d134629df/docs/internals/vopr.md
//! A seeded PRNG feeds every scenario; `SIM_SEED` replays the event stream
//! and behavioral assertions under equivalent isolated fixtures. Timestamps are
//! fixed host inputs; the walk checks ledger invariants after every event and
//! compares the immutable JSON projection with the final ledger.
//! Supplier grants use a FIFO of individual units; defensive pools retain
//! an independent outer FIFO/residue model and per-seat source prefixes.
//! Nested-hit walks keep an attack pending through Thorns and Inferno, then
//! check another target against fresh modifier budgets from the same play.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use profiler_core::data::events;
use profiler_core::data::state::{self, CardStat, CombatResult, STATE, SourceKind};
use profiler_core::test_util::{SourceFixture, combat_epoch};

const DEFAULT_SEED: u64 = 0x5EED_5EED_5EED_5EED;
const SCENARIOS: u32 = 20;
const EVENTS_PER_SCENARIO: u32 = 40;
const BASE_SOURCES: usize = 6;
const MAX_BLOCK_MODIFIERS: usize = 16;
const TOKEN_KIND_BITS: u32 = 3;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((i64::from(hi) - i64::from(lo) + 1) as u64) as i32
    }
}

fn sim_seed() -> u64 {
    // A malformed override must fail loudly: silently falling back to the
    // default would replay a different walk than the one being debugged.
    match std::env::var("SIM_SEED") {
        Ok(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("SIM_SEED must be a u64 seed, got {value:?}")),
        Err(std::env::VarError::NotPresent) => DEFAULT_SEED,
        Err(std::env::VarError::NotUnicode(_)) => panic!("SIM_SEED must be valid Unicode"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RowKey {
    slot: u8,
    id: Box<str>,
    kind: u8,
}

type Roots = Vec<(RowKey, u64)>;

impl RowKey {
    fn from_card(card: &CardStat) -> Self {
        Self {
            slot: card.player,
            id: card.id.clone(),
            kind: card.kind as u8,
        }
    }

    fn unknown() -> Self {
        Self {
            slot: state::TEAM_SLOT,
            id: "UNATTRIBUTED".into(),
            kind: SourceKind::Unknown as u8,
        }
    }

    fn destination(token: u64) -> Self {
        assert_eq!(
            token >> 32,
            combat_epoch(),
            "source roots retain their combat epoch"
        );
        let index = ((token as u32) >> TOKEN_KIND_BITS) as usize;
        match token & ((1 << TOKEN_KIND_BITS) - 1) {
            0 => STATE.with(|cell| {
                Self::from_card(
                    &cell
                        .borrow()
                        .current
                        .as_ref()
                        .expect("source decoding follows a successful current-combat epoch check")
                        .cards[index],
                )
            }),
            1 => Self {
                slot: index as u8,
                ..Self::unknown()
            },
            _ => panic!("a source packet contains only destination tokens"),
        }
    }
}

enum Origin {
    Ordinary,
    Generated,
    Relic,
    Power,
}

struct SimSource {
    actual: SourceFixture,
    roots: Roots,
    origin: Origin,
    owner: i32,
}

impl SimSource {
    fn card(id: &str, slot: i32) -> Rc<Self> {
        Rc::new(Self {
            actual: SourceFixture::card(id, slot),
            roots: vec![(
                RowKey {
                    slot: slot as u8,
                    id: id.into(),
                    kind: 0,
                },
                1,
            )],
            origin: Origin::Ordinary,
            owner: slot,
        })
    }

    fn relic(id: &str, slot: i32) -> Rc<Self> {
        Rc::new(Self {
            actual: SourceFixture::relic(id, slot),
            roots: vec![(
                RowKey {
                    slot: slot as u8,
                    id: id.into(),
                    kind: 1,
                },
                1,
            )],
            origin: Origin::Relic,
            owner: slot,
        })
    }

    fn power(power: &PowerModel) -> Rc<Self> {
        let source = Rc::new(Self {
            actual: SourceFixture::power(power.instance),
            roots: if power.trusted && (power.id != "POISON_POWER" || !power.units.is_empty()) {
                power.last.clone()
            } else {
                vec![(RowKey::unknown(), 1)]
            },
            origin: Origin::Power,
            owner: 4,
        });
        source.verify();
        source
    }

    fn verify(&self) {
        self.actual.with_transfer(|transfer| {
            let actual: Roots = (0..events::source_count(transfer))
                .map(|index| {
                    (
                        RowKey::destination(events::source_destination(transfer, index)),
                        events::source_weight(transfer, index),
                    )
                })
                .collect();
            assert_eq!(
                actual, self.roots,
                "frozen suppliers match the independent grant model"
            );
        });
    }
}

fn proportional_units(amount: u64, weights: &[u64]) -> Vec<u64> {
    let total: u64 = weights.iter().sum();
    let mut shares = vec![0; weights.len()];
    if total == 0 {
        return shares;
    }
    // Count individual credit thresholds inside each supplier's weighted interval.
    for unit in 1..=amount {
        let mut end = 0;
        for (index, weight) in weights.iter().enumerate() {
            end += weight;
            if amount * end >= unit * total {
                shares[index] += 1;
                break;
            }
        }
    }
    shares
}

struct PowerModel {
    id: &'static str,
    instance: u64,
    owner: (u64, i32, i32),
    observed: i32,
    units: VecDeque<RowKey>,
    last: Roots,
    trusted: bool,
}

impl PowerModel {
    fn new(id: &'static str, instance: u64, owner: (u64, i32, i32), source: &SimSource) -> Self {
        assert_eq!(
            source
                .actual
                .with_transfer(|transfer| events::power_attached(
                    combat_epoch(),
                    instance,
                    id,
                    owner.0,
                    owner.1,
                    owner.2,
                    0,
                    transfer
                )),
            1
        );
        Self {
            id,
            instance,
            owner,
            observed: 0,
            units: VecDeque::new(),
            last: source.roots.clone(),
            trusted: true,
        }
    }

    fn invalidate(&mut self) {
        assert_eq!(
            events::power_provenance_invalidate(combat_epoch(), self.instance),
            1
        );
        self.trusted = false;
    }

    fn change(&mut self, before: i32, after: i32, source: &SimSource) {
        assert_eq!(
            source.roots.len(),
            1,
            "grant units name individual suppliers"
        );
        assert_eq!(
            source
                .actual
                .with_transfer(|transfer| events::power_amount_changed(
                    combat_epoch(),
                    self.instance,
                    self.id,
                    self.owner.0,
                    self.owner.1,
                    self.owner.2,
                    before,
                    after,
                    transfer
                )),
            1
        );
        if !self.trusted || self.observed != before {
            self.units = std::iter::repeat_n(RowKey::unknown(), before.max(0) as usize).collect();
            self.last = vec![(RowKey::unknown(), 1)];
        }
        if after > before {
            self.units.extend(std::iter::repeat_n(
                source.roots[0].0.clone(),
                (after - before) as usize,
            ));
        } else {
            for _ in after..before {
                self.units
                    .pop_front()
                    .expect("observed positive units cover FIFO expiry");
            }
        }
        if !self.units.is_empty() {
            let mut counts: Roots = Vec::new();
            for key in &self.units {
                if let Some((_, count)) = counts.iter_mut().find(|(root, _)| root == key) {
                    *count += 1;
                } else {
                    counts.push((key.clone(), 1));
                }
            }
            let smallest = counts
                .iter()
                .map(|(_, count)| *count)
                .min()
                .expect("a nonempty supplier-unit pool produces at least one positive count");
            let divisor = (1..=smallest)
                .rev()
                .find(|divisor| {
                    counts
                        .iter()
                        .all(|(_, count)| count.is_multiple_of(*divisor))
                })
                .expect("positive supplier counts always share the divisor one");
            for (_, count) in &mut counts {
                *count /= divisor;
            }
            self.last = counts;
        }
        self.observed = after;
        self.trusted = true;
    }
}

struct NaivePrefix {
    roots: Roots,
    seats: Vec<u64>,
}

impl NaivePrefix {
    fn new(roots: Roots) -> Self {
        Self {
            seats: vec![0; roots.len()],
            roots,
        }
    }

    fn credit(&mut self, amount: u64) -> Roots {
        let mut deltas = vec![0; self.roots.len()];
        for _ in 0..amount {
            let mut winner = 0;
            for index in 1..self.roots.len() {
                if self.roots[index].1 * (self.seats[winner] + 1)
                    >= self.roots[winner].1 * (self.seats[index] + 1)
                {
                    winner = index;
                }
            }
            self.seats[winner] += 1;
            deltas[winner] += 1;
        }
        self.roots
            .iter()
            .zip(deltas)
            .filter(|(_, amount)| *amount > 0)
            .map(|((key, _), amount)| (key.clone(), amount))
            .collect()
    }
}

struct NaiveModifier {
    source: NaivePrefix,
    original: u64,
}
struct NaiveChunk {
    base: NaivePrefix,
    base_original: u64,
    remaining: u64,
    mods: Box<[NaiveModifier]>,
}
#[derive(Default)]
struct NaivePool {
    chunks: Vec<NaiveChunk>,
}

impl NaivePool {
    fn push(&mut self, source: Roots, base: u64, modifiers: Vec<(Roots, u64)>, receiver: u8) {
        if modifiers.is_empty()
            && let Some(chunk) = self
                .chunks
                .last_mut()
                .filter(|chunk| chunk.mods.is_empty() && chunk.base.roots == source)
        {
            chunk.remaining += base;
            chunk.base_original += base;
            return;
        }
        if self.chunks.len() == state::caps::BLOCK_POOL {
            let amount = base + modifiers.iter().map(|(_, amount)| amount).sum::<u64>();
            let tail = self.chunks.last_mut().expect("a full pool contains a tail");
            let remaining = tail.remaining + amount;
            let mut unknown = RowKey::unknown();
            unknown.slot = receiver;
            *tail = NaiveChunk {
                base: NaivePrefix::new(vec![(unknown, 1)]),
                base_original: remaining,
                remaining,
                mods: Box::default(),
            };
            return;
        }
        assert!(
            modifiers.len() <= MAX_BLOCK_MODIFIERS,
            "model accepts complete bounded commands"
        );
        let mods: Box<[_]> = modifiers
            .into_iter()
            .filter(|(_, amount)| *amount > 0)
            .map(|(roots, original)| NaiveModifier {
                source: NaivePrefix::new(roots),
                original,
            })
            .collect();
        let remaining = base + mods.iter().map(|modifier| modifier.original).sum::<u64>();
        if remaining > 0 {
            self.chunks.push(NaiveChunk {
                base: NaivePrefix::new(source),
                base_original: base,
                remaining,
                mods,
            });
        }
    }

    fn consume(&mut self, amount: u64, receiver: u8) -> Vec<(RowKey, Field, i64)> {
        let mut remaining = amount;
        let mut credits = Vec::new();
        while remaining > 0 && !self.chunks.is_empty() {
            let chunk = &mut self.chunks[0];
            let take = remaining.min(chunk.remaining);
            let total = chunk.base_original
                + chunk
                    .mods
                    .iter()
                    .map(|modifier| modifier.original)
                    .sum::<u64>();
            let before = total - chunk.remaining;
            let mut modifier_total = 0;
            let outer = |points: u64| {
                let mut seats = vec![0_u64; chunk.mods.len() + 1];
                let mut weights: Vec<_> = chunk.mods.iter().map(|m| m.original).collect();
                weights.push(chunk.base_original);
                for _ in 0..points {
                    let winner = (0..weights.len())
                        .max_by(|&a, &b| {
                            (weights[a] * (seats[b] + 1)).cmp(&(weights[b] * (seats[a] + 1)))
                        })
                        .expect("a block slice has its producer seat");
                    seats[winner] += 1;
                }
                seats
            };
            let portions = if chunk.mods.len() > 1 {
                let previous = outer(before);
                outer(before + take)
                    .iter()
                    .zip(previous)
                    .map(|(after, before)| after - before)
                    .collect::<Vec<_>>()
            } else {
                chunk
                    .mods
                    .iter()
                    .map(|modifier| {
                        modifier.original * (before + take) / total
                            - modifier.original * before / total
                    })
                    .collect()
            };
            for (modifier, delta) in chunk.mods.iter_mut().zip(portions) {
                modifier_total += delta;
                credits.extend(
                    modifier
                        .source
                        .credit(delta)
                        .into_iter()
                        .map(|(key, amount)| (key, Field::BlockModifier, amount as i64)),
                );
            }
            for (key, amount) in chunk.base.credit(take - modifier_total) {
                credits.push((key, Field::BlockEffective, amount as i64));
            }
            chunk.remaining -= take;
            remaining -= take;
            if chunk.remaining == 0 {
                self.chunks.remove(0);
            }
        }
        if remaining > 0 {
            let mut unknown = RowKey::unknown();
            unknown.slot = receiver;
            credits.push((unknown, Field::BlockEffective, remaining as i64));
        }
        credits
    }
}

struct NaiveSummon {
    source: NaivePrefix,
    remaining: u64,
}

#[derive(Clone, Copy)]
enum Field {
    BlockGained,
    BlockEffective,
    BlockModifier,
    Buff,
    SelfDamage,
    Forge,
}

struct DamageShare {
    key: RowKey,
    segment: i32,
    amount: u64,
}

impl DamageShare {
    fn append(credits: &mut Vec<Self>, roots: &[(RowKey, u64)], segment: i32, amount: u64) {
        let weights: Vec<_> = roots.iter().map(|(_, weight)| *weight).collect();
        for ((key, _), amount) in roots.iter().zip(proportional_units(amount, &weights)) {
            if amount == 0 {
                continue;
            }
            if let Some(credit) = credits
                .iter_mut()
                .find(|credit| credit.key == *key && credit.segment == segment)
            {
                credit.amount += amount;
            } else {
                credits.push(Self {
                    key: key.clone(),
                    segment,
                    amount,
                });
            }
        }
    }
}

struct PendingHit {
    calculation: u64,
    source: Rc<SimSource>,
    segment: i32,
    modifiers: Box<[(Rc<SimSource>, u64)]>,
    total: i32,
    blocked: i32,
}

impl PendingHit {
    fn begin(
        source: &Rc<SimSource>,
        total: i32,
        blocked: i32,
        modifiers: Box<[(Rc<SimSource>, u64)]>,
        target: u64,
    ) -> Self {
        let (role, segment) = match source.origin {
            Origin::Ordinary | Origin::Generated => (1, 0),
            Origin::Relic => (3, 0),
            Origin::Power => (2, 1),
        };
        let calculation = source.actual.with_transfer(|transfer| {
            events::damage_calculation_begin(combat_epoch(), transfer, role, segment, target)
        });
        assert_ne!(calculation, 0);
        for (modifier, amount) in &modifiers {
            modifier.actual.contribution(calculation, *amount as i32);
        }
        Self {
            calculation,
            source: Rc::clone(source),
            segment,
            modifiers,
            total,
            blocked,
        }
    }

    fn commit(self, ledger: &mut LedgerModel) {
        assert_eq!(
            events::damage_result_append(
                self.calculation,
                self.total,
                self.total - self.blocked,
                self.blocked,
                0,
                4,
                0
            ),
            1
        );
        assert_eq!(events::damage_calculation_commit(self.calculation), 1);
        ledger.damage(
            &self.source,
            self.segment,
            &self.modifiers,
            self.total as u64,
            self.blocked as u64,
        );
    }
}

struct LedgerModel {
    rows: BTreeMap<RowKey, CardStat>,
    pools: [NaivePool; 5],
    osty: [Vec<NaiveSummon>; 5],
    players: Vec<bool>,
    plays: u32,
    generated_plays: u32,
    generation_triggers: u32,
    outgoing: i64,
    blocked: i64,
    received: i64,
    block_total: i64,
    turns: u32,
    potions: u32,
}

impl LedgerModel {
    fn new(sources: &[Rc<SimSource>]) -> Self {
        let mut model = Self {
            rows: BTreeMap::new(),
            pools: std::array::from_fn(|_| NaivePool::default()),
            osty: std::array::from_fn(|_| Vec::new()),
            players: Vec::new(),
            plays: 0,
            generated_plays: 0,
            generation_triggers: 0,
            outgoing: 0,
            blocked: 0,
            received: 0,
            block_total: 0,
            turns: 0,
            potions: 0,
        };
        for source in sources {
            for (key, _) in &source.roots {
                model.row(key);
            }
        }
        model
    }

    fn row(&mut self, key: &RowKey) -> &mut CardStat {
        self.rows.entry(key.clone()).or_insert_with(|| CardStat {
            player: key.slot,
            id: key.id.clone(),
            kind: SourceKind::from_c(i32::from(key.kind)),
            ..CardStat::default()
        })
    }

    fn observe(&mut self, slot: i32) {
        self.players
            .resize(self.players.len().max(slot as usize + 1), false);
    }

    fn add(&mut self, key: &RowKey, field: Field, amount: i64) {
        if amount == 0 {
            return;
        }
        let row = self.row(key);
        let value = match field {
            Field::BlockGained => &mut row.block_gained,
            Field::BlockEffective => &mut row.block_effective,
            Field::BlockModifier => &mut row.blk_modifier,
            Field::Buff => &mut row.mitigate_buff,
            Field::SelfDamage => &mut row.self_damage,
            Field::Forge => &mut row.forge,
        };
        *value += amount;
    }

    fn credit(&mut self, roots: &[(RowKey, u64)], field: Field, amount: u64) {
        let weights: Vec<_> = roots.iter().map(|(_, weight)| *weight).collect();
        for ((key, _), amount) in roots.iter().zip(proportional_units(amount, &weights)) {
            self.add(key, field, amount as i64);
        }
    }

    fn damage(
        &mut self,
        source: &SimSource,
        segment: i32,
        modifiers: &[(Rc<SimSource>, u64)],
        total: u64,
        blocked: u64,
    ) {
        let weights: Vec<_> = modifiers.iter().map(|(_, amount)| *amount).collect();
        let budget = total.min(weights.iter().sum());
        let mut credits = Vec::new();
        for ((source, _), amount) in modifiers.iter().zip(proportional_units(budget, &weights)) {
            DamageShare::append(&mut credits, &source.roots, 2, amount);
        }
        DamageShare::append(&mut credits, &source.roots, segment, total - budget);
        let weights: Vec<_> = credits.iter().map(|credit| credit.amount).collect();
        for (credit, blocked) in credits.iter().zip(proportional_units(blocked, &weights)) {
            let row = self.row(&credit.key);
            row.damage_dealt += credit.amount as i64;
            row.damage_blocked += blocked as i64;
            match credit.segment {
                0 => row.dmg_direct += credit.amount as i64,
                1 => row.dmg_attributed += credit.amount as i64,
                2 => row.dmg_modifier += credit.amount as i64,
                _ => panic!("model damage uses only the three wire segments"),
            }
        }
        self.outgoing += total as i64;
        self.blocked += blocked as i64;
    }

    fn play(&mut self, source: &SimSource) -> u64 {
        let play = source.actual.play();
        assert_ne!(play, 0);
        self.plays += 1;
        self.observe(source.owner);
        match source.origin {
            Origin::Ordinary => self.row(&source.roots[0].0).plays += 1,
            Origin::Generated => self.generated_plays += 1,
            _ => panic!("only actual cards start a card play"),
        }
        play
    }

    fn generate(&mut self, source: &SimSource) {
        for (key, _) in &source.roots {
            if key.kind != SourceKind::Card as u8 {
                self.row(key).plays += 1;
                self.generation_triggers += 1;
            }
        }
    }

    fn gain(
        &mut self,
        source: &SimSource,
        base: u64,
        modifiers: &[(Rc<SimSource>, u64)],
        receiver: i32,
    ) {
        let total = base + modifiers.iter().map(|(_, amount)| amount).sum::<u64>();
        let entries: Vec<_> = modifiers
            .iter()
            .map(|(modifier, amount)| {
                modifier
                    .actual
                    .with_transfer(|source| profiler_core::abi::BlockModifier {
                        source,
                        credit: *amount as i64,
                    })
            })
            .collect();
        assert_eq!(
            source
                .actual
                .with_transfer(|source| events::block_gained_with_modifiers(
                    combat_epoch(),
                    total as i32,
                    source,
                    receiver,
                    &entries,
                    false,
                )),
            1
        );
        self.block_total += total as i64;
        self.credit(&source.roots, Field::BlockGained, total);
        if total > 0 {
            self.observe(receiver);
            self.pools[receiver as usize].push(
                source.roots.clone(),
                base,
                modifiers
                    .iter()
                    .map(|(source, amount)| (source.roots.clone(), *amount))
                    .collect(),
                receiver as u8,
            );
        }
    }

    fn receive(&mut self, source: &SimSource, total: u64, blocked: u64, kind: i32, receiver: i32) {
        source
            .actual
            .hit(total as i32, blocked as i32, kind, receiver);
        self.record_received(source, total, blocked, kind, receiver);
    }

    fn record_received(
        &mut self,
        source: &SimSource,
        total: u64,
        blocked: u64,
        kind: i32,
        receiver: i32,
    ) {
        self.observe(receiver);
        if kind == 4 {
            let mut remaining = total;
            while remaining > 0 && !self.osty[receiver as usize].is_empty() {
                let entry = self.osty[receiver as usize]
                    .last_mut()
                    .expect("the absorption loop runs only while an Osty grant remains");
                let take = remaining.min(entry.remaining);
                let credits = entry.source.credit(take);
                entry.remaining -= take;
                remaining -= take;
                if entry.remaining == 0 {
                    self.osty[receiver as usize].pop();
                }
                for (key, amount) in credits {
                    self.add(&key, Field::BlockEffective, amount as i64);
                }
            }
            let key = RowKey {
                slot: state::TEAM_SLOT,
                id: "OSTY".into(),
                kind: SourceKind::Osty as u8,
            };
            self.add(&key, Field::BlockEffective, remaining as i64);
        } else {
            self.received += total as i64;
            for (key, field, amount) in
                self.pools[receiver as usize].consume(blocked, receiver as u8)
            {
                self.add(&key, field, amount);
            }
            if kind == 2 {
                self.credit(&source.roots, Field::SelfDamage, total - blocked);
            }
        }
    }

    fn kill_osty(&mut self, source: &SimSource) {
        let remaining: u64 = self.osty[source.owner as usize]
            .iter()
            .map(|entry| entry.remaining)
            .sum();
        let weights: Vec<_> = source.roots.iter().map(|(_, weight)| *weight).collect();
        for ((key, _), amount) in source
            .roots
            .iter()
            .zip(proportional_units(remaining, &weights))
        {
            self.add(key, Field::BlockEffective, -(amount as i64));
        }
        self.osty[source.owner as usize].clear();
    }

    fn check(&self, repro: &str, step: u32) {
        STATE.with(|cell| {
            let state = cell.borrow();
            let combat = state
                .current
                .as_ref()
                .expect("model events retain their active combat");
            let actual: BTreeMap<_, _> = combat
                .cards
                .iter()
                .map(|row| (RowKey::from_card(row), row.clone()))
                .collect();
            assert_eq!(
                actual, self.rows,
                "{repro} step {step}: complete credited rows"
            );
            assert_eq!(
                (
                    combat.plays,
                    combat.generated_plays,
                    combat.generation_triggers
                ),
                (self.plays, self.generated_plays, self.generation_triggers),
                "{repro} step {step}: play counts"
            );
            assert_eq!(
                (
                    combat.damage_received,
                    combat.block_total,
                    combat.turns,
                    combat.potions_used
                ),
                (self.received, self.block_total, self.turns, self.potions),
                "{repro} step {step}: combat totals"
            );
            assert_eq!(
                combat.cards.iter().map(|row| row.damage_dealt).sum::<i64>(),
                self.outgoing,
                "{repro} step {step}: actual outgoing damage"
            );
            assert_eq!(
                combat
                    .cards
                    .iter()
                    .map(|row| row.damage_blocked)
                    .sum::<i64>(),
                self.blocked,
                "{repro} step {step}: actual blocked damage"
            );
            assert_eq!(
                state
                    .per_player
                    .iter()
                    .map(|player| player.died)
                    .collect::<Vec<_>>(),
                self.players,
                "{repro} step {step}: physical player lifetimes"
            );
        });
        check_invariants(repro, step);
    }
}

struct Walk {
    sources: Vec<Rc<SimSource>>,
    power: PowerModel,
    ledger: LedgerModel,
    next_generated: u64,
}

impl Walk {
    fn new() -> Self {
        let sources = vec![
            SimSource::card("STRIKE", 0),
            SimSource::card("DEFEND", 1),
            SimSource::card("INFLAME", 2),
            SimSource::card("WHITE_NOISE", 0),
            SimSource::relic("VAJRA", 1),
            SimSource::relic("CRACKED_CORE", 2),
        ];
        let power = PowerModel::new("POISON_POWER", 100_001, (900, 1, 4), &sources[0]);
        let ledger = LedgerModel::new(&sources);
        Self {
            sources,
            power,
            ledger,
            next_generated: 200_001,
        }
    }

    fn source(&self, rng: &mut Rng) -> Rc<SimSource> {
        let index = rng.below(self.sources.len() as u64 + 1) as usize;
        if index == self.sources.len() {
            SimSource::power(&self.power)
        } else {
            Rc::clone(&self.sources[index])
        }
    }

    fn card(&self, rng: &mut Rng) -> Rc<SimSource> {
        let cards: Vec<_> = self
            .sources
            .iter()
            .filter(|source| matches!(source.origin, Origin::Ordinary | Origin::Generated))
            .collect();
        Rc::clone(cards[rng.below(cards.len() as u64) as usize])
    }

    fn damage(&mut self, rng: &mut Rng, source: &Rc<SimSource>) {
        let total = rng.range_i32(0, 30);
        let blocked = rng.range_i32(0, total);
        let modifiers = (0..rng.below(4))
            .map(|_| {
                let modifier = self.source(rng);
                let amount = rng.range_i32(0, 8);
                (modifier, amount as u64)
            })
            .collect();
        PendingHit::begin(source, total, blocked, modifiers, 900).commit(&mut self.ledger);
    }

    fn block(&mut self, rng: &mut Rng, source: &Rc<SimSource>) {
        let base = rng.range_i32(0, 25);
        let receiver = rng.range_i32(0, 2);
        let mut modifiers = Vec::new();
        for _ in 0..rng.below(3) {
            let modifier = self.source(rng);
            let amount = rng.range_i32(1, 8);
            modifiers.push((modifier, amount as u64));
        }
        self.ledger.gain(source, base as u64, &modifiers, receiver);
    }

    fn change_power(&mut self, rng: &mut Rng) {
        if rng.below(6) == 0 {
            self.power.invalidate();
            return;
        }
        let source = &self.sources[rng.below(BASE_SOURCES as u64) as usize];
        let before = if rng.below(5) == 0 {
            rng.range_i32(0, 12)
        } else {
            self.power.observed
        };
        self.power.change(before, rng.range_i32(0, 12), source);
    }

    fn generate(&mut self, rng: &mut Rng) {
        let source = self.source(rng);
        let owner = rng.range_i32(0, 2);
        source.actual.generate(self.next_generated);
        self.ledger.generate(&source);
        let generated = Rc::new(SimSource {
            actual: SourceFixture::generated("GENERATED", owner, self.next_generated),
            roots: source.roots.clone(),
            origin: Origin::Generated,
            owner,
        });
        generated.verify();
        self.sources.push(generated);
        self.next_generated += 1;
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one dispatch keeps the seeded event weights and ledger updates together"
    )]
    fn event(&mut self, rng: &mut Rng) {
        let source = self.source(rng);
        let epoch = combat_epoch();
        match rng.below(99) {
            0..=15 => {
                let card = self.card(rng);
                let play = self.ledger.play(&card);
                if rng.below(3) == 0 {
                    self.generate(rng);
                } else {
                    self.damage(rng, &card);
                }
                card.actual.finish(play);
            }
            16..=33 => self.damage(rng, &source),
            34..=43 => self.block(rng, &source),
            44..=57 => self.change_power(rng),
            58..=63 => self.generate(rng),
            64..=73 => {
                let total = rng.range_i32(0, 30);
                let blocked = rng.range_i32(0, total);
                let receiver = rng.range_i32(0, 2);
                let kind = rng.range_i32(1, 2);
                self.ledger
                    .receive(&source, total as u64, blocked as u64, kind, receiver);
            }
            74..=77 => {
                let amount = rng.range_i32(1, 8);
                source.actual.forge(amount);
                self.ledger
                    .credit(&source.roots, Field::Forge, amount as u64);
            }
            78..=81 => {
                let amount = rng.range_i32(1, 8);
                assert_eq!(
                    source
                        .actual
                        .with_transfer(|transfer| events::buff_mitigation(epoch, transfer, amount)),
                    1
                );
                self.ledger
                    .credit(&source.roots, Field::Buff, amount as u64);
            }
            82..=84 => {
                assert_eq!(events::turn_started(epoch), 1);
                self.ledger.turns += 1;
            }
            85..=87 => {
                let slot = rng.range_i32(0, 2);
                assert_eq!(events::block_pool_clear(epoch, slot), 1);
                self.ledger.pools[slot as usize].chunks.clear();
            }
            88..=90 => {
                let amount = rng.range_i32(1, 10);
                let owner = rng.range_i32(0, 2);
                let accepted = source.actual.with_transfer(|transfer| {
                    events::osty_summoned(epoch, transfer, amount, owner)
                });
                if self.ledger.osty[owner as usize].len() == state::caps::OSTY_STACK {
                    assert_eq!(accepted, 0);
                } else {
                    assert_eq!(accepted, 1);
                    self.ledger.observe(owner);
                    self.ledger.osty[owner as usize].push(NaiveSummon {
                        source: NaivePrefix::new(source.roots.clone()),
                        remaining: amount as u64,
                    });
                }
            }
            91..=93 => {
                let amount = rng.range_i32(1, 10);
                let owner = rng.range_i32(0, 2);
                self.ledger.receive(&source, amount as u64, 0, 4, owner);
            }
            94 => {
                let card = self.card(rng);
                let play = self.ledger.play(&card);
                assert_eq!(events::osty_killed(epoch, card.owner, play), 1);
                self.ledger.kill_osty(&card);
                card.actual.finish(play);
            }
            95..=96 => {
                assert_eq!(events::potion_used(epoch), 1);
                self.ledger.potions += 1;
            }
            _ => {
                let slot = rng.range_i32(0, 2);
                assert_eq!(events::player_died(epoch, slot), 1);
                assert_eq!(events::player_died(epoch, slot), 1);
                self.ledger.observe(slot);
                self.ledger.players[slot as usize] = true;
            }
        }
    }

    fn check(&self, repro: &str, step: u32) {
        SimSource::power(&self.power);
        self.ledger.check(repro, step);
    }
}

fn check_invariants(repro: &str, step: u32) {
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state
            .current
            .as_ref()
            .expect("simulation events retain a combat");
        let plays: u64 = combat.cards.iter().map(|card| u64::from(card.plays)).sum();
        assert_eq!(
            u64::from(combat.plays) + u64::from(combat.generation_triggers),
            plays + u64::from(combat.generated_plays),
            "{repro} step {step}: play conservation"
        );
        assert!(
            combat.block_total >= 0 && combat.damage_received >= 0,
            "{repro} step {step}: nonnegative combat totals"
        );
        assert!(
            combat.cards.len() <= state::caps::COMBAT_CARDS,
            "{repro} step {step}: row capacity"
        );
        assert!(
            state.per_player.len() <= state::caps::MAX_PLAYER_SLOTS,
            "{repro} step {step}: physical player capacity"
        );
        let mut seen = BTreeSet::new();
        for card in &combat.cards {
            assert!(
                card.player <= state::TEAM_SLOT && seen.insert(RowKey::from_card(card)),
                "{repro} step {step}: unique valid (slot, id, kind) row identity"
            );
            assert_eq!(
                card.damage_dealt,
                card.dmg_direct + card.dmg_attributed + card.dmg_modifier,
                "{repro} step {step}: {} segment sum",
                card.id
            );
            assert!(
                card.damage_blocked >= 0 && card.damage_blocked <= card.damage_dealt,
                "{repro} step {step}: {} blocked damage is covered by actual damage",
                card.id
            );
            assert!(
                card.dmg_direct >= 0 && card.dmg_attributed >= 0 && card.dmg_modifier >= 0,
                "{repro} step {step}: {} nonnegative damage segments",
                card.id
            );
            assert!(
                card.block_gained >= 0
                    && card.blk_modifier >= 0
                    && card.mitigate_debuff >= 0
                    && card.mitigate_buff >= 0
                    && card.mitigate_str >= 0
                    && card.self_damage >= 0
                    && card.forge >= 0,
                "{repro} step {step}: {} nonnegative positive-only metrics",
                card.id
            );
        }
    });
}

#[test]
fn randomized_frozen_sources_match_naive_attribution() {
    let seed = sim_seed();
    let repro = format!("SIM_SEED={seed} frozen sources");
    let mut rng = Rng::new(seed);
    events::test_reset();
    events::combat_started("FROZEN_SOURCES", "test");
    let mut walk = Walk::new();
    let mut frozen = VecDeque::new();
    for step in 0..256 {
        if step == 20 {
            assert_eq!(events::turn_started(combat_epoch()), 1);
            walk.ledger.turns += 1;
        }
        if step == 90 {
            let epoch = combat_epoch();
            let pending = walk.sources[0]
                .actual
                .with_transfer(|source| events::damage_calculation_begin(epoch, source, 1, 0, 900));
            assert_ne!(pending, 0);
            events::combat_started("NEXT_SOURCES", "test");
            assert_eq!(events::damage_calculation_abort(pending), 0);
            assert_eq!(events::damage_unattributed(epoch, 1, 1, 0, 0, 4, 0), 0);
            walk = Walk::new();
            frozen.clear();
        }
        frozen.push_back(SimSource::power(&walk.power));
        if frozen.len() > 16 {
            frozen.pop_front();
        }
        walk.change_power(&mut rng);
        let source = Rc::clone(&frozen[rng.below(frozen.len() as u64) as usize]);
        walk.damage(&mut rng, &source);
        walk.block(&mut rng, &source);
        walk.check(&repro, step);
    }
}

#[test]
fn randomized_nested_hits_keep_frozen_suppliers_and_per_target_budgets() {
    let seed = sim_seed();
    let repro = format!("SIM_SEED={seed} nested hits");
    let mut rng = Rng::new(seed ^ 0xCA11_BACC_1AFE);
    events::test_reset();
    events::combat_started("NESTED_HITS", "test");
    let sources = [
        SimSource::card("STRIKE", 0),
        SimSource::card("INFERNO", 0),
        SimSource::card("WHITE_NOISE", 1),
        SimSource::card("INFLAME", 2),
        SimSource::card("DEFEND", 0),
    ];
    let [attack, supplier, generator, modifier, defend] = &sources;
    let mut power = PowerModel::new("INFERNO_POWER", 100_001, (100, 0, 0), supplier);
    power.change(0, 3, supplier);
    power.change(3, 5, generator);
    let mut model = LedgerModel::new(&sources);
    for step in 0..256 {
        let play = model.play(attack);
        let block = rng.range_i32(0, 5);
        model.gain(defend, block as u64, &[], 0);
        model.check(&format!("{repro} before parent"), step);
        let total = rng.range_i32(1, 30);
        let modifiers = Box::new([(Rc::clone(modifier), rng.range_i32(1, 8) as u64)]);
        let parent = PendingHit::begin(attack, total, rng.range_i32(0, total), modifiers, 900);
        model.check(&format!("{repro} pending parent"), step);

        // Thorns damages the attacker before its pending hit; positive HP loss
        // then triggers Inferno on that player's turn in game v0.111.0.
        let thorns = block + rng.range_i32(1, 5);
        assert_eq!(
            events::damage_unattributed(combat_epoch(), thorns, thorns - block, block, 1, 0, 0),
            1
        );
        model.record_received(attack, thorns as u64, block as u64, 1, 0);
        model.check(&format!("{repro} after Thorns"), step);
        let frozen = SimSource::power(&power);
        let child = PendingHit::begin(
            &frozen,
            power.observed,
            rng.range_i32(0, power.observed),
            Box::default(),
            900,
        );
        let incoming = if rng.below(2) == 0 {
            supplier
        } else {
            generator
        };
        power.change(power.observed, rng.range_i32(1, 12), incoming);
        model.check(&format!("{repro} suspended Inferno"), step);
        child.commit(&mut model);
        model.check(&format!("{repro} after Inferno"), step);
        parent.commit(&mut model);
        model.check(&format!("{repro} after parent"), step);

        let total = rng.range_i32(0, 5);
        let modifiers = Box::new([(Rc::clone(modifier), rng.range_i32(9, 16) as u64)]);
        PendingHit::begin(attack, total, rng.range_i32(0, total), modifiers, 901)
            .commit(&mut model);
        attack.actual.finish(play);
        model.check(&format!("{repro} after next target"), step);
    }
}

#[test]
fn randomized_combat_lifecycle_invariants() {
    let base_seed = sim_seed();
    for scenario in 0..SCENARIOS {
        let repro = format!("SIM_SEED={base_seed} scenario {scenario}");
        let mut rng = Rng::new(base_seed ^ u64::from(scenario).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        events::test_reset();
        let player_count = if scenario % 2 == 0 { 3 } else { 4 };
        STATE.with(|cell| {
            cell.borrow_mut().combat_started(
                1,
                "SIM_ENCOUNTER",
                "test",
                1_786_579_200,
                player_count as i32,
            )
        });
        let mut walk = Walk::new();
        walk.ledger.players = vec![false; player_count];
        for step in 0..EVENTS_PER_SCENARIO {
            walk.event(&mut rng);
            walk.check(&repro, step);
        }
        assert_eq!(events::combat_ended(combat_epoch()), 1);
        let player_died =
            !walk.ledger.players.is_empty() && walk.ledger.players.iter().all(|died| *died);
        STATE.with(|cell| {
            let state = cell.borrow();
            let combat = state.current.as_ref().expect("finished combat exists");
            assert_eq!(
                combat.result(),
                Some(if player_died {
                    CombatResult::Defeat
                } else {
                    CombatResult::Completed
                }),
                "{repro}"
            );
            let doc: serde_json::Value =
                serde_json::from_str(&state.snapshot()).expect("snapshot JSON parses");
            assert_eq!(doc["turns"], combat.turns);
            assert_eq!(doc["damage_received"], combat.damage_received);
            assert_eq!(
                doc["cards"],
                serde_json::to_value(&combat.cards).expect("rows serialize")
            );
        });
    }
}

#[test]
fn block_pool_consume_matches_naive_model() {
    let base_seed = sim_seed();
    let repro = format!("SIM_SEED={base_seed} block pool");
    let mut rng = Rng::new(base_seed ^ 0xB10C_3001_C0DE);
    events::test_reset();
    events::combat_started("BLOCKPOOL_SIM", "test");
    let mut sources = vec![
        SimSource::card("DEFEND", 0),
        SimSource::card("ARMAMENTS", 1),
        SimSource::card("IRON_WAVE", 2),
        SimSource::card("BODYGUARD", 0),
        SimSource::card("CRIMSON_MANTLE", 1),
    ];
    let modifiers = [
        SimSource::card("FOOTWORK", 0),
        SimSource::card("TEMPORARY_DEXTERITY", 2),
        SimSource::relic("SMOOTH_STONE", 1),
    ];
    let mut first = PowerModel::new("DEXTERITY_POWER", 100_001, (1000, 0, 0), &modifiers[0]);
    first.change(0, 2, &modifiers[0]);
    first.change(2, 5, &modifiers[1]);
    let mut second = PowerModel::new("DEXTERITY_POWER", 100_002, (1001, 0, 1), &modifiers[2]);
    second.change(0, 1, &modifiers[2]);
    second.change(1, 2, &modifiers[0]);
    let all: Vec<_> = sources.iter().chain(&modifiers).cloned().collect();
    let mut model = LedgerModel::new(&all);
    let modifiers = [
        Rc::clone(&modifiers[0]),
        SimSource::power(&first),
        SimSource::power(&second),
    ];
    sources.push(SimSource::power(&first));
    let mut step = 0;
    for _round in 0..40 {
        for _ in 0..rng.below(5) + 1 {
            let mut pending = Vec::new();
            if rng.below(3) == 0 {
                for _ in 0..rng.below(4) + 1 {
                    pending.push((
                        Rc::clone(&modifiers[rng.below(modifiers.len() as u64) as usize]),
                        rng.range_i32(1, 10) as u64,
                    ));
                }
            }
            let source = &sources[rng.below(sources.len() as u64) as usize];
            model.gain(source, rng.range_i32(0, 25) as u64, &pending, 0);
            model.check(&repro, step);
            step += 1;
        }
        for _ in 0..rng.below(3) + 1 {
            let amount = if rng.below(4) == 0 {
                rng.range_i32(1, 500)
            } else {
                rng.range_i32(1, 30)
            } as u64;
            model.receive(&sources[0], amount, amount, 1, 0);
            model.check(&repro, step);
            step += 1;
        }
    }
    model.receive(&sources[0], 10_000, 10_000, 1, 0);
    model.check(&repro, step);
    assert!(
        model.pools[0].chunks.is_empty(),
        "{repro}: the final hit drains every modeled chunk"
    );
}
