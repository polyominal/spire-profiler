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
        var producer = FlowCapture.Current.Epoch;
        var pending = ProvenanceCapture.Pending.Epoch;
        var execution = PlayCapture.Execution.Epoch;
        var play = PlayCapture.Current?.Epoch ?? default;
        var damage = DamageCapture.Current.Epoch;
        var command = CommandCapture.Current.Epoch;
        if (Stale(producer)) return producer;
        if (Stale(pending)) return pending;
        if (Stale(execution)) return execution;
        if (Stale(play)) return play;
        if (Stale(damage)) return damage;
        if (Stale(command)) return command;
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
    internal static SourceSnapshot CaptureSource(CaptureEpoch epoch, CaptureKind kind, ulong identity, string id = "", int sourceKind = 5, int slot = 4, GenerationState generation = GenerationState.Unclassified)
    {
        try
        {
            if (!Valid(epoch)) return SourceSnapshot.Unavailable;
            ulong source = Backend.Capture(epoch.Sequence, kind, identity, id, sourceKind, slot, generation);
            return source == 0 ? SourceSnapshot.Unavailable : new SourceSnapshot(epoch.Sequence, source);
        }
        catch (Exception ex) { Fail("source-capture", ex); return SourceSnapshot.Unavailable; }
    }
    internal static T WithSource<T>(CaptureEpoch epoch, SourceSnapshot source, Func<ulong, T> consume, T unavailable = default)
    {
        if (!Valid(epoch)) return unavailable;
        if (source.Epoch != 0 && source.Epoch != epoch.Sequence) return unavailable;
        return consume(source.Handle);
    }
}
