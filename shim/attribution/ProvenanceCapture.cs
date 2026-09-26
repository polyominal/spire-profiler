using System;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Collections.Generic;
using HarmonyLib;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Combat.History;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;

namespace SpireProfiler;

internal sealed record PendingPower(CaptureEpoch Epoch, object Power, object Target, SourceSnapshot Source, AuditCommand Audit = null)
{
    internal static readonly PendingPower Barrier = new(default, null, null, SourceSnapshot.Unavailable);
}
internal enum MutationOperation { Attachment, AmountChange, Removal }
internal abstract record MutationState
{
    private MutationState() { }
    internal AuditMutation Audit { get; init; }
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
        var audit = PoisonAudit.Attempt(__originalMethod, __args);
        Pending = PendingPower.Barrier with { Audit = audit };
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Epoch);
            Pending = PendingPower.Barrier with { Epoch = epoch, Audit = audit };
            if (!CaptureRuntime.Valid(epoch)) return;
            bool apply = __originalMethod.Name == "Apply";
            object power = __args[1], target = apply ? __args[2] : null, card = __args[apply ? 5 : 4];
            Pending = new(epoch, power, target, FlowCapture.Supplied(card, epoch), audit);
        }
        catch (Exception ex) { CaptureRuntime.Fail("power-command", ex); }
    }
    internal static void CommandFinalizer(PendingPower __state) { if (__state != null) Pending = __state; }
    internal static void ApplyCompleted(ref AsyncTaskMethodBuilder builder)
    {
        PoisonAudit.Complete(Pending.Audit, "completed");
        builder.SetResult();
    }
    internal static void ApplyFailed(ref AsyncTaskMethodBuilder builder, Exception error)
    {
        PoisonAudit.Complete(Pending.Audit, error is OperationCanceledException ? "cancelled" : "faulted", error: error);
        builder.SetException(error);
    }
    internal static void ChangeCompleted(ref AsyncTaskMethodBuilder<int> builder, int result)
    {
        PoisonAudit.Complete(Pending.Audit, "completed", result);
        builder.SetResult(result);
    }
    internal static void ChangeFailed(ref AsyncTaskMethodBuilder<int> builder, Exception error)
    {
        PoisonAudit.Complete(Pending.Audit, error is OperationCanceledException ? "cancelled" : "faulted", error: error);
        builder.SetException(error);
    }
    internal static void EnvenomCompleted(ref AsyncTaskMethodBuilder builder)
    {
        PoisonAudit.EnvenomEnd("completed");
        builder.SetResult();
    }
    internal static void EnvenomFailed(ref AsyncTaskMethodBuilder builder, Exception error)
    {
        PoisonAudit.EnvenomEnd(error is OperationCanceledException ? "cancelled" : "faulted", error);
        builder.SetException(error);
    }
    internal static IEnumerable<CodeInstruction> CompletionTranspiler(IEnumerable<CodeInstruction> instructions, MethodBase __originalMethod)
    {
        var code = instructions.Select(instruction => new CodeInstruction(instruction)).ToList();
        string name = __originalMethod.DeclaringType.FullName;
        bool envenom = name.Contains("EnvenomPower"), change = name.Contains("ModifyAmount");
        var builder = change ? typeof(AsyncTaskMethodBuilder<int>) : typeof(AsyncTaskMethodBuilder);
        var complete = AccessTools.DeclaredMethod(builder, "SetResult", change ? new[] { typeof(int) } : Type.EmptyTypes);
        var fail = AccessTools.DeclaredMethod(builder, "SetException", new[] { typeof(Exception) });
        if (code.Count(instruction => instruction.Calls(complete)) != 1 || code.Count(instruction => instruction.Calls(fail)) != 1)
            throw new InvalidOperationException("Audited power command completion IL changed: " + name);
        foreach (var instruction in code)
        {
            string bridge = instruction.Calls(complete)
                ? envenom ? nameof(EnvenomCompleted) : change ? nameof(ChangeCompleted) : nameof(ApplyCompleted)
                : instruction.Calls(fail) ? envenom ? nameof(EnvenomFailed) : change ? nameof(ChangeFailed) : nameof(ApplyFailed) : null;
            if (bridge == null) continue;
            instruction.opcode = OpCodes.Call;
            instruction.operand = AccessTools.DeclaredMethod(typeof(ProvenanceCapture), bridge);
        }
        return code;
    }
    internal static void MutationPrefix(object __instance, MethodBase __originalMethod, object[] __args, out MutationState __state)
    {
        var audit = PoisonAudit.BeforeMutation(__instance, __originalMethod, __args);
        __state = audit == null ? MutationState.Inactive : MutationState.Inactive with { Audit = audit };
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
            __state = new MutationState.ObservationFailed(epoch, metadata) { Audit = audit };
            var before = CaptureRuntime.Backend.ObservePower(power, amount ? null : __instance);
            __state = new MutationState.Observed(epoch, metadata, operation, before, SourceSnapshot.Unavailable, metadata.Dirty) { Audit = audit };
            var source = operation == MutationOperation.Removal ? FlowCapture.Source(power, epoch)
                : Matching(power) ? Pending.Source : FlowCapture.Supplied(null, epoch);
            __state = new MutationState.Observed(epoch, metadata, operation, before, source, metadata.Dirty) { Audit = audit };
            if (operation == MutationOperation.Removal) metadata.Detached = source;
        }
        catch (Exception ex) { CaptureRuntime.Fail("power-observe-prefix", ex); }
    }
    internal static void MutationFinalizer(MutationState __state)
    {
        var tracked = __state as MutationState.Tracked;
        int? status = null;
        try
        {
            if (tracked == null || !CaptureRuntime.Valid(tracked.Epoch)) return;
            if (tracked is MutationState.ObservationFailed)
            {
                tracked.Metadata.Dirty = true;
                status = CaptureRuntime.Backend.PowerInvalidate(tracked.Epoch.Sequence, tracked.Metadata.Identity);
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
                    status = CaptureRuntime.Backend.PowerRemoved(tracked.Epoch.Sequence, tracked.Metadata.Identity);
                    if (status == 1) tracked.Metadata.Dirty = false;
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
                status = CaptureRuntime.WithSource(tracked.Epoch, observed.Source, transfer => attachment
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
        finally { PoisonAudit.AfterMutation(__state?.Audit, status); }
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
            int status = CaptureRuntime.WithSource(epoch, source, transfer => CaptureRuntime.Backend.CardGenerated(epoch.Sequence, metadata?.Identity ?? 0, transfer, FlowCapture.Current.Role));
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
        foreach (var target in new[] { apply, change })
        {
            CapturePatches.Patch(harmony, FlowCapture.DeclaredMethod(target), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(CommandPrefix)), finalizer: new HarmonyMethod(typeof(ProvenanceCapture), nameof(CommandFinalizer)));
            CapturePatches.Patch(harmony, TemporalPowerCapture.Body(target), transpiler: new HarmonyMethod(typeof(ProvenanceCapture), nameof(CompletionTranspiler)));
        }
        CapturePatches.Patch(harmony, TemporalPowerCapture.Body(AccessTools.DeclaredMethod(typeof(EnvenomPower), "AfterDamageGiven")), transpiler: new HarmonyMethod(typeof(ProvenanceCapture), nameof(CompletionTranspiler)));
        foreach (var target in new[] { AccessTools.DeclaredMethod(typeof(Creature), "ApplyPowerInternal"), AccessTools.DeclaredMethod(typeof(Creature), "RemovePowerInternal"), AccessTools.DeclaredMethod(typeof(PowerModel), "SetAmount") })
            PatchMutation(harmony, target);
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatHistory), "CardGenerated"), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(GeneratedPrefix)));
    }
    internal static void PatchMutation(Harmony harmony, MethodInfo target)
        => CapturePatches.Patch(harmony, FlowCapture.DeclaredMethod(target), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(MutationPrefix)), finalizer: new HarmonyMethod(typeof(ProvenanceCapture), nameof(MutationFinalizer)));
}
