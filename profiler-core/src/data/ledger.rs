//! Append-only `(player, id, kind)` rows. Ordinary rows cannot consume the
//! five reserved Unknown slots; capacity loss preserves the known creditor.

use super::state::{CardStat, Combat, SourceKind, SourceSlot, caps};

impl CardStat {
    // Bound positive and negative subtotals separately: filtering players or
    // omitting negative defense rows must not expose an overflowing sum.
    pub(crate) fn arithmetic_representable<'a>(rows: impl IntoIterator<Item = &'a Self>) -> bool {
        let mut positive = [0_i128; 15];
        let mut negative = [0_i128; 15];
        for row in rows {
            let mut damage = 0_i128;
            for value in [row.dmg_direct, row.dmg_attributed, row.dmg_modifier] {
                damage += i128::from(value);
                if i64::try_from(damage).is_err() {
                    return false;
                }
            }
            let mut defense = 0_i128;
            for value in [
                row.block_effective,
                row.blk_modifier,
                row.mitigate_debuff,
                row.mitigate_buff,
                row.mitigate_str,
            ] {
                defense += i128::from(value);
                if i64::try_from(defense).is_err() {
                    return false;
                }
            }
            if i64::try_from(defense - i128::from(row.self_damage)).is_err() {
                return false;
            }
            let values = [
                i128::from(row.damage_dealt),
                i128::from(row.damage_blocked),
                i128::from(row.block_gained),
                i128::from(row.block_effective),
                i128::from(row.dmg_direct),
                i128::from(row.dmg_attributed),
                i128::from(row.dmg_modifier),
                i128::from(row.blk_modifier),
                i128::from(row.mitigate_debuff),
                i128::from(row.mitigate_buff),
                i128::from(row.mitigate_str),
                i128::from(row.self_damage),
                i128::from(row.forge),
                damage,
                defense,
            ];
            for ((positive, negative), value) in positive.iter_mut().zip(&mut negative).zip(values)
            {
                *positive += value.max(0);
                *negative += value.min(0);
                if *positive > i128::from(i64::MAX) || *negative < i128::from(i64::MIN) {
                    return false;
                }
            }
        }
        true
    }
}

pub(super) fn get_or_create_card_kind(
    combat: &mut Combat,
    slot: SourceSlot,
    id: &str,
    kind: SourceKind,
) -> Option<usize> {
    let id = if kind == SourceKind::Unknown {
        "UNATTRIBUTED"
    } else {
        id
    };
    if let Some(index) = combat
        .cards
        .iter()
        .position(|row| row.player == slot && row.id == id && row.kind == kind)
    {
        return Some(index);
    }
    if kind != SourceKind::Unknown
        && combat
            .cards
            .iter()
            .filter(|row| row.kind != SourceKind::Unknown)
            .count()
            >= caps::COMBAT_CARDS - caps::UNKNOWN_ROWS
    {
        if !combat.row_capacity_logged {
            combat.row_capacity_logged = true;
            crate::fail!("combat row capacity exhausted; using credited-slot Unknown");
        }
        return get_or_create_card_kind(combat, slot, "UNATTRIBUTED", SourceKind::Unknown);
    }
    if combat.cards.len() >= caps::COMBAT_CARDS {
        crate::fail!("combat row invariant violated: no reserved Unknown slot");
        return None;
    }
    let index = combat.cards.len();
    combat.cards.push(CardStat {
        player: slot,
        id: id.to_owned(),
        kind,
        ..CardStat::default()
    });
    Some(index)
}
