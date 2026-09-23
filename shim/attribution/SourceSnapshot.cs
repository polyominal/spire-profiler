namespace SpireProfiler;

// Combat-owned native provenance. Managed capture retains identity, never weights.
internal sealed class SourceSnapshot
{
    internal ulong Epoch { get; }
    internal ulong Handle { get; }
    internal static readonly SourceSnapshot Unavailable = new(0, 0);
    internal SourceSnapshot(ulong epoch, ulong handle) { Epoch = epoch; Handle = handle; }
}
