using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;

namespace SpireProfiler;

internal enum ProducerRole { Unknown = 0, Card = 1, Power = 2, Relic = 3, Potion = 4, Orb = 5 }
internal enum DamageSegment { Direct = 0, Attributed = 1, Modifier = 2 }
internal enum CaptureKind { Unknown = 0, CardInstance = 1, PowerInstance = 2, OrbInstance = 3, DirectModel = 4, WeakHead = 5 }
internal enum GenerationState { Ordinary = 0, GeneratedRecorded = 1, GeneratedUnavailable = 2, Unclassified = 3 }
internal enum ResultKind { Outgoing = 0, Incoming = 1, SelfDamage = 2, OstyDealt = 3, OstyAbsorbed = 4 }
internal readonly record struct SourceShare(ulong Destination, ulong Weight);
internal sealed class SourceSnapshot
{
    // A source can span at most the native contract's 128 distinct roots.
    internal const int MaxDestinations = 128;
    private readonly SourceShare[] shares;
    internal ulong Epoch { get; }
    internal int Count => shares.Length;
    internal SourceShare this[int index] => shares[index];
    internal static readonly SourceSnapshot Unavailable = new(0, Array.Empty<SourceShare>());
    private SourceSnapshot(ulong epoch, SourceShare[] entries) { Epoch = epoch; shares = entries; }
    internal static SourceSnapshot Create(ulong epoch, IReadOnlyList<SourceShare> entries)
    {
        if (epoch == 0 || epoch > uint.MaxValue || entries.Count < 1 || entries.Count > MaxDestinations)
            throw new InvalidOperationException("Invalid source shape");
        var copy = new SourceShare[entries.Count];
        ulong total = 0, gcd = 0;
        for (int i = 0; i < entries.Count; i++)
        {
            var share = entries[i];
            var tag = share.Destination & 7;
            var payload = (uint)share.Destination >> 3;
            if (share.Weight == 0 || share.Destination >> 32 != epoch || tag > 1 || (tag == 1 && payload > 4))
                throw new InvalidOperationException("Invalid source destination or weight");
            for (int j = 0; j < i; j++)
                if (copy[j].Destination == share.Destination) throw new InvalidOperationException("Duplicate source destination");
            total = checked(total + share.Weight);
            gcd = Gcd(gcd, share.Weight);
            copy[i] = share;
        }
        if (gcd != 1) throw new InvalidOperationException("Source is not normalized");
        return new SourceSnapshot(epoch, copy);
    }
    internal static ulong Gcd(ulong a, ulong b) { while (b != 0) { (a, b) = (b, a % b); } return a; }
    internal static SourceSnapshot Unknown(ulong epoch) => Create(epoch, new[] { new SourceShare((epoch << 32) | 33, 1) });
}

internal readonly record struct CaptureEpoch(ulong Sequence, object Combat)
{
    internal bool IsAvailable => Sequence > 0 && Sequence <= uint.MaxValue && Combat != null;
}
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

internal static class CaptureRuntime
{
    internal static AttributionBackend Backend { get; private set; }
    internal static CaptureEpoch Epoch { get; private set; }
    private static int thread;
    private static readonly HashSet<string> diagnostics = new();
    private static volatile bool wrongThreadObserved;
    internal static bool WrongThreadObserved => wrongThreadObserved;
    internal static bool OnThread
    {
        get
        {
            if (Environment.CurrentManagedThreadId == thread)
            {
                if (WrongThreadObserved) Fail("wrong-thread-gameplay");
                return true;
            }
            wrongThreadObserved = true;
            return false;
        }
    }
    internal static void Initialize(AttributionBackend backend)
    {
        if (thread != 0 && !OnThread) return;
        thread = Environment.CurrentManagedThreadId;
        Backend = backend;
        InvalidateEpoch();
    }
    internal static void Register(AttributionBackend backend, ulong sequence, object combat)
    {
        if (thread == 0) Initialize(backend);
        if (!OnThread) return;
        Backend = backend;
        Epoch = new CaptureEpoch(sequence, combat);
        diagnostics.Clear();
        IdentityCapture.NewEpoch();
        TemporalPowerCapture.Clear();
    }
    internal static void InvalidateEpoch()
    {
        if (!OnThread) return;
        Epoch = default;
        IdentityCapture.CancelPreparation();
        TemporalPowerCapture.Clear();
    }
    internal static bool Stale(CaptureEpoch captured) => captured.Sequence != 0
        && (captured.Sequence != Epoch.Sequence || !ReferenceEquals(captured.Combat, Epoch.Combat));
    internal static CaptureEpoch EntryEpoch(CaptureEpoch saved = default)
    {
        if (Stale(saved)) return saved;
        foreach (var inherited in new[] { FlowCapture.Current.Epoch, ProvenanceCapture.Pending.Epoch,
            PlayCapture.Execution.Epoch, PlayCapture.Current?.Epoch ?? default, DamageCapture.Current.Epoch, CommandCapture.Current.Epoch })
            if (Stale(inherited)) return inherited;
        return Epoch;
    }
    internal static bool Valid(CaptureEpoch epoch)
    {
        if (!OnThread) return false;
        return epoch.IsAvailable && epoch.Sequence == Epoch.Sequence && ReferenceEquals(epoch.Combat, Epoch.Combat)
            && ReferenceEquals(epoch.Combat, Backend.CurrentCombat);
    }
    internal static void Fail(string category, Exception error = null)
    {
        if (Environment.CurrentManagedThreadId != thread) { wrongThreadObserved = true; return; }
        if (diagnostics.Add(category))
            try { Backend?.Diagnostic(category, error); } catch (Exception) { }
    }
    internal static SourceSnapshot Copy(CaptureEpoch epoch, CaptureKind kind, ulong identity, string id = "", int sourceKind = 5, int slot = 4, GenerationState generation = GenerationState.Unclassified)
    {
        ulong lease = 0;
        try
        {
            if (!Valid(epoch)) return SourceSnapshot.Unavailable;
            lease = Backend.Capture(epoch.Sequence, kind, identity, id, sourceKind, slot, generation);
            if (lease == 0) return SourceSnapshot.Unavailable;
            int count = Backend.SourceCount(lease);
            if (count < 1 || count > SourceSnapshot.MaxDestinations) throw new InvalidOperationException("Invalid source count");
            var entries = new SourceShare[count];
            for (int i = 0; i < count; i++) entries[i] = new(Backend.SourceDestination(lease, i), Backend.SourceWeight(lease, i));
            return SourceSnapshot.Create(epoch.Sequence, entries);
        }
        catch (Exception ex) { Fail("source-copy", ex); return SourceSnapshot.Unavailable; }
        finally { Release(lease); }
    }
    internal static T Upload<T>(CaptureEpoch epoch, SourceSnapshot source, Func<ulong, T> consume, T unavailable = default)
    {
        ulong lease = 0;
        try
        {
            if (!Valid(epoch)) return unavailable;
            if (source.Epoch == 0) return consume(0);
            if (source.Epoch != epoch.Sequence) return unavailable;
            try
            {
                lease = Backend.TransferBegin(epoch.Sequence);
                if (lease == 0) return consume(0);
                for (int i = 0; i < source.Count; i++)
                    if (Backend.TransferAdd(lease, source[i].Destination, source[i].Weight) != 1)
                        throw new InvalidOperationException("Source upload rejected");
                if (Backend.TransferSeal(lease) != 1) throw new InvalidOperationException("Source seal rejected");
            }
            catch (Exception ex) { Fail("source-upload", ex); Release(lease); lease = 0; }
            return consume(lease);
        }
        finally { Release(lease); }
    }
    private static void Release(ulong lease)
    {
        if (lease == 0) return;
        try { if (Backend.TransferRelease(lease) != 1) Fail("source-release"); }
        catch (Exception ex) { Fail("source-release", ex); }
    }
}

internal sealed class IdentityMetadata
{
    internal readonly ulong Identity;
    internal ulong Epoch;
    internal object Preparation;
    internal GenerationState Generation;
    internal bool Dirty;
    internal SourceSnapshot Detached;
    internal IdentityMetadata(ulong identity) { Identity = identity; }
}
internal static class IdentityCapture
{
    // Limits newly encountered objects and wrappers without retaining failed object keys.
    internal const int PerCombat = 4096;
    private static readonly ConditionalWeakTable<object, IdentityMetadata> identities = new();
    private static ulong nextIdentity, nextExecution;
    private static int allocated, executions;
    private static object pendingPreparation, activePreparation;
    internal static bool Preparing { get; private set; }
    internal static void PrepareCombat()
    {
        if (!CaptureRuntime.OnThread) return;
        Preparing = true;
        allocated = 0;
        executions = 0;
        pendingPreparation = new object();
    }
    internal static void CancelPreparation() { Preparing = false; pendingPreparation = null; }
    internal static void NewEpoch()
    {
        activePreparation = Preparing ? pendingPreparation : null;
        if (!Preparing) { allocated = 0; executions = 0; }
        CancelPreparation();
    }
    internal static void GeneratedBeforeReady(object card)
    {
        if (card == null || !Preparing || pendingPreparation == null || !CaptureRuntime.OnThread) return;
        if (!identities.TryGetValue(card, out var metadata))
        {
            if (allocated == PerCombat || nextIdentity == ulong.MaxValue) { CaptureRuntime.Fail("identity-cap"); return; }
            metadata = new IdentityMetadata(++nextIdentity);
            identities.Add(card, metadata);
            allocated++;
        }
        metadata.Epoch = 0;
        metadata.Preparation = pendingPreparation;
        metadata.Generation = GenerationState.GeneratedUnavailable;
        metadata.Dirty = false;
        metadata.Detached = null;
    }
    internal static IdentityMetadata Get(object model, CaptureEpoch epoch)
    {
        if (model == null || !CaptureRuntime.Valid(epoch)) return null;
        if (!identities.TryGetValue(model, out var metadata))
        {
            if (allocated == PerCombat || nextIdentity == ulong.MaxValue) { CaptureRuntime.Fail("identity-cap"); return null; }
            metadata = new IdentityMetadata(++nextIdentity);
            identities.Add(model, metadata);
            allocated++;
        }
        if (metadata.Epoch != epoch.Sequence)
        {
            metadata.Epoch = epoch.Sequence;
            metadata.Generation = metadata.Preparation != null && ReferenceEquals(metadata.Preparation, activePreparation) ? GenerationState.GeneratedUnavailable : GenerationState.Ordinary;
            metadata.Preparation = null;
            metadata.Dirty = false;
            metadata.Detached = null;
        }
        return metadata;
    }
    internal static ulong Execution(CaptureEpoch epoch)
    {
        if (!CaptureRuntime.Valid(epoch)) return 0;
        if (executions == PerCombat || nextExecution == ulong.MaxValue) { CaptureRuntime.Fail("execution-cap"); return 0; }
        executions++;
        return ++nextExecution;
    }
}
