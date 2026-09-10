using System;
using System.Collections.Generic;

namespace SpireProfiler;

internal readonly record struct CaptureEpoch(ulong Sequence, object Combat)
{
    internal bool IsAvailable => Sequence > 0 && Sequence <= uint.MaxValue && Combat != null;
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
