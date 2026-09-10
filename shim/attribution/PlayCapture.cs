using System;
using System.Linq;
using System.Reflection;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Combat.History;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Models;

namespace SpireProfiler;

internal sealed record ExecutionFrame(CaptureEpoch Epoch, ulong Identity, object Model, bool Potion)
{
    internal static readonly ExecutionFrame Barrier = new(default, 0, null, false);
}
internal sealed record PlayFrame(CaptureEpoch Epoch, ulong Token, ulong Execution, object Card, int Slot, SourceSnapshot Source, PlayFrame Previous);
internal readonly record struct WrapperState(ExecutionFrame Execution, PlayFrame Play, ProducerFrame Producer);
internal static class PlayCapture
{
    // Mirrors the native bound on nested/replayed frames within one physical player.
    private const int MaxActivePlays = 32;
    private static readonly AsyncLocal<ExecutionFrame> execution = new();
    private static readonly AsyncLocal<PlayFrame> play = new();
    internal static ExecutionFrame Execution { get => execution.Value ?? ExecutionFrame.Barrier; private set => execution.Value = value; }
    internal static PlayFrame Current { get => play.Value; private set => play.Value = value; }
    internal static void WrapperPrefix(object __instance, MethodBase __originalMethod, out WrapperState __state)
    {
        __state = new(Execution, Current, FlowCapture.Current);
        Execution = ExecutionFrame.Barrier;
        Current = null;
        FlowCapture.Current = ProducerFrame.Barrier;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Producer.Epoch);
            if (CaptureRuntime.Stale(__state.Execution.Epoch)) epoch = __state.Execution.Epoch;
            Execution = ExecutionFrame.Barrier with { Epoch = epoch };
            if (!CaptureRuntime.Valid(epoch)) return;
            Execution = new(epoch, IdentityCapture.Execution(epoch), __instance, __originalMethod.Name == "OnUseWrapper");
        }
        catch (Exception ex) { CaptureRuntime.Fail("wrapper-prefix", ex); }
    }
    internal static void WrapperFinalizer(WrapperState __state)
    {
        if (__state.Execution == null) return;
        Execution = __state.Execution;
        Current = __state.Play;
        FlowCapture.Current = __state.Producer;
    }
    internal static void Started(object card, int slot, int index, int count)
    {
        var prior = Current;
        int depth = 0;
        for (var frame = prior; frame != null; frame = frame.Previous) depth++;
        if (depth >= MaxActivePlays) { CaptureRuntime.Fail("managed-play-cap"); prior = null; }
        Current = new(default, 0, 0, card, slot, SourceSnapshot.Unavailable, prior);
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch)) return;
            var metadata = IdentityCapture.Get(card, epoch);
            string id = "";
            try { id = CaptureRuntime.Backend.Describe(card).Id; }
            catch (Exception ex) { CaptureRuntime.Fail("play-card-descriptor", ex); }
            var source = FlowCapture.Source(card, epoch);
            var generation = metadata?.Generation ?? GenerationState.Unclassified;
            ulong executionId = CaptureRuntime.Valid(Execution.Epoch) && !Execution.Potion ? Execution.Identity : 0;
            ulong token = CaptureRuntime.Upload(epoch, source, transfer => CaptureRuntime.Backend.PlayStarted(epoch.Sequence, executionId,
                metadata?.Identity ?? 0, id, slot, index, count, generation, transfer));
            Current = new(epoch, token, executionId, card, slot, source, prior);
        }
        catch (Exception ex) { CaptureRuntime.Fail("play-start", ex); }
    }
    internal static void Finished(object card)
    {
        try
        {
            var frame = Current;
            if (frame == null || !ReferenceEquals(frame.Card, card)) { CaptureRuntime.Fail("play-finish-mismatch"); return; }
            Current = frame.Previous;
            if (frame.Token != 0 && CaptureRuntime.Valid(frame.Epoch)) CaptureRuntime.Backend.PlayFinished(frame.Token);
        }
        catch (Exception ex) { CaptureRuntime.Fail("play-finish", ex); }
    }
    internal static void EndEffectPrefix()
    {
        try
        {
            var frame = Execution;
            if (CaptureRuntime.Valid(frame.Epoch) && frame.Identity != 0)
            {
                CaptureRuntime.Backend.ExecutionEnded(frame.Epoch.Sequence, frame.Identity);
                TemporalPowerCapture.AbandonExecution(frame.Identity);
            }
            Current = null;
        }
        catch (Exception ex) { CaptureRuntime.Fail("execution-end", ex); }
    }
    internal static void StartedPostfix(object combatState, CardPlay cardPlay)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (CaptureRuntime.Valid(epoch) && ReferenceEquals(epoch.Combat, combatState))
                Started(cardPlay.Card, RunContext.PlayerSlot(cardPlay.Player), cardPlay.PlayIndex, cardPlay.PlayCount);
        }
        catch (Exception ex) { CaptureRuntime.Fail("play-history-start", ex); }
    }
    internal static void FinishedPostfix(object combatState, CardPlay cardPlay)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (CaptureRuntime.Valid(epoch) && ReferenceEquals(epoch.Combat, combatState)) Finished(cardPlay.Card);
        }
        catch (Exception ex) { CaptureRuntime.Fail("play-history-finish", ex); }
    }
    internal static void OrbPrefix(object __instance, out ProducerFrame __state)
    {
        __state = FlowCapture.Current;
        FlowCapture.Current = ProducerFrame.Barrier;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Epoch);
            FlowCapture.Current = ProducerFrame.Barrier with { Epoch = epoch };
            if (!CaptureRuntime.Valid(epoch)) return;
            var descriptor = CaptureRuntime.Backend.Describe(__instance);
            var identity = IdentityCapture.Get(__instance, epoch);
            if (identity == null) return;
            var current = Current;
            bool unavailablePlay = current != null && (!CaptureRuntime.Valid(current.Epoch) || current.Token == 0)
                || Execution.Model != null && Execution.Identity == 0;
            if (unavailablePlay) return;
            ulong token = current != null && current.Slot == descriptor.Slot ? current.Token : 0;
            int policy = CaptureRuntime.Backend.OrbBegin(epoch.Sequence, identity.Identity, token, descriptor.Slot);
            SourceSnapshot source = policy == 1 ? FlowCapture.Source(__instance, epoch)
                : policy == 2 && current != null ? current.Source : SourceSnapshot.Unavailable;
            FlowCapture.Current = new(epoch, __instance, source, ProducerRole.Orb, policy == 2 ? DamageSegment.Direct : DamageSegment.Attributed);
        }
        catch (Exception ex) { CaptureRuntime.Fail("orb-entry", ex); }
    }
    internal static void OrbFinalizer(ProducerFrame __state) { if (__state != null) FlowCapture.Current = __state; }
    internal static void Install(Harmony harmony)
    {
        foreach (var target in new[] { AccessTools.DeclaredMethod(typeof(CardModel), "OnPlayWrapper"), AccessTools.DeclaredMethod(typeof(PotionModel), "OnUseWrapper") })
            PatchWrapper(harmony, target);
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatHistory), "CardPlayStarted"), postfix: new HarmonyMethod(typeof(PlayCapture), nameof(StartedPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatHistory), "CardPlayFinished"), postfix: new HarmonyMethod(typeof(PlayCapture), nameof(FinishedPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatManager), "EndCardOrPotionEffect"), prefix: new HarmonyMethod(typeof(PlayCapture), nameof(EndEffectPrefix)));
        foreach (var type in typeof(OrbModel).Assembly.GetTypes().Where(t => !t.IsAbstract && typeof(OrbModel).IsAssignableFrom(t)))
            foreach (var target in type.GetMethods(BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly).Where(m => m.Name is "Passive" or "Evoke"))
                CapturePatches.Patch(harmony, FlowCapture.DeclaredMethod(target), prefix: new HarmonyMethod(typeof(PlayCapture), nameof(OrbPrefix)), finalizer: new HarmonyMethod(typeof(PlayCapture), nameof(OrbFinalizer)));
    }
    internal static void PatchWrapper(Harmony harmony, MethodInfo target)
        => CapturePatches.Patch(harmony, FlowCapture.DeclaredMethod(target), prefix: new HarmonyMethod(typeof(PlayCapture), nameof(WrapperPrefix)), finalizer: new HarmonyMethod(typeof(PlayCapture), nameof(WrapperFinalizer)));
}
