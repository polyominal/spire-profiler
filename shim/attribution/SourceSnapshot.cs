using System;
using System.Collections.Generic;

namespace SpireProfiler;

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
