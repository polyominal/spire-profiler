//! Instance ancestry and exact execution-owned play lifetimes. Physical playing
//! slots govern orb timing; immutable snapshots govern creditor ownership.

use super::*;
use crate::data::state::SourceKind;

impl State {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn card_source_into_incoming(
        &mut self,
        epoch: CombatEpoch,
        instance: u64,
        id: &str,
        slot: SourceSlot,
        generation: GenerationState,
        given: bool,
    ) {
        if generation == GenerationState::GeneratedUnavailable {
            self.provenance
                .generated
                .retain(|entry| entry.instance != instance);
            self.provenance.incoming.set_unknown(epoch);
            return;
        }
        let generated = self
            .provenance
            .generated
            .iter()
            .find(|entry| entry.instance == instance);
        match generation {
            GenerationState::Ordinary if instance != 0 && !id.is_empty() && generated.is_none() => {
                if given {
                    if self.provenance.incoming.shares().len() != 1 {
                        self.source_transfers
                            .diagnostics
                            .report(SourceFailure::Packet);
                        self.provenance.incoming.set_unknown(epoch);
                    }
                } else {
                    let Some(combat) = Combat::active_mut(&mut self.current) else {
                        self.provenance.incoming.set_unknown(epoch);
                        return;
                    };
                    let destination = crate::data::ledger::get_or_create_card_kind(
                        combat,
                        slot,
                        id,
                        SourceKind::Card,
                    )
                    .map_or(Destination::Unknown(slot), |row| {
                        if combat.cards[row].kind == SourceKind::Unknown {
                            Destination::Unknown(slot)
                        } else {
                            Destination::Row(row)
                        }
                    });
                    self.provenance.incoming.set_single(epoch, destination);
                }
            }
            GenerationState::Ordinary => {
                self.source_transfers
                    .diagnostics
                    .report(SourceFailure::Packet);
                self.provenance.incoming.set_unknown(epoch);
            }
            GenerationState::GeneratedRecorded => {
                if let Some(entry) = generated {
                    crate::data::persistence::event_log!(
                        "  generated instance {instance}, supplier role {:?}",
                        entry.producer_role
                    );
                    if !given {
                        self.provenance.incoming.clone_from(&entry.source);
                    }
                } else {
                    self.provenance.incoming.set_unknown(epoch);
                }
            }
            _ => self.provenance.incoming.set_unknown(epoch),
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
            if instance != 0 && self.provenance.generated.len() == caps::GENERATED_INSTANCES {
                return Err(SourceFailure::Capacity);
            }
            let role = ProducerRole::decode(producer_role, &mut self.source_transfers.diagnostics);
            self.capture_source_snapshot(combat_seq, transfer)?;
            let mut stage = LedgerStage::new(self)?;
            for share in self.provenance.incoming.shares() {
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
            // Missing card identity prevents ancestry retention, not the observed trigger.
            if instance == 0 {
                return Err(SourceFailure::Capacity);
            }
            let entry = self
                .provenance
                .generated
                .vacant_mut()
                .expect("generated admission leaves one initialized slot");
            entry.instance = instance;
            entry.source.clone_from(&self.provenance.incoming);
            entry.producer_role = role;
            self.provenance.generated.activate();
            Ok(())
        })();
        self.source_status(result)
    }

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "keep frame admission before ledger publication and missing-identity counting explicit"
    )]
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
            if execution != 0
                && instance != 0
                && (self.provenance.play_serial == PAYLOAD_MAX
                    || self.provenance.plays.len() == caps::ACTIVE_PLAYS
                    || self
                        .provenance
                        .plays
                        .iter()
                        .filter(|play| play.owner_slot == owner)
                        .count()
                        == caps::ACTIVE_PLAYS_PER_SLOT)
            {
                return Err(SourceFailure::Capacity);
            }
            let generation =
                GenerationState::decode(generation_state, &mut self.source_transfers.diagnostics);
            self.capture_source_snapshot(combat_seq, transfer)?;
            self.card_source_into_incoming(epoch, instance, id, owner, generation, true);
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
                stage.row_play(self.provenance.incoming.shares()[0].destination())?;
            }
            stage.commit(self)?;
            self.slot_index(i32::from(owner));
            // History confirms the play even when no identity can own its frame.
            if execution == 0 || instance == 0 {
                return Err(SourceFailure::Capacity);
            }
            let serial = self.provenance.play_serial + 1;
            let entry = self
                .provenance
                .plays
                .vacant_mut()
                .expect("play admission leaves one initialized slot");
            entry.serial = serial;
            entry.execution = execution;
            entry.card_instance = instance;
            entry.owner_slot = owner;
            entry.source.clone_from(&self.provenance.incoming);
            entry.first_orb_used = false;
            self.provenance.plays.activate();
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
            let play = &self.provenance.plays[index];
            crate::data::persistence::event_log!(
                "  play finished: card {}, execution {}",
                play.card_instance,
                play.execution
            );
            self.provenance.plays.remove(index);
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
            if instance == 0 || self.provenance.orbs.len() == caps::ORB_SOURCES {
                return Err(SourceFailure::Capacity);
            }
            self.capture_source_snapshot(combat_seq, transfer)?;
            let entry = self
                .provenance
                .orbs
                .vacant_mut()
                .expect("orb admission leaves one initialized slot");
            entry.instance = instance;
            entry.source.clone_from(&self.provenance.incoming);
            self.provenance.orbs.activate();
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
