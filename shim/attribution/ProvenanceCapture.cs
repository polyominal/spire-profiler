using System;
using System.Linq;
using System.Reflection;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Combat.History;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;

namespace SpireProfiler;

internal sealed record PendingPower(CaptureEpoch Epoch, object Power, object Target, SourceSnapshot Source)
{
    internal static readonly PendingPower Barrier = new(default, null, null, SourceSnapshot.Unavailable);
}
internal enum MutationOperation { Attachment, AmountChange, Removal }
internal abstract record MutationState
{
    private MutationState() { }
    internal static readonly MutationState Inactive = new Idle();
    private sealed record Idle : MutationState;
    internal abstract record Tracked(CaptureEpoch Epoch, IdentityMetadata Metadata) : MutationState;
    internal sealed record ObservationFailed(CaptureEpoch Epoch, IdentityMetadata Metadata) : Tracked(Epoch, Metadata);
    internal sealed record Observed(CaptureEpoch Epoch, IdentityMetadata Metadata, MutationOperation Operation,
        PowerObservation Before, SourceSnapshot Source, bool PriorDirty) : Tracked(Epoch, Metadata);
}
internal static class ProvenanceCapture
{
    private static readonly AsyncLocal<PendingPower> pending = new();
    internal static PendingPower Pending { get => pending.Value ?? PendingPower.Barrier; private set => pending.Value = value; }
    internal static bool Matching(object power) => ReferenceEquals(power, Pending.Power) && CaptureRuntime.Valid(Pending.Epoch);
    internal static void CommandPrefix(MethodBase __originalMethod, object[] __args, out PendingPower __state)
    {
        __state = Pending;
        Pending = PendingPower.Barrier;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Epoch);
            Pending = PendingPower.Barrier with { Epoch = epoch };
            if (!CaptureRuntime.Valid(epoch)) return;
            bool apply = __originalMethod.Name == "Apply";
            object power = __args[1], target = apply ? __args[2] : null, card = __args[apply ? 5 : 4];
            Pending = new(epoch, power, target, FlowCapture.Supplied(card, epoch));
        }
        catch (Exception ex) { CaptureRuntime.Fail("power-command", ex); }
    }
    internal static void CommandFinalizer(PendingPower __state) { if (__state != null) Pending = __state; }
    internal static void MutationPrefix(object __instance, MethodBase __originalMethod, object[] __args, out MutationState __state)
    {
        __state = MutationState.Inactive;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch)) return;
            var operation = __originalMethod.Name switch
            {
                "ApplyPowerInternal" => MutationOperation.Attachment,
                "SetAmount" => MutationOperation.AmountChange,
                "RemovePowerInternal" => MutationOperation.Removal,
                _ => throw new InvalidOperationException("Unsupported power mutation")
            };
            bool amount = operation == MutationOperation.AmountChange;
            object power = amount ? __instance : __args[0];
            var metadata = IdentityCapture.Get(power, epoch);
            if (metadata == null) return;
            __state = new MutationState.ObservationFailed(epoch, metadata);
            var before = CaptureRuntime.Backend.ObservePower(power, amount ? null : __instance);
            __state = new MutationState.Observed(epoch, metadata, operation, before, SourceSnapshot.Unavailable, metadata.Dirty);
            var source = operation == MutationOperation.Removal ? FlowCapture.Source(power, epoch)
                : Matching(power) ? Pending.Source : FlowCapture.Supplied(null, epoch);
            __state = new MutationState.Observed(epoch, metadata, operation, before, source, metadata.Dirty);
            if (operation == MutationOperation.Removal) metadata.Detached = source;
        }
        catch (Exception ex) { CaptureRuntime.Fail("power-observe-prefix", ex); }
    }
    internal static void MutationFinalizer(MutationState __state)
    {
        var tracked = __state as MutationState.Tracked;
        try
        {
            if (tracked == null || !CaptureRuntime.Valid(tracked.Epoch)) return;
            if (tracked is MutationState.ObservationFailed)
            {
                tracked.Metadata.Dirty = true;
                CaptureRuntime.Backend.PowerInvalidate(tracked.Epoch.Sequence, tracked.Metadata.Identity);
                return;
            }
            if (tracked is not MutationState.Observed observed) return;
            var before = observed.Before;
            var after = CaptureRuntime.Backend.ObservePower(before.Power, before.Owner);
            if (observed.Operation == MutationOperation.Removal)
            {
                if (before.Attached && !after.Attached)
                {
                    tracked.Metadata.Detached = observed.Source;
                    tracked.Metadata.Dirty = true;
                    if (CaptureRuntime.Backend.PowerRemoved(tracked.Epoch.Sequence, tracked.Metadata.Identity) == 1) tracked.Metadata.Dirty = false;
                    TemporalPowerCapture.RemovePower(tracked.Metadata.Identity);
                }
                else if (after.Attached) tracked.Metadata.Detached = null;
                return;
            }
            bool attachment = observed.Operation == MutationOperation.Attachment;
            if (!after.Attached || (attachment ? before.Attached : !before.Attached)) return;
            tracked.Metadata.Dirty = true;
            try
            {
                if (observed.PriorDirty && CaptureRuntime.Backend.PowerInvalidate(tracked.Epoch.Sequence, tracked.Metadata.Identity) != 1)
                    throw new InvalidOperationException("Prior dirty provenance could not be invalidated");
                var owner = IdentityCapture.Get(after.Owner, tracked.Epoch);
                if (owner == null) throw new InvalidOperationException("Unavailable power owner identity");
                int status = CaptureRuntime.Upload(tracked.Epoch, observed.Source, transfer => attachment
                    ? CaptureRuntime.Backend.PowerAttached(tracked.Epoch, tracked.Metadata.Identity, owner.Identity, after, transfer)
                    : CaptureRuntime.Backend.PowerChanged(tracked.Epoch, tracked.Metadata.Identity, owner.Identity, after, before.Amount, transfer));
                if (status != 1) throw new InvalidOperationException("Observed power mutation rejected");
                tracked.Metadata.Dirty = false;
                tracked.Metadata.Detached = null;
            }
            catch (Exception ex)
            {
                CaptureRuntime.Fail("power-observe-update", ex);
                try { CaptureRuntime.Backend.PowerInvalidate(tracked.Epoch.Sequence, tracked.Metadata.Identity); }
                catch (Exception invalidate) { CaptureRuntime.Fail("power-invalidate", invalidate); }
            }
        }
        catch (Exception ex)
        {
            if (tracked != null)
            {
                tracked.Metadata.Dirty = true;
                try { if (CaptureRuntime.Valid(tracked.Epoch)) CaptureRuntime.Backend.PowerInvalidate(tracked.Epoch.Sequence, tracked.Metadata.Identity); }
                catch (Exception invalidate) { CaptureRuntime.Fail("power-invalidate", invalidate); }
            }
            CaptureRuntime.Fail("power-observe-finalizer", ex);
        }
    }
    internal static void Generated(object card)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch)) return;
            var metadata = IdentityCapture.Get(card, epoch);
            if (metadata != null) metadata.Generation = GenerationState.GeneratedUnavailable;
            var source = FlowCapture.Supplied(null, epoch);
            int status = CaptureRuntime.Upload(epoch, source, transfer => CaptureRuntime.Backend.CardGenerated(epoch.Sequence, metadata?.Identity ?? 0, transfer, FlowCapture.Current.Role));
            if (metadata != null && status == 1) metadata.Generation = GenerationState.GeneratedRecorded;
        }
        catch (Exception ex) { CaptureRuntime.Fail("card-generated", ex); }
    }
    internal static void GeneratedPrefix(object[] __args)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (IdentityCapture.Preparing && CaptureRuntime.OnThread && ReferenceEquals(__args[0], CaptureRuntime.Backend.CurrentCombat))
                IdentityCapture.GeneratedBeforeReady(__args[1]);
            else if (CaptureRuntime.Valid(epoch) && ReferenceEquals(__args[0], epoch.Combat)) Generated(__args[1]);
        }
        catch (Exception ex) { CaptureRuntime.Fail("card-generated-history", ex); }
    }
    internal static void Install(Harmony harmony)
    {
        var apply = AccessTools.DeclaredMethod(typeof(PowerCmd), "Apply", new[] { typeof(PlayerChoiceContext), typeof(PowerModel), typeof(Creature), typeof(decimal), typeof(Creature), typeof(CardModel), typeof(bool) });
        var change = AccessTools.DeclaredMethod(typeof(PowerCmd), "ModifyAmount", new[] { typeof(PlayerChoiceContext), typeof(PowerModel), typeof(decimal), typeof(Creature), typeof(CardModel), typeof(bool) });
        foreach (var target in new[] { apply, change }) CapturePatches.Patch(harmony, FlowCapture.DeclaredMethod(target), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(CommandPrefix)), finalizer: new HarmonyMethod(typeof(ProvenanceCapture), nameof(CommandFinalizer)));
        foreach (var target in new[] { AccessTools.DeclaredMethod(typeof(Creature), "ApplyPowerInternal"), AccessTools.DeclaredMethod(typeof(Creature), "RemovePowerInternal"), AccessTools.DeclaredMethod(typeof(PowerModel), "SetAmount") })
            PatchMutation(harmony, target);
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatHistory), "CardGenerated"), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(GeneratedPrefix)));
    }
    internal static void PatchMutation(Harmony harmony, MethodInfo target)
        => CapturePatches.Patch(harmony, FlowCapture.DeclaredMethod(target), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(MutationPrefix)), finalizer: new HarmonyMethod(typeof(ProvenanceCapture), nameof(MutationFinalizer)));
}
