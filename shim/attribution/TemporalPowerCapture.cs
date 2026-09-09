using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using HarmonyLib;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;

namespace SpireProfiler;

internal readonly record struct TemporalEntry(SourceSnapshot Source, int Amount, ulong Execution);
internal static class TemporalPowerCapture
{
    // Bounds all saved-card, saved-turn and Rupture accumulator entries together.
    internal const int MaxPending = 128;
    private static readonly Dictionary<(ulong Power, ulong Card), TemporalEntry> pending = new();
    private static readonly Type[] savedPowers = { typeof(StormPower), typeof(StranglePower), typeof(SerpentFormPower), typeof(GravityPower), typeof(AfterimagePower), typeof(OblivionPower), typeof(RupturePower) };
    internal static void Clear() => pending.Clear();
    internal static void RemovePower(ulong power)
    {
        foreach (var key in pending.Keys.Where(k => k.Power == power).ToArray()) pending.Remove(key);
    }
    internal static void AbandonExecution(ulong execution)
    {
        foreach (var key in pending.Where(p => p.Value.Execution == execution).Select(p => p.Key).ToArray()) pending.Remove(key);
    }
    internal static void AbandonCard(object card, CaptureEpoch epoch)
    {
        var identity = IdentityCapture.Get(card, epoch);
        if (identity != null) foreach (var key in pending.Keys.Where(k => k.Card == identity.Identity).ToArray()) pending.Remove(key);
    }
    internal static void Save(object power, object card, int amount, SourceSnapshot source, CaptureEpoch epoch)
    {
        if (!CaptureRuntime.Valid(epoch)) return;
        var powerId = IdentityCapture.Get(power, epoch);
        var cardId = card == null ? null : IdentityCapture.Get(card, epoch);
        if (powerId == null || (card != null && cardId == null)) return;
        var key = (powerId.Identity, cardId?.Identity ?? 0);
        if (!pending.ContainsKey(key) && pending.Count == MaxPending) { CaptureRuntime.Fail("temporal-cap"); return; }
        pending[key] = new(source, amount, PlayCapture.Execution.Identity);
    }
    internal static SourceSnapshot Take(object power, object card, CaptureEpoch epoch)
    {
        if (!CaptureRuntime.Valid(epoch)) return SourceSnapshot.Unavailable;
        var powerId = IdentityCapture.Get(power, epoch);
        var cardId = card == null ? null : IdentityCapture.Get(card, epoch);
        if (powerId == null || (card != null && cardId == null)) return SourceSnapshot.Unavailable;
        return pending.Remove((powerId.Identity, cardId?.Identity ?? 0), out var entry) ? entry.Source : SourceSnapshot.Unavailable;
    }
    internal static SourceSnapshot TurnSource(object power, CaptureEpoch epoch)
    {
        if (!CaptureRuntime.Valid(epoch)) return SourceSnapshot.Unavailable;
        var identity = IdentityCapture.Get(power, epoch);
        return identity != null && pending.TryGetValue((identity.Identity, 0), out var entry) ? entry.Source : SourceSnapshot.Unavailable;
    }
    internal static void Add(Dictionary<CardModel, int> dictionary, CardModel card, int amount)
    {
        dictionary.Add(card, amount);
        try
        {
            var frame = FlowCapture.Current;
            Save(frame.Model, card, amount, frame.Source, frame.Epoch);
        }
        catch (Exception ex) { CaptureRuntime.Fail("temporal-add", ex); }
    }
    internal static bool Remove(Dictionary<CardModel, int> dictionary, CardModel card, out int amount)
    {
        bool removed = dictionary.Remove(card, out amount);
        if (!removed) return false;
        var previous = FlowCapture.Current;
        FlowCapture.Current = previous with { Source = SourceSnapshot.Unavailable };
        try { FlowCapture.Current = previous with { Source = Take(previous.Model, card, previous.Epoch) }; }
        catch (Exception ex) { CaptureRuntime.Fail("temporal-remove", ex); }
        return true;
    }
    internal static void Accumulate(Dictionary<CardModel, int> dictionary, CardModel card, int amount)
    {
        int before = dictionary[card];
        dictionary[card] = amount;
        try
        {
            var frame = FlowCapture.Current;
            var prior = Take(frame.Model, card, frame.Epoch);
            var combined = Combine(frame.Epoch, prior, Math.Max(before, 0), frame.Source, checked(amount - before));
            Save(frame.Model, card, amount, combined, frame.Epoch);
        }
        catch (Exception ex) { CaptureRuntime.Fail("temporal-accumulate", ex); }
    }
    internal static SourceSnapshot Combine(CaptureEpoch epoch, SourceSnapshot first, int firstAmount, SourceSnapshot second, int secondAmount)
    {
        if (firstAmount < 0 || secondAmount < 0) return SourceSnapshot.Unavailable;
        if (firstAmount == 0) return second;
        if (secondAmount == 0) return first;
        if (first.Epoch == 0) first = SourceSnapshot.Unknown(epoch.Sequence);
        if (second.Epoch == 0) second = SourceSnapshot.Unknown(epoch.Sequence);
        if (first.Epoch != epoch.Sequence || second.Epoch != epoch.Sequence) return SourceSnapshot.Unavailable;
        try
        {
            ulong firstSum = 0, secondSum = 0;
            for (int i = 0; i < first.Count; i++) firstSum = checked(firstSum + first[i].Weight);
            for (int i = 0; i < second.Count; i++) secondSum = checked(secondSum + second[i].Weight);
            var destinations = new List<ulong>();
            var weights = new List<UInt128>();
            ulong divisor = SourceSnapshot.Gcd(firstSum, secondSum);
            foreach (var piece in new[] { (Source: first, Amount: firstAmount, Scale: secondSum / divisor), (Source: second, Amount: secondAmount, Scale: firstSum / divisor) })
                for (int i = 0; i < piece.Source.Count; i++)
                {
                    var share = piece.Source[i];
                    UInt128 weight = checked((UInt128)share.Weight * (uint)piece.Amount * piece.Scale);
                    int index = destinations.IndexOf(share.Destination);
                    if (index < 0) { destinations.Add(share.Destination); weights.Add(weight); }
                    else weights[index] = checked(weights[index] + weight);
                }
            UInt128 gcd = 0;
            foreach (var weight in weights)
            {
                UInt128 a = gcd, b = weight;
                while (b != 0) (a, b) = (b, a % b);
                gcd = a;
            }
            var shares = new SourceShare[weights.Count];
            for (int i = 0; i < shares.Length; i++) shares[i] = new(destinations[i], checked((ulong)(weights[i] / gcd)));
            return SourceSnapshot.Create(epoch.Sequence, shares);
        }
        catch (Exception ex) { CaptureRuntime.Fail("temporal-mixture", ex); return SourceSnapshot.Unavailable; }
    }
    internal static void SetTurnAmount(PowerModel power, int amount)
    {
        power.AmountOnTurnStart = amount;
        if (power is not HelloWorldPower) return;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch) || !ReferenceEquals(power.Owner?.CombatState, epoch.Combat)) return;
            Save(power, null, amount, FlowCapture.Source(power, epoch), epoch);
        }
        catch (Exception ex) { CaptureRuntime.Fail("temporal-turn", ex); }
    }
    internal static MethodInfo Body(MethodInfo method)
    {
        method = FlowCapture.DeclaredMethod(method);
        var state = method.GetCustomAttribute<AsyncStateMachineAttribute>()?.StateMachineType;
        return state == null ? method : AccessTools.DeclaredMethod(state, "MoveNext", Type.EmptyTypes);
    }
    internal static IEnumerable<CodeInstruction> Transpiler(IEnumerable<CodeInstruction> instructions, MethodBase __originalMethod)
    {
        var code = instructions.Select(c => new CodeInstruction(c)).ToList();
        const BindingFlags instance = BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly;
        var add = typeof(Dictionary<CardModel, int>).GetMethod("Add", instance, null, new[] { typeof(CardModel), typeof(int) }, null);
        var remove = typeof(Dictionary<CardModel, int>).GetMethod("Remove", instance, null, new[] { typeof(CardModel), typeof(int).MakeByRefType() }, null);
        var set = typeof(Dictionary<CardModel, int>).GetMethod("set_Item", instance, null, new[] { typeof(CardModel), typeof(int) }, null);
        var turn = typeof(PowerModel).GetMethod("set_AmountOnTurnStart", instance, null, new[] { typeof(int) }, null);
        if (add?.ReturnType != typeof(void) || remove?.ReturnType != typeof(bool) || set?.ReturnType != typeof(void) || turn?.ReturnType != typeof(void))
            throw new InvalidOperationException("Temporal bridge signature changed");
        int additions = code.Count(c => c.Calls(add)), removals = code.Count(c => c.Calls(remove)), sets = code.Count(c => c.Calls(set)), turns = code.Count(c => c.Calls(turn));
        string name = __originalMethod.DeclaringType.FullName + "." + __originalMethod.Name;
        bool before = name.Contains("BeforeCardPlayed"), after = name.Contains("AfterCardPlayed"), rupture = name.Contains("RupturePower") && name.Contains("AfterDamageReceived"), turnStart = __originalMethod.Name == "BeforeTurnStart";
        if ((additions, removals, sets, turns) != (before ? 1 : 0, after ? 1 : 0, rupture ? 1 : 0, turnStart ? 1 : 0))
            throw new InvalidOperationException($"Temporal IL drift: {name}: Add={additions} Remove={removals} Set={sets} Turn={turns}");
        foreach (var instruction in code)
        {
            string bridge = instruction.Calls(add) ? nameof(Add) : instruction.Calls(remove) ? nameof(Remove)
                : instruction.Calls(set) ? nameof(Accumulate) : instruction.Calls(turn) ? nameof(SetTurnAmount) : null;
            if (bridge != null) { instruction.opcode = OpCodes.Call; instruction.operand = AccessTools.Method(typeof(TemporalPowerCapture), bridge); }
        }
        return code;
    }
    internal static void Install(Harmony harmony, Action<string> report)
    {
        foreach (var type in savedPowers)
            foreach (var name in new[] { "BeforeCardPlayed", "AfterCardPlayed" })
            {
                var target = Body(AccessTools.DeclaredMethod(type, name));
                CapturePatches.Patch(harmony, target, transpiler: new HarmonyMethod(typeof(TemporalPowerCapture), nameof(Transpiler)));
                report($"TEMPORAL BRIDGE {type.Name}.{name}: {(name == "BeforeCardPlayed" ? "Add=1" : "Remove=1")}");
            }
        CapturePatches.Patch(harmony, Body(AccessTools.DeclaredMethod(typeof(RupturePower), "AfterDamageReceived")), transpiler: new HarmonyMethod(typeof(TemporalPowerCapture), nameof(Transpiler)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Creature), "BeforeTurnStart"), transpiler: new HarmonyMethod(typeof(TemporalPowerCapture), nameof(Transpiler)));
        report("TEMPORAL BRIDGE RupturePower.AfterDamageReceived: Set=1");
        report("TEMPORAL BRIDGE Creature.BeforeTurnStart: AmountOnTurnStart=1");
    }
}
