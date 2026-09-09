//! Instance ancestry and exact execution-owned play lifetimes. Physical playing
//! slots govern orb timing; immutable snapshots govern creditor ownership.

use super::*;
use crate::data::state::SourceKind;

impl State {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn card_source(
        &mut self,
        epoch: CombatEpoch,
        instance: u64,
        id: &str,
        slot: SourceSlot,
        generation: GenerationState,
        given: Option<SourceSnapshot>,
    ) -> SourceSnapshot {
        if generation == GenerationState::GeneratedUnavailable {
            self.provenance
                .generated
                .retain(|entry| entry.instance != instance);
            return SourceSnapshot::unknown(epoch);
        }
        let generated = self
            .provenance
            .generated
            .iter()
            .find(|entry| entry.instance == instance);
        match generation {
            GenerationState::Ordinary if instance != 0 && !id.is_empty() && generated.is_none() => {
                if let Some(source) = given {
                    if source.shares().len() == 1 {
                        source
                    } else {
                        self.source_transfers
                            .diagnostics
                            .report(SourceFailure::Packet);
                        SourceSnapshot::unknown(epoch)
                    }
                } else {
                    self.named_source(epoch, slot, id, SourceKind::Card)
                }
            }
            GenerationState::Ordinary => {
                self.source_transfers
                    .diagnostics
                    .report(SourceFailure::Packet);
                SourceSnapshot::unknown(epoch)
            }
            GenerationState::GeneratedRecorded => generated.map_or_else(
                || SourceSnapshot::unknown(epoch),
                |entry| {
                    crate::data::persistence::event_log!(
                        "  generated instance {instance}, supplier role {:?}",
                        entry.producer_role
                    );
                    given.unwrap_or_else(|| entry.source.clone())
                },
            ),
            _ => SourceSnapshot::unknown(epoch),
        }
    }

    pub(crate) fn card_generated(
        &mut self,
        combat_seq: u64,
        instance: u64,
        transfer: u64,
        producer_role: i32,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            self.provenance
                .generated
                .retain(|entry| entry.instance != instance);
            let role = ProducerRole::decode(producer_role, &mut self.source_transfers.diagnostics);
            let source = self.source_snapshot(combat_seq, transfer)?;
            let mut stage = LedgerStage::new(self)?;
            for share in source.shares() {
                let row = stage.row(share.destination())?;
                if stage.combat.cards[row].kind != SourceKind::Card {
                    stage.row_play(share.destination())?;
                    stage.combat.generation_triggers = stage
                        .combat
                        .generation_triggers
                        .checked_add(1)
                        .ok_or(SourceFailure::Arithmetic)?;
                }
            }
            stage.commit(self)?;
            if instance == 0 || self.provenance.generated.len() == caps::GENERATED_INSTANCES {
                return Err(SourceFailure::Capacity);
            }
            self.provenance.generated.push(GeneratedSource {
                instance,
                source,
                producer_role: role,
            });
            Ok(())
        })();
        self.source_status(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn card_play_started(
        &mut self,
        combat_seq: u64,
        execution: u64,
        instance: u64,
        id: &str,
        player_slot: i32,
        play_index: i32,
        play_count: i32,
        generation_state: i32,
        transfer: u64,
    ) -> u64 {
        let result = (|| {
            let epoch = self.provenance_epoch(combat_seq)?;
            if play_index < 0 || play_count <= 0 || play_index >= play_count {
                return Err(SourceFailure::Packet);
            }
            let owner = super::super::state::clamp_source_slot(player_slot);
            let generation =
                GenerationState::decode(generation_state, &mut self.source_transfers.diagnostics);
            let source = self.source_snapshot(combat_seq, transfer)?;
            let source = self.card_source(epoch, instance, id, owner, generation, Some(source));
            let mut stage = LedgerStage::new(self)?;
            stage.combat.plays = stage
                .combat
                .plays
                .checked_add(1)
                .ok_or(SourceFailure::Arithmetic)?;
            if matches!(
                generation,
                GenerationState::GeneratedRecorded | GenerationState::GeneratedUnavailable
            ) {
                stage.combat.generated_plays = stage
                    .combat
                    .generated_plays
                    .checked_add(1)
                    .ok_or(SourceFailure::Arithmetic)?;
            } else {
                stage.row_play(source.shares()[0].destination())?;
            }
            stage.commit(self)?;
            self.slot_index(i32::from(owner));
            if execution == 0
                || instance == 0
                || self.provenance.play_serial == PAYLOAD_MAX
                || self
                    .provenance
                    .plays
                    .iter()
                    .filter(|play| play.owner_slot == owner)
                    .count()
                    == caps::ACTIVE_PLAYS_PER_SLOT
            {
                return Err(SourceFailure::Capacity);
            }
            let serial = self.provenance.play_serial + 1;
            self.provenance.plays.push(ActiveSourcePlay {
                serial,
                execution,
                card_instance: instance,
                owner_slot: owner,
                source,
                first_orb_used: false,
            });
            self.provenance.play_serial = serial;
            Ok(Token {
                epoch,
                kind: TokenKind::CardPlay,
                payload: serial,
            }
            .encode())
        })();
        match result {
            Ok(token) => token,
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
                0
            }
        }
    }

    pub(crate) fn card_play_finished(&mut self, play: u64) -> i32 {
        let result = (|| {
            let token = self.provenance_token(play, TokenKind::CardPlay)?;
            let index = self
                .provenance
                .plays
                .iter()
                .position(|play| play.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            let play = self.provenance.plays.remove(index);
            crate::data::persistence::event_log!(
                "  play finished: card {}, execution {}",
                play.card_instance,
                play.execution
            );
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn card_execution_ended(&mut self, combat_seq: u64, execution: u64) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            if execution == 0 {
                return Err(SourceFailure::Packet);
            }
            self.provenance
                .plays
                .retain(|play| play.execution != execution);
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn orb_channeled(&mut self, combat_seq: u64, instance: u64, transfer: u64) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            self.provenance
                .orbs
                .retain(|entry| entry.instance != instance);
            let source = self.source_snapshot(combat_seq, transfer)?;
            if instance == 0 || self.provenance.orbs.len() == caps::ORB_SOURCES {
                return Err(SourceFailure::Capacity);
            }
            self.provenance
                .orbs
                .push(InstanceSource { instance, source });
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn orb_context_begin(
        &mut self,
        combat_seq: u64,
        instance: u64,
        play: u64,
        owner_slot: i32,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let owner = super::super::state::clamp_source_slot(owner_slot);
            if play != 0 {
                let token = self.provenance_token(play, TokenKind::CardPlay)?;
                let play = self
                    .provenance
                    .plays
                    .iter_mut()
                    .find(|play| play.serial == token.payload && play.owner_slot == owner)
                    .ok_or(SourceFailure::Token)?;
                if play.first_orb_used {
                    return Ok(2);
                }
                play.first_orb_used = true;
            }
            if self
                .provenance
                .orbs
                .iter()
                .any(|orb| orb.instance == instance)
            {
                Ok(1)
            } else {
                Ok(0)
            }
        })();
        match result {
            Ok(decision) => decision,
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
                0
            }
        }
    }
}
