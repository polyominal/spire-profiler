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
internal readonly record struct MutationState(CaptureEpoch Epoch, PowerObservation Before, SourceSnapshot Source, IdentityMetadata Metadata, bool PriorDirty, bool Captured = false);
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
        __state = default;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch)) return;
            bool amount = __originalMethod.Name == "SetAmount";
            object power = amount ? __instance : __args[0];
            var metadata = IdentityCapture.Get(power, epoch);
            if (metadata == null) return;
            __state = new(epoch, default, SourceSnapshot.Unavailable, metadata, metadata.Dirty);
            var before = CaptureRuntime.Backend.ObservePower(power, amount ? null : __instance);
            __state = new(epoch, before, SourceSnapshot.Unavailable, metadata, metadata.Dirty, true);
            var source = __originalMethod.Name == "RemovePowerInternal" ? FlowCapture.Source(power, epoch)
                : Matching(power) ? Pending.Source : FlowCapture.Supplied(null, epoch);
            __state = new(epoch, before, source, metadata, metadata.Dirty, true);
            if (__originalMethod.Name == "RemovePowerInternal") metadata.Detached = source;
        }
        catch (Exception ex) { CaptureRuntime.Fail("power-observe-prefix", ex); }
    }
    internal static void MutationFinalizer(MethodBase __originalMethod, MutationState __state)
    {
        try
        {
            if (__state.Metadata == null || !CaptureRuntime.Valid(__state.Epoch)) return;
            if (!__state.Captured)
            {
                __state.Metadata.Dirty = true;
                CaptureRuntime.Backend.PowerInvalidate(__state.Epoch.Sequence, __state.Metadata.Identity);
                return;
            }
            var before = __state.Before;
            var after = CaptureRuntime.Backend.ObservePower(before.Power, before.Owner);
            if (__originalMethod.Name == "RemovePowerInternal")
            {
                if (before.Attached && !after.Attached)
                {
                    __state.Metadata.Detached = __state.Source;
                    __state.Metadata.Dirty = true;
                    if (CaptureRuntime.Backend.PowerRemoved(__state.Epoch.Sequence, __state.Metadata.Identity) == 1) __state.Metadata.Dirty = false;
                    TemporalPowerCapture.RemovePower(__state.Metadata.Identity);
                }
                else if (after.Attached) __state.Metadata.Detached = null;
                return;
            }
            bool attachment = __originalMethod.Name == "ApplyPowerInternal";
            if (!after.Attached || (attachment ? before.Attached : !before.Attached)) return;
            __state.Metadata.Dirty = true;
            try
            {
                if (__state.PriorDirty && CaptureRuntime.Backend.PowerInvalidate(__state.Epoch.Sequence, __state.Metadata.Identity) != 1)
                    throw new InvalidOperationException("Prior dirty provenance could not be invalidated");
                var owner = IdentityCapture.Get(after.Owner, __state.Epoch);
                if (owner == null) throw new InvalidOperationException("Unavailable power owner identity");
                int status = CaptureRuntime.Upload(__state.Epoch, __state.Source, transfer => attachment
                    ? CaptureRuntime.Backend.PowerAttached(__state.Epoch, __state.Metadata.Identity, owner.Identity, after, transfer)
                    : CaptureRuntime.Backend.PowerChanged(__state.Epoch, __state.Metadata.Identity, owner.Identity, after, before.Amount, transfer));
                if (status != 1) throw new InvalidOperationException("Observed power mutation rejected");
                __state.Metadata.Dirty = false;
                __state.Metadata.Detached = null;
            }
            catch (Exception ex)
            {
                CaptureRuntime.Fail("power-observe-update", ex);
                try { CaptureRuntime.Backend.PowerInvalidate(__state.Epoch.Sequence, __state.Metadata.Identity); }
                catch (Exception invalidate) { CaptureRuntime.Fail("power-invalidate", invalidate); }
            }
        }
        catch (Exception ex)
        {
            if (__state.Metadata != null)
            {
                __state.Metadata.Dirty = true;
                try { if (CaptureRuntime.Valid(__state.Epoch)) CaptureRuntime.Backend.PowerInvalidate(__state.Epoch.Sequence, __state.Metadata.Identity); }
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
