using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Commands;
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
    internal bool Failed, Closed;
    internal ModifierFrame(CaptureEpoch epoch, decimal initial) { Epoch = epoch; Initial = initial; }
    internal void Record(ObservedModifier observation)
    {
        if (Closed) return;
        if (Observations.Count == 64) { Failed = true; CaptureRuntime.Fail("modifier-observation-cap"); return; }
        Observations.Add(observation);
    }
    internal ModifierCredit[] Contributions(decimal result, bool damage)
    {
        if (Failed || !CaptureRuntime.Valid(Epoch)) throw new InvalidOperationException("Modifier observation is incomplete");
        var input = new ModifierObservation[Observations.Sum(observation => 1 + (observation.Nested?.Count ?? 0))];
        int index = 0;
        foreach (var observation in Observations)
        {
            int parent = index;
            input[index++] = Encode(observation, ModifierObservation.TopLevel, observation.Vulnerable ? ModifierObservation.Vulnerable
                : observation.Multiplicative ? ModifierObservation.Multiplicative : ModifierObservation.Additive);
            if (observation.Nested != null)
                foreach (var part in observation.Nested) input[index++] = Encode(part, parent, ModifierObservation.Nested);
        }
        return CaptureRuntime.Backend.CalculateModifierContributions(input, Initial, result, damage);
    }
    private ModifierObservation Encode(ObservedModifier observation, int parent, int kind)
    {
        var source = observation.Source;
        if (source.Epoch != 0 && source.Epoch != Epoch.Sequence) throw new InvalidOperationException("Modifier source belongs to another combat");
        return new(source.Handle, observation.Input, observation.Output, parent, kind);
    }

}
internal readonly record struct ModifierScope(ModifierFrame Previous, ModifierFrame Frame, bool Entered = true);
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
        var scope = new ModifierScope(current.Value, frame);
        admitted.Value = frame;
        current.Value = frame;
        return scope;
    }
    internal static void EndCalculation(ModifierScope scope)
    {
        if (scope.Frame != null) scope.Frame.Closed = true;
        if (ReferenceEquals(admitted.Value, scope.Frame)) admitted.Value = null;
        current.Value = scope.Previous;
    }
    internal static void HookPrefix(out ModifierScope __state)
    {
        __state = new(current.Value, admitted.Value);
        current.Value = admitted.Value;
        admitted.Value = null;
    }
    internal static void DamageHookPostfix(bool __runOriginal, ModifierScope __state)
    {
        if (__runOriginal || __state.Frame == null) return;
        __state.Frame.Failed = true;
        CaptureRuntime.Fail("damage-hook-skipped");
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
        CaptureEpoch epoch = default;
        try
        {
            var command = CommandCapture.Current;
            if (command.Kind == CommandKind.Block && ReferenceEquals(command.Receiver, target)
                && CaptureRuntime.Valid(command.Epoch) && ReferenceEquals(command.Epoch.Combat, combatState)) epoch = command.Epoch;
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-modifier-entry", ex); }
        var scope = BeginCalculation(epoch, block, epoch.Sequence != 0);
        try { return Hook.ModifyBlock(combatState, target, block, props, card, play, out modifiers); }
        finally { EndCalculation(scope); }
    }
    internal static void BlockPostfix(decimal __result, ModifierScope __state, Creature target, bool __runOriginal)
    {
        var frame = __state.Frame;
        if (frame == null) return;
        frame.Closed = true;
        try
        {
            if (!CaptureRuntime.Valid(frame.Epoch)) return;
            if (!__runOriginal) throw new InvalidOperationException("Original block hook did not run");
            var receiver = CaptureRuntime.Backend.DescribeCreature(target);
            if (!receiver.Player || !ReferenceEquals(receiver.Combat, frame.Epoch.Combat)) return;
            foreach (var contribution in frame.Contributions(__result, false))
                if (CaptureRuntime.Backend.BlockModifier(frame.Epoch.Sequence, contribution.Source, contribution.Amount, receiver.Slot) != 1)
                    throw new InvalidOperationException("Block modifier observation was rejected");
        }
        catch (Exception ex)
        {
            CommandCapture.InvalidateBlockSource(frame.Epoch);
            CaptureRuntime.Fail("block-modifier", ex);
        }
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
    internal static IEnumerable<CodeInstruction> BlockCommandTranspiler(IEnumerable<CodeInstruction> instructions)
    {
        var code = instructions.Select(instruction => new CodeInstruction(instruction)).ToList();
        var original = AccessTools.DeclaredMethod(typeof(Hook), "ModifyBlock");
        var replacements = code.Where(instruction => instruction.Calls(original)).ToArray();
        if (replacements.Length != 1) throw new InvalidOperationException("Canonical block calculation IL pattern changed");
        replacements[0].operand = AccessTools.DeclaredMethod(typeof(ModifierCapture), nameof(ModifyBlock));
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
    internal static void Install(Harmony harmony)
    {
        CapturePatches.Patch(harmony, DamageCapture.HookMethod, prefix: new HarmonyMethod(typeof(ModifierCapture), nameof(HookPrefix)),
            postfix: new HarmonyMethod(typeof(ModifierCapture), nameof(DamageHookPostfix)), finalizer: new HarmonyMethod(typeof(ModifierCapture), nameof(ScopeFinalizer)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "ModifyDamageInternal"), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(Transpiler)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "ModifyBlock"), prefix: new HarmonyMethod(typeof(ModifierCapture), nameof(HookPrefix)),
            postfix: new HarmonyMethod(typeof(ModifierCapture), nameof(BlockPostfix)), finalizer: new HarmonyMethod(typeof(ModifierCapture), nameof(ScopeFinalizer)), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(Transpiler)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(VulnerablePower), "ModifyDamageMultiplicative"), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(VulnerableTranspiler)));
        var blockCommand = AccessTools.DeclaredMethod(typeof(CreatureCmd), "GainBlock", new[] { typeof(Creature), typeof(decimal), typeof(ValueProp), typeof(CardPlay), typeof(bool) });
        CapturePatches.Patch(harmony, TemporalPowerCapture.Body(blockCommand), transpiler: new HarmonyMethod(typeof(ModifierCapture), nameof(BlockCommandTranspiler)));
    }
}
