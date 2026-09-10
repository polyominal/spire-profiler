using System;

namespace SpireProfiler;

internal enum ProducerRole { Unknown = 0, Card = 1, Power = 2, Relic = 3, Potion = 4, Orb = 5 }
internal enum DamageSegment { Direct = 0, Attributed = 1, Modifier = 2 }
internal enum CaptureKind { Unknown = 0, CardInstance = 1, PowerInstance = 2, OrbInstance = 3, DirectModel = 4, WeakHead = 5 }
internal enum GenerationState { Ordinary = 0, GeneratedRecorded = 1, GeneratedUnavailable = 2, Unclassified = 3 }
internal enum ResultKind { Outgoing = 0, Incoming = 1, SelfDamage = 2, OstyDealt = 3, OstyAbsorbed = 4 }

internal readonly record struct ModelDescriptor(CaptureKind Kind, ProducerRole Role, string Id, int SourceKind, int Slot, object Owner = null, bool Poison = false, object Combat = null);
internal readonly record struct CreatureDescriptor(bool Player, bool Osty, int Slot, object Combat);
internal readonly record struct PowerObservation(object Power, object Owner, string Id, int OwnerKind, int OwnerSlot, int Amount, bool Attached);
internal readonly record struct ResultPacket(int Total, int Unblocked, int Blocked, ResultKind Kind, int ReceiverSlot, int WeakPrevented = 0);

// This boundary is injected before installation; it never owns a game Task.
internal abstract class AttributionBackend
{
    internal abstract object CurrentCombat { get; }
    internal abstract ModelDescriptor Describe(object model);
    internal abstract CreatureDescriptor DescribeCreature(object creature);
    internal abstract PowerObservation ObservePower(object power, object owner = null);
    internal virtual bool TemporaryPower(object power) => false;
    internal abstract ulong Capture(ulong epoch, CaptureKind kind, ulong instance, string id, int sourceKind, int slot, GenerationState generation);
    internal abstract int SourceCount(ulong transfer);
    internal abstract ulong SourceDestination(ulong transfer, int index);
    internal abstract ulong SourceWeight(ulong transfer, int index);
    internal abstract ulong TransferBegin(ulong epoch);
    internal abstract int TransferAdd(ulong transfer, ulong destination, ulong weight);
    internal abstract int TransferSeal(ulong transfer);
    internal abstract int TransferRelease(ulong transfer);
    internal virtual int PowerAttached(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, ulong source) => 0;
    internal virtual int PowerChanged(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, int before, ulong source) => 0;
    internal virtual int PowerRemoved(ulong epoch, ulong identity) => 0;
    internal virtual int PowerInvalidate(ulong epoch, ulong identity) => 0;
    internal virtual int CardGenerated(ulong epoch, ulong identity, ulong source, ProducerRole role) => 0;
    internal virtual ulong PlayStarted(ulong epoch, ulong execution, ulong card, string id, int slot, int index, int count, GenerationState generation, ulong source) => 0;
    internal virtual int PlayFinished(ulong play) => 0;
    internal virtual int ExecutionEnded(ulong epoch, ulong execution) => 0;
    internal virtual int OrbChanneled(ulong epoch, ulong orb, ulong source) => 0;
    internal virtual int OrbBegin(ulong epoch, ulong orb, ulong play, int ownerSlot) => 0;
    internal virtual ulong DamageBegin(ulong epoch, ulong source, ProducerRole role, DamageSegment segment, ulong target) => 0;
    internal virtual int DamageModifier(ulong calculation, ulong source, int amount) => 0;
    internal virtual int DamageEnemyHit(ulong calculation, ulong dealer, int baseDamage, int strength) => 0;
    internal virtual int DamageWeak(ulong calculation, ulong source) => 0;
    internal virtual int DamageAppend(ulong calculation, ResultPacket packet) => 0;
    internal virtual int DamageCommit(ulong calculation) => 0;
    internal virtual int DamageAbort(ulong calculation) => 0;
    internal virtual int DamageFallback(ulong epoch, ResultPacket packet) => 0;
    internal virtual int BlockGained(ulong epoch, int amount, ulong source, int slot) => 0;
    internal virtual int BlockModifier(ulong epoch, ulong source, int amount, int slot) => 0;
    internal virtual int Forge(ulong epoch, ulong source, int amount) => 0;
    internal virtual int OstySummoned(ulong epoch, ulong source, int hp, int slot) => 0;
    internal virtual int OstyKilled(ulong epoch, int slot, ulong play) => 0;
    internal virtual int BuffMitigation(ulong epoch, ulong source, int prevented) => 0;
    internal virtual ulong DoomBegin(ulong epoch) => 0;
    internal virtual int DoomTarget(ulong batch, ulong creature, ulong power, int hp) => 0;
    internal virtual int DoomComplete(ulong batch) => 0;
    internal virtual int DoomAbort(ulong batch) => 0;
    internal virtual int TurnStarted(ulong epoch) => 0;
    internal virtual int BlockCleared(ulong epoch, int slot) => 0;
    internal virtual int PlayerDied(ulong epoch, int slot) => 0;
    internal virtual int PotionUsed(ulong epoch) => 0;
    internal virtual ulong CombatStarted(string encounter, string type) => 0;
    internal virtual int CombatEnded(ulong epoch) => 0;
    internal virtual void Diagnostic(string category, Exception error) { }
}
