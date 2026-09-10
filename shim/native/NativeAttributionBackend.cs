using System;
using MegaCrit.Sts2.Core.Logging;

namespace SpireProfiler;

internal sealed class NativeAttributionBackend : GameAttributionBackend
{
    internal override object CurrentCombat => RunContext.CurrentCombat;
    internal override ulong Capture(ulong epoch, CaptureKind kind, ulong instance, string id, int sourceKind, int slot, GenerationState generation)
        => ProfilerNative.SourceCapture(epoch, (int)kind, instance, id, sourceKind, slot, (int)generation);
    internal override int SourceCount(ulong transfer) => ProfilerNative.SourceCount(transfer);
    internal override ulong SourceDestination(ulong transfer, int index) => ProfilerNative.SourceDestination(transfer, index);
    internal override ulong SourceWeight(ulong transfer, int index) => ProfilerNative.SourceWeight(transfer, index);
    internal override ulong TransferBegin(ulong epoch) => ProfilerNative.SourceTransferBegin(epoch);
    internal override int TransferAdd(ulong transfer, ulong destination, ulong weight) => ProfilerNative.SourceTransferAdd(transfer, destination, weight);
    internal override int TransferSeal(ulong transfer) => ProfilerNative.SourceTransferSeal(transfer);
    internal override int TransferRelease(ulong transfer) => ProfilerNative.SourceTransferRelease(transfer);
    internal override int PowerAttached(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, ulong source)
        => ProfilerNative.PowerAttached(epoch.Sequence, identity, observed.Id, owner, observed.OwnerKind, observed.OwnerSlot, observed.Amount, source);
    internal override int PowerChanged(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, int before, ulong source)
        => ProfilerNative.PowerAmountChanged(epoch.Sequence, identity, observed.Id, owner, observed.OwnerKind, observed.OwnerSlot, before, observed.Amount, source);
    internal override int PowerRemoved(ulong epoch, ulong identity) => ProfilerNative.PowerRemoved(epoch, identity);
    internal override int PowerInvalidate(ulong epoch, ulong identity) => ProfilerNative.PowerProvenanceInvalidate(epoch, identity);
    internal override int CardGenerated(ulong epoch, ulong identity, ulong source, ProducerRole role) => ProfilerNative.CardGenerated(epoch, identity, source, (int)role);
    internal override ulong PlayStarted(ulong epoch, ulong execution, ulong card, string id, int slot, int index, int count, GenerationState generation, ulong source)
        => ProfilerNative.CardPlayStarted(epoch, execution, card, id, slot, index, count, (int)generation, source);
    internal override int PlayFinished(ulong play) => ProfilerNative.CardPlayFinished(play);
    internal override int ExecutionEnded(ulong epoch, ulong execution) => ProfilerNative.CardExecutionEnded(epoch, execution);
    internal override int OrbBegin(ulong epoch, ulong orb, ulong play, int ownerSlot) => ProfilerNative.OrbContextBegin(epoch, orb, play, ownerSlot);
    internal override int OrbChanneled(ulong epoch, ulong orb, ulong source) => ProfilerNative.OrbChanneled(epoch, orb, source);
    internal override ulong DamageBegin(ulong epoch, ulong source, ProducerRole role, DamageSegment segment, ulong target)
        => ProfilerNative.DamageCalculationBegin(epoch, source, (int)role, (int)segment, target);
    internal override int DamageModifier(ulong calculation, ulong source, int amount) => ProfilerNative.DamageModifierContribution(calculation, source, amount);
    internal override int DamageEnemyHit(ulong calculation, ulong dealer, int baseDamage, int strength) => ProfilerNative.DamageCalculationEnemyHit(calculation, dealer, baseDamage, strength);
    internal override int DamageWeak(ulong calculation, ulong source) => ProfilerNative.DamageCalculationWeakSource(calculation, source);
    internal override int DamageAppend(ulong calculation, ResultPacket packet)
        => ProfilerNative.DamageResultAppend(calculation, packet.Total, packet.Unblocked, packet.Blocked, (int)packet.Kind, packet.ReceiverSlot, packet.WeakPrevented);
    internal override int DamageCommit(ulong calculation) => ProfilerNative.DamageCalculationCommit(calculation);
    internal override int DamageAbort(ulong calculation) => ProfilerNative.DamageCalculationAbort(calculation);
    internal override int DamageFallback(ulong epoch, ResultPacket packet)
        => ProfilerNative.DamageUnattributed(epoch, packet.Total, packet.Unblocked, packet.Blocked, (int)packet.Kind, packet.ReceiverSlot, packet.WeakPrevented);
    internal override int BlockGained(ulong epoch, int amount, ulong source, int slot) => ProfilerNative.BlockGained(epoch, amount, source, slot);
    internal override int BlockModifier(ulong epoch, ulong source, int amount, int slot) => ProfilerNative.BlockModifierContribution(epoch, source, amount, slot);
    internal override int Forge(ulong epoch, ulong source, int amount) => ProfilerNative.Forge(epoch, source, amount);
    internal override int OstySummoned(ulong epoch, ulong source, int hp, int slot) => ProfilerNative.OstySummoned(epoch, source, hp, slot);
    internal override int OstyKilled(ulong epoch, int slot, ulong play) => ProfilerNative.OstyKilled(epoch, slot, play);
    internal override int BuffMitigation(ulong epoch, ulong source, int prevented) => ProfilerNative.BuffMitigation(epoch, source, prevented);
    internal override ulong DoomBegin(ulong epoch) => ProfilerNative.DoomBatchBegin(epoch);
    internal override int DoomTarget(ulong batch, ulong creature, ulong power, int hp) => ProfilerNative.DoomTargetCapture(batch, creature, power, hp);
    internal override int DoomComplete(ulong batch) => ProfilerNative.DoomKillsCompleted(batch);
    internal override int DoomAbort(ulong batch) => ProfilerNative.DoomBatchAbort(batch);
    internal override int TurnStarted(ulong epoch) => ProfilerNative.TurnStarted(epoch);
    internal override int BlockCleared(ulong epoch, int slot) => ProfilerNative.BlockPoolClear(epoch, slot);
    internal override int PlayerDied(ulong epoch, int slot) => ProfilerNative.PlayerDied(epoch, slot);
    internal override int PotionUsed(ulong epoch) => ProfilerNative.PotionUsed(epoch);
    internal override ulong CombatStarted(string encounter, string type) => ProfilerNative.CombatStarted(encounter, type);
    internal override int CombatEnded(ulong epoch) => ProfilerNative.CombatEnded(epoch);
    internal override void Diagnostic(string category, Exception error) => Log.Error($"[SpireProfiler] attribution {category}: {error?.Message}");
}
