using System;
using System.Collections.Generic;
using HarmonyLib;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Models.Powers;

namespace SpireProfiler;

internal readonly record struct DoomObservation(bool Alive, int Hp, object Power, object Combat);
internal sealed class DoomState
{
    internal readonly CaptureEpoch Epoch;
    internal readonly List<int> HitPoints = new(DoomCapture.MaxTargets);
    internal ulong Token;
    internal long Remainder;
    internal bool Complete = true;
    internal bool Reported;
    internal DoomState(CaptureEpoch epoch) { Epoch = epoch; }
}
internal static class DoomCapture
{
    // Mirrors the native bound on one synchronous Doom kickoff's frozen targets.
    internal const int MaxTargets = 16;
    internal static Func<object, DoomObservation> Inspect = value =>
    {
        var creature = (Creature)value;
        return new(creature.IsAlive, creature.CurrentHp, creature.GetPower<DoomPower>(), creature.CombatState);
    };
    internal static void Prefix(IReadOnlyList<Creature> creatures, out DoomState __state)
    {
        __state = null;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            __state = new(epoch);
            if (!CaptureRuntime.Valid(epoch) || creatures == null) return;
            try { __state.Token = CaptureRuntime.Backend.DoomBegin(epoch.Sequence); }
            catch (Exception ex) { CaptureRuntime.Fail("doom-begin", ex); }
            if (__state.Token == 0) __state.Complete = false;
            foreach (var creature in creatures)
            {
                try
                {
                    if (creature == null) { __state.Complete = false; CaptureRuntime.Fail("doom-null-target"); continue; }
                    var target = Inspect(creature);
                    if (!ReferenceEquals(target.Combat, epoch.Combat)) { __state.Complete = false; CaptureRuntime.Fail("doom-stale-target"); continue; }
                    if (!target.Alive) continue;
                    if (target.Hp <= 0) { __state.Complete = false; CaptureRuntime.Fail("doom-invalid-hp"); continue; }
                    if (__state.HitPoints.Count < MaxTargets) __state.HitPoints.Add(target.Hp);
                    else
                    {
                        __state.Remainder = checked(__state.Remainder + target.Hp);
                        __state.Complete = false;
                        CaptureRuntime.Fail("doom-target-cap");
                    }
                    if (!__state.Complete) continue;
                    var identity = IdentityCapture.Get(creature, epoch);
                    var power = target.Power == null ? null : IdentityCapture.Get(target.Power, epoch);
                    if (identity == null || (target.Power != null && (power == null || power.Dirty))
                        || CaptureRuntime.Backend.DoomTarget(__state.Token, identity.Identity, power?.Identity ?? 0, target.Hp) != 1)
                        __state.Complete = false;
                }
                catch (Exception ex) { __state.Complete = false; CaptureRuntime.Fail("doom-target", ex); }
            }
            if (!__state.Complete) Abort(__state);
        }
        catch (Exception ex)
        {
            if (__state != null) { __state.Complete = false; Abort(__state); }
            CaptureRuntime.Fail("doom-entry", ex);
        }
    }
    internal static void Postfix(DoomState __state, bool __runOriginal)
    {
        try
        {
            if (__state == null || !__runOriginal || __state.Reported || !CaptureRuntime.Valid(__state.Epoch)) return;
            __state.Reported = true;
            bool committed = false;
            if (__state.Complete && __state.Token != 0)
            {
                try { committed = CaptureRuntime.Backend.DoomComplete(__state.Token) == 1; }
                catch (Exception ex) { CaptureRuntime.Fail("doom-commit", ex); }
            }
            if (committed) { __state.Token = 0; return; }
            Abort(__state);
            foreach (int hp in __state.HitPoints)
                if (CaptureRuntime.Backend.DamageFallback(__state.Epoch.Sequence, new(hp, hp, 0, ResultKind.Outgoing, 4)) != 1)
                    CaptureRuntime.Fail("doom-fallback");
            long remaining = __state.Remainder;
            while (remaining > 0)
            {
                int amount = (int)Math.Min(remaining, int.MaxValue);
                if (CaptureRuntime.Backend.DamageFallback(__state.Epoch.Sequence, new(amount, amount, 0, ResultKind.Outgoing, 4)) != 1)
                    CaptureRuntime.Fail("doom-fallback");
                remaining -= amount;
            }
        }
        catch (Exception ex) { CaptureRuntime.Fail("doom-report", ex); }
    }
    internal static void Finalizer(DoomState __state) { if (__state != null) Abort(__state); }
    private static void Abort(DoomState state)
    {
        ulong token = state.Token;
        state.Token = 0;
        if (token == 0) return;
        try
        {
            if (CaptureRuntime.Valid(state.Epoch) && CaptureRuntime.Backend.DoomAbort(token) != 1) CaptureRuntime.Fail("doom-abort");
        }
        catch (Exception ex) { CaptureRuntime.Fail("doom-abort", ex); }
    }
    internal static void Install(Harmony harmony, Action<string> report)
    {
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(DoomPower), "DoomKill", new[] { typeof(IReadOnlyList<Creature>) }),
            prefix: new HarmonyMethod(typeof(DoomCapture), nameof(Prefix)), postfix: new HarmonyMethod(typeof(DoomCapture), nameof(Postfix)), finalizer: new HarmonyMethod(typeof(DoomCapture), nameof(Finalizer)));
        report("DOOM CAPTURE batch_prefix=1 post_kickoff_commit=1 finalizer=1");
    }
}
