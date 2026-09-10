using System.Runtime.CompilerServices;

namespace SpireProfiler;

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
