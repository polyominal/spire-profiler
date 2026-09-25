using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Hooks;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Models.Relics;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

internal sealed record ObservedModifier(SourceSnapshot Source, decimal Input, decimal Output, bool Multiplicative,
    bool Vulnerable, List<ObservedModifier> Nested = null);
internal sealed class ModifierFrame
{
    internal readonly CaptureEpoch Epoch;
    internal readonly decimal Initial;
    internal readonly List<ObservedModifier> Observations = new();
    internal bool Failed, Closed, Finished;
    internal ModifierFrame(CaptureEpoch epoch, decimal initial) { Epoch = epoch; Initial = initial; }
    internal void Record(ObservedModifier observation)
    {
        if (Closed) return;
        if (Observations.Count == 64) { Failed = true; CaptureRuntime.Fail("modifier-observation-cap"); return; }
        Observations.Add(observation);
    }
    internal SourceCredit[] Contributions(decimal result, bool damage)
    {
        if (Failed || !CaptureRuntime.Valid(Epoch)) throw new InvalidOperationException("Modifier observation is incomplete");
        if (result <= Initial) return Array.Empty<SourceCredit>();
        var credits = new List<SourceCredit>();
        foreach (var observation in Observations)
        {
            if (!observation.Multiplicative)
            {
                if (damage || observation.Output > 0)
                {
                    int amount = ModifierCapture.Additive(observation.Output, damage);
                    if (amount > 0) credits.Add(new(observation.Source, amount));
                }
                continue;
            }
            if (observation.Output <= 1) continue;
            var parts = observation.Nested;
            decimal multiplier = parts is { Count: > 0 } ? parts[0].Input : observation.Output;
            bool coherent = observation.Vulnerable && multiplier >= 1;
            foreach (var part in parts ?? Enumerable.Empty<ObservedModifier>())
            {
                coherent &= part.Input == multiplier;
                multiplier = part.Output;
            }
            if (coherent && multiplier == observation.Output)
            {
                decimal baseline = parts is { Count: > 0 } ? parts[0].Input : observation.Output;
                int amount = ModifierCapture.Product(observation.Input, baseline - 1, result);
                if (amount > 0) credits.Add(new(observation.Source, amount));
                foreach (var part in parts ?? Enumerable.Empty<ObservedModifier>())
                {
                    amount = ModifierCapture.Product(observation.Input, part.Output - part.Input, result);
                    if (amount > 0) credits.Add(new(part.Source, amount));
                }
                continue;
            }
            int increase = ModifierCapture.Increase(observation.Input, observation.Output, result);
            if (increase > 0) credits.Add(new(observation.Source, increase));
        }
        return credits.ToArray();
    }

}
internal readonly record struct ModifierScope(ModifierFrame Previous, ModifierFrame Frame, bool Entered = true, ModifierFrame PreviousAdmission = null);
internal sealed record NestedModifierFrame(ModifierFrame Owner, List<ObservedModifier> Observations);

internal static class ModifierCapture
{
    // A canonical calculation admits one Hook invocation; recursive previews get
    // an empty scope. Call-site bridges preserve the game's original dispatch.
    private static readonly AsyncLocal<ModifierFrame> current = new();
    private static readonly AsyncLocal<ModifierFrame> admitted = new();
    private static readonly AsyncLocal<NestedModifierFrame> nested = new();
    internal static ModifierFrame Current => current.Value is { Closed: false } frame ? frame : null;
    internal static ModifierScope BeginCalculation(CaptureEpoch epoch, decimal amount, bool enabled = true)
    {
        var frame = enabled ? new ModifierFrame(epoch, amount) : null;
        var scope = new ModifierScope(current.Value, frame, PreviousAdmission: admitted.Value);
        admitted.Value = frame;
        current.Value = frame;
        return scope;
    }
    internal static void EndCalculation(ModifierScope scope)
    {
        if (scope.Frame != null)
        {
            scope.Frame.Closed = scope.Frame.Finished = true;
            foreach (var observation in scope.Frame.Observations) observation.Nested?.Clear();
            scope.Frame.Observations.Clear();
        }
        admitted.Value = scope.PreviousAdmission is { Closed: false, Finished: false } pending ? pending : null;
        current.Value = scope.Previous;
    }
    internal static void HookPrefix(out ModifierScope __state)
    {
        // An earlier Harmony prefix can recurse before this prefix runs. Its
        // preview closes the armed frame; the outer invocation must reject it.
        if (admitted.Value == null && current.Value is { Closed: true, Finished: false } displaced)
        {
            displaced.Failed = true;
            CaptureRuntime.Fail("modifier-admission");
        }
        var frame = admitted.Value is { Finished: false } pending ? pending : null;
        __state = new(current.Value, frame);
        current.Value = frame;
        admitted.Value = null;
    }
    internal static void HookPostfix(bool __runOriginal, ModifierScope __state)
    {
        var frame = __state.Frame ?? admitted.Value ?? (current.Value is { Closed: true, Finished: false } displaced ? displaced : null);
        if (__runOriginal || frame == null) return;
        frame.Failed = true;
        CaptureRuntime.Fail("modifier-hook-skipped");
    }
    internal static void ScopeFinalizer(ModifierScope __state)
    {
        if (!__state.Entered) return;
        if (__state.Frame != null) __state.Frame.Closed = true;
        current.Value = __state.Previous;
    }
    private static SourceSnapshot Source(ModifierFrame frame, AbstractModel model)
    {
        if (frame == null) return SourceSnapshot.Unavailable;
        try { return FlowCapture.Source(model, frame.Epoch); }
        catch (Exception ex) { frame.Failed = true; CaptureRuntime.Fail("modifier-source", ex); return SourceSnapshot.Unavailable; }
    }
    private static void Record(ModifierFrame frame, SourceSnapshot source, decimal input, decimal output, bool multiplicative,
        bool vulnerable = false, List<ObservedModifier> parts = null)
    {
        if (frame == null || (multiplicative ? output == 1 : output == 0)) return;
        try { frame.Record(new(source, input, output, multiplicative, vulnerable, parts)); }
        catch (Exception ex) { frame.Failed = true; CaptureRuntime.Fail("modifier-observation", ex); }
    }
    internal static decimal DamageAdditive(AbstractModel model, Creature target, decimal amount, ValueProp props, Creature dealer, CardModel card, CardPlay play)
    {
        var frame = Current;
        var source = Source(frame, model);
        decimal result = model.ModifyDamageAdditive(target, amount, props, dealer, card, play);
        Record(frame, source, amount, result, false);
        return result;
    }
    internal static decimal DamageMultiplicative(AbstractModel model, Creature target, decimal amount, ValueProp props, Creature dealer, CardModel card, CardPlay play)
    {
        var frame = Current;
        var source = Source(frame, model);
        var previous = nested.Value;
        var parts = frame != null && model is VulnerablePower ? new List<ObservedModifier>() : null;
        nested.Value = parts == null ? null : new(frame, parts);
        decimal result;
        try { result = model.ModifyDamageMultiplicative(target, amount, props, dealer, card, play); }
        finally { nested.Value = previous; }
        Record(frame, source, amount, result, true, model is VulnerablePower, parts);
        return result;
    }
    internal static decimal BlockAdditive(AbstractModel model, Creature target, decimal amount, ValueProp props, CardModel card, CardPlay play)
    {
        var frame = Current;
        var source = Source(frame, model);
        decimal result = model.ModifyBlockAdditive(target, amount, props, card, play);
        Record(frame, source, amount, result, false);
        return result;
    }
    internal static decimal BlockMultiplicative(AbstractModel model, Creature target, decimal amount, ValueProp props, CardModel card, CardPlay play)
    {
        var frame = Current;
        var source = Source(frame, model);
        decimal result = model.ModifyBlockMultiplicative(target, amount, props, card, play);
        Record(frame, source, amount, result, true);
        return result;
    }
    private static void NestedResult(NestedModifierFrame frame, SourceSnapshot source, decimal input, decimal output)
    {
        if (frame == null || !ReferenceEquals(frame.Owner, Current)) return;
        try
        {
            if (frame.Observations.Count == 16) throw new InvalidOperationException("Nested modifier capacity exceeded");
            frame.Observations.Add(new(source, input, output, true, false));
        }
        catch (Exception ex) { frame.Owner.Failed = true; CaptureRuntime.Fail("nested-modifier-result", ex); }
    }
    internal static decimal Phrog(PaperPhrog model, Creature target, decimal amount, ValueProp props, Creature dealer, CardModel card)
    {
        var frame = nested.Value;
        var source = Source(ReferenceEquals(frame?.Owner, Current) ? Current : null, model);
        decimal result = model.ModifyVulnerableMultiplier(target, amount, props, dealer, card);
        NestedResult(frame, source, amount, result);
        return result;
    }
    internal static decimal Cruelty(CrueltyPower model, Creature target, decimal amount, ValueProp props, Creature dealer, CardModel card)
    {
        var frame = nested.Value;
        var source = Source(ReferenceEquals(frame?.Owner, Current) ? Current : null, model);
        decimal result = model.ModifyVulnerableMultiplier(target, amount, props, dealer, card);
        NestedResult(frame, source, amount, result);
        return result;
    }
    internal static decimal Debilitate(DebilitatePower model, Creature target, decimal amount, ValueProp props, Creature dealer, CardModel card)
    {
        var frame = nested.Value;
        var source = Source(ReferenceEquals(frame?.Owner, Current) ? Current : null, model);
        decimal result = model.ModifyVulnerableMultiplier(target, amount, props, dealer, card);
        NestedResult(frame, source, amount, result);
        return result;
    }
    internal static decimal ModifyBlock(ICombatState combatState, Creature target, decimal block, ValueProp props, CardModel card, CardPlay play,
        out IEnumerable<AbstractModel> modifiers)
    {
        var command = CommandCapture.Current;
        bool enabled = false;
        try
        {
            enabled = command.Kind == CommandKind.Block && !command.Closed && ReferenceEquals(command.Receiver, target)
                && CaptureRuntime.Valid(command.Epoch) && ReferenceEquals(command.Epoch.Combat, combatState);
        }
        catch (Exception ex) { command.Incomplete = true; CaptureRuntime.Fail("block-modifier-entry", ex); }
        var scope = BeginCalculation(command.Epoch, block, enabled);
        try
        {
            decimal result = Hook.ModifyBlock(combatState, target, block, props, card, play, out modifiers);
            if (enabled)
            {
                try
                {
                    var contributions = scope.Frame.Contributions(result, false);
                    if (contributions.Length > CommandFrame.MaxModifiers) throw new InvalidOperationException("Block modifier capacity exceeded");
                    command.Modifiers = contributions;
                }
                catch (Exception ex) { command.Incomplete = true; command.Modifiers = Array.Empty<SourceCredit>(); CaptureRuntime.Fail("block-modifier", ex); }
            }
            return result;
        }
        catch
        {
            if (enabled) { command.Incomplete = true; command.Modifiers = Array.Empty<SourceCredit>(); }
            throw;
        }
        finally { EndCalculation(scope); }
    }
    internal static IEnumerable<CodeInstruction> Transpiler(IEnumerable<CodeInstruction> instructions, MethodBase __originalMethod)
    {
        var code = instructions.Select(instruction => new CodeInstruction(instruction)).ToList();
        bool damage = __originalMethod.Name == "ModifyDamageInternal";
        foreach (var kind in new[] { "Additive", "Multiplicative" })
        {
            string family = damage ? "Damage" : "Block";
            var original = AccessTools.DeclaredMethod(typeof(AbstractModel), "Modify" + family + kind);
            var replacements = code.Where(instruction => instruction.Calls(original)).ToArray();
            if (replacements.Length != 1) throw new InvalidOperationException("Modifier observation IL pattern changed: " + family + kind);
            replacements[0].opcode = OpCodes.Call;
            replacements[0].operand = AccessTools.DeclaredMethod(typeof(ModifierCapture), family + kind);
        }
        return code;
    }
    internal static IEnumerable<CodeInstruction> VulnerableTranspiler(IEnumerable<CodeInstruction> instructions)
    {
        var code = instructions.Select(instruction => new CodeInstruction(instruction)).ToList();
        foreach (var entry in new[] { (typeof(PaperPhrog), nameof(Phrog)), (typeof(CrueltyPower), nameof(Cruelty)), (typeof(DebilitatePower), nameof(Debilitate)) })
        {
            var original = AccessTools.DeclaredMethod(entry.Item1, "ModifyVulnerableMultiplier");
            var replacements = code.Where(instruction => instruction.Calls(original)).ToArray();
            if (replacements.Length != 1) throw new InvalidOperationException("Nested Vulnerable observation IL pattern changed: " + entry.Item1.Name);
            replacements[0].opcode = OpCodes.Call;
            replacements[0].operand = AccessTools.DeclaredMethod(typeof(ModifierCapture), entry.Item2);
        }
        return code;
    }
    internal static int Additive(decimal value, bool damage)
        => CaptureRuntime.Backend.CalculateModifierCredit(0, value, damage ? 0 : 1, 0);
    internal static int Increase(decimal basis, decimal multiplier, decimal result)
        => CaptureRuntime.Backend.CalculateModifierCredit(basis, multiplier, 2, result);
    internal static int Product(decimal basis, decimal delta, decimal result)
        => CaptureRuntime.Backend.CalculateModifierCredit(basis, delta, 3, result);
    internal static void Install(Harmony harmony)
    {
        CapturePatches.Patch(harmony, DamageCapture.HookMethod, prefix: new HarmonyMethod(typeof(ModifierCapture), nameof(HookPrefix)),
            postfix: new HarmonyMethod(typeof(ModifierCapture), nameof(HookPostfix)), finalizer: new HarmonyMethod(typeof(ModifierCapture), nameof(ScopeFinalizer)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "ModifyDamageInternal"), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(Transpiler)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "ModifyBlock"), prefix: new HarmonyMethod(typeof(ModifierCapture), nameof(HookPrefix)),
            postfix: new HarmonyMethod(typeof(ModifierCapture), nameof(HookPostfix)), finalizer: new HarmonyMethod(typeof(ModifierCapture), nameof(ScopeFinalizer)), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(Transpiler)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(VulnerablePower), "ModifyDamageMultiplicative"), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(VulnerableTranspiler)));
    }
}
