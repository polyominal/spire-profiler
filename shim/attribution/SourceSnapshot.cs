using System;
using System.Collections.Generic;

namespace SpireProfiler;

// One managed object owns each exported native handle. Async frames share that
// object; native provenance retains independent Rc snapshots. Only the game
// thread sweeps dead weak entries, after GC, so no finalizer calls into Rust.
internal sealed class SourceSnapshot
{
    private static Dictionary<ulong, WeakReference<SourceSnapshot>> owned = new();
    private static int collections;
    internal ulong Epoch { get; }
    internal ulong Handle { get; }
    internal static readonly SourceSnapshot Unavailable = new(0, 0);
    private SourceSnapshot(ulong epoch, ulong handle) { Epoch = epoch; Handle = handle; }
    internal static void Reset()
    {
        owned = new();
        collections = GC.CollectionCount(0);
    }
    internal static SourceSnapshot Own(ulong epoch, ulong handle)
    {
        if (handle == 0) return Unavailable;
        if (owned.TryGetValue(handle, out var weak) && weak.TryGetTarget(out var existing)) return existing;
        var source = new SourceSnapshot(epoch, handle);
        owned[handle] = new(source);
        return source;
    }
    internal static void Collect()
    {
        int current = GC.CollectionCount(0);
        if (current == collections) return;
        collections = current;
        List<ulong> dead = null;
        foreach (var pair in owned)
            if (!pair.Value.TryGetTarget(out _)) (dead ??= new()).Add(pair.Key);
        if (dead == null) return;
        foreach (ulong handle in dead)
        {
            if (CaptureRuntime.Backend.SourceRelease(handle) != 1) CaptureRuntime.Fail("source-release");
            owned.Remove(handle);
        }
    }
}
