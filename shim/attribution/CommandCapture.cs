using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Combat.History;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Hooks;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Monsters;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Rooms;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

internal enum CommandKind { Unknown, Block, Forge, Summon }
internal sealed record CommandFrame(CaptureEpoch Epoch, CommandKind Kind, SourceSnapshot Source, object Receiver, decimal Amount, int Slot)
{
    internal const int MaxModifiers = 16;
    // This object follows the command's ExecutionContext through awaits. Only
    // its own physical history event consumes the modifier batch.
    internal SourceCredit[] Modifiers = Array.Empty<SourceCredit>();
    internal bool Incomplete, Closed;
    internal static readonly CommandFrame Barrier = new(default, CommandKind.Unknown, SourceSnapshot.Unavailable, null, 0, 4);
}
internal readonly record struct CommandState(CommandFrame Previous, ProducerFrame Producer);
internal sealed record BuffCapture(CaptureEpoch Epoch, SourceSnapshot Source, decimal Amount, bool Buffer);
internal readonly record struct BlockLossState(CaptureEpoch Epoch, int Slot, int Before);
internal static class CommandCapture
{
    private static readonly AsyncLocal<CommandFrame> current = new();
    internal static CommandFrame Current { get => current.Value ?? CommandFrame.Barrier; private set => current.Value = value; }
    internal static void Prefix(MethodBase __originalMethod, object[] __args, out CommandState __state)
    {
        __state = new(Current, FlowCapture.Current);
        Current = CommandFrame.Barrier;
        FlowCapture.Current = ProducerFrame.Barrier;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Producer.Epoch);
            if (CaptureRuntime.Stale(__state.Previous.Epoch)) epoch = __state.Previous.Epoch;
            Current = CommandFrame.Barrier with { Epoch = epoch };
            FlowCapture.Current = ProducerFrame.Barrier with { Epoch = epoch };
            if (!CaptureRuntime.Valid(epoch)) return;
            object model, receiver;
            decimal amount;
            CommandKind kind;
            int slot;
            if (__originalMethod.Name == "GainBlock")
            {
                receiver = __args[0]; amount = (decimal)__args[1]; model = (__args[3] as CardPlay)?.Card;
                kind = CommandKind.Block; slot = CaptureRuntime.Backend.DescribeCreature(receiver).Slot;
            }
            else
            {
                bool forge = __originalMethod.Name == "Forge";
                receiver = __args[1]; amount = (decimal)__args[forge ? 0 : 2]; model = __args[forge ? 2 : 3];
                kind = forge ? CommandKind.Forge : CommandKind.Summon;
                slot = receiver is Player player ? RunContext.PlayerSlot(player) : 4;
            }
            Current = new(epoch, kind, SourceSnapshot.Unavailable, receiver, amount, slot);
            var source = FlowCapture.Supplied(model, epoch, __state.Producer);
            Current = Current with { Source = source };
            var descriptor = model == null ? default : CaptureRuntime.Backend.Describe(model);
            var role = model == null ? __state.Producer.Role : descriptor.Role;
            var segment = model == null ? __state.Producer.Segment : role is ProducerRole.Card or ProducerRole.Relic or ProducerRole.Potion ? DamageSegment.Direct : DamageSegment.Attributed;
            FlowCapture.Current = new(epoch, model ?? __state.Producer.Model, source, role, segment);
        }
        catch (Exception ex) { CaptureRuntime.Fail("command-entry", ex); }
    }
    internal static void Postfix(CommandState __state, bool __runOriginal)
    {
        try
        {
            if (__state.Previous == null || !__runOriginal) return;
            var frame = Current;
            if (!CaptureRuntime.Valid(frame.Epoch) || frame.Amount <= 0) return;
            int amount = checked((int)frame.Amount);
            if (amount <= 0) return;
            if (frame.Kind == CommandKind.Forge)
            {
                if (CaptureRuntime.WithSource(frame.Epoch, frame.Source, transfer => CaptureRuntime.Backend.Forge(frame.Epoch.Sequence, transfer, amount)) != 1)
                    CaptureRuntime.Fail("forge-report");
            }
            else if (frame.Kind == CommandKind.Summon)
            {
                if (CaptureRuntime.WithSource(frame.Epoch, frame.Source, transfer => CaptureRuntime.Backend.OstySummoned(frame.Epoch.Sequence, transfer, amount, frame.Slot)) != 1)
                    CaptureRuntime.Fail("summon-report");
            }
        }
        catch (Exception ex) { CaptureRuntime.Fail("command-report", ex); }
    }
    internal static void Finalizer(CommandState __state)
    {
        if (__state.Previous == null) return;
        Current = __state.Previous;
        FlowCapture.Current = __state.Producer;
    }
    internal static void BlockGained(object combatState, object receiver, int amount)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch) || !ReferenceEquals(epoch.Combat, combatState)) return;
            var target = CaptureRuntime.Backend.DescribeCreature(receiver);
            if (!target.Player || !ReferenceEquals(target.Combat, epoch.Combat)) return;
            var operation = Current;
            bool owns = operation.Kind == CommandKind.Block && !operation.Closed && ReferenceEquals(operation.Receiver, receiver) && CaptureRuntime.Valid(operation.Epoch);
            var source = owns ? operation.Source : SourceSnapshot.Unavailable;
            var modifiers = owns ? operation.Modifiers : Array.Empty<SourceCredit>();
            bool incomplete = owns && operation.Incomplete;
            if (owns) { operation.Modifiers = Array.Empty<SourceCredit>(); operation.Closed = true; }
            try
            {
                if (CaptureRuntime.WithSource(epoch, source, transfer => CaptureRuntime.Backend.BlockGained(epoch.Sequence, amount, transfer, target.Slot, modifiers, incomplete)) != 1)
                    CaptureRuntime.Fail("block-report");
            }
            finally { GC.KeepAlive(modifiers); }
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-report", ex); }
    }
    internal static void BlockCompleted(ref AsyncTaskMethodBuilder<decimal> builder, decimal result)
    {
        var command = Current;
        if (command.Kind == CommandKind.Block) { command.Modifiers = Array.Empty<SourceCredit>(); command.Closed = true; }
        builder.SetResult(result);
    }
    internal static void BlockFailed(ref AsyncTaskMethodBuilder<decimal> builder, Exception error)
    {
        var command = Current;
        if (command.Kind == CommandKind.Block) { command.Modifiers = Array.Empty<SourceCredit>(); command.Closed = true; }
        builder.SetException(error);
    }
    internal static IEnumerable<CodeInstruction> BlockTranspiler(IEnumerable<CodeInstruction> instructions)
    {
        var code = instructions.Select(instruction => new CodeInstruction(instruction)).ToList();
        var replacements = new[]
        {
            (AccessTools.DeclaredMethod(typeof(Hook), "ModifyBlock"), AccessTools.DeclaredMethod(typeof(ModifierCapture), nameof(ModifierCapture.ModifyBlock))),
            (AccessTools.DeclaredMethod(typeof(AsyncTaskMethodBuilder<decimal>), "SetResult", new[] { typeof(decimal) }), AccessTools.DeclaredMethod(typeof(CommandCapture), nameof(BlockCompleted))),
            (AccessTools.DeclaredMethod(typeof(AsyncTaskMethodBuilder<decimal>), "SetException", new[] { typeof(Exception) }), AccessTools.DeclaredMethod(typeof(CommandCapture), nameof(BlockFailed)))
        };
        foreach (var (original, bridge) in replacements)
        {
            var matches = code.Where(instruction => instruction.Calls(original)).ToArray();
            if (matches.Length != 1) throw new InvalidOperationException("Canonical block IL pattern changed: " + original.Name);
            matches[0].opcode = OpCodes.Call;
            matches[0].operand = bridge;
        }
        return code;
    }
    internal static void BuffPrefix(object __instance, decimal amount, out BuffCapture __state)
    {
        __state = null;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            __state = new(epoch, SourceSnapshot.Unavailable, amount, __instance is BufferPower);
            if (CaptureRuntime.Valid(epoch)) __state = __state with { Source = FlowCapture.Source(__instance, epoch) };
        }
        catch (Exception ex) { CaptureRuntime.Fail("buff-entry", ex); }
    }
    internal static void BuffPostfix(decimal __result, BuffCapture __state, object target)
    {
        try
        {
            if (__state == null || !CaptureRuntime.Valid(__state.Epoch) || __state.Amount <= __result || __result < 0) return;
            var receiver = CaptureRuntime.Backend.DescribeCreature(target);
            if (!receiver.Player || !ReferenceEquals(receiver.Combat, __state.Epoch.Combat) || (__state.Buffer && __result != 0)) return;
            int prevented = checked((int)(__state.Amount - __result));
            if (prevented > 0 && CaptureRuntime.WithSource(__state.Epoch, __state.Source, transfer => CaptureRuntime.Backend.BuffMitigation(__state.Epoch.Sequence, transfer, prevented)) != 1)
                CaptureRuntime.Fail("buff-report");
        }
        catch (Exception ex) { CaptureRuntime.Fail("buff-report", ex); }
    }
    internal static void OrbChanneled(object combatState, object orb)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch) || !ReferenceEquals(epoch.Combat, combatState)) return;
            var identity = IdentityCapture.Get(orb, epoch);
            if (identity == null) return;
            identity.Dirty = true;
            var source = FlowCapture.Supplied(null, epoch);
            if (CaptureRuntime.WithSource(epoch, source, transfer => CaptureRuntime.Backend.OrbChanneled(epoch.Sequence, identity.Identity, transfer)) == 1) identity.Dirty = false;
            else CaptureRuntime.Fail("orb-channel");
        }
        catch (Exception ex) { CaptureRuntime.Fail("orb-channel", ex); }
    }
    internal static void SetupPrefix()
    {
        try
        {
            CaptureRuntime.InvalidateEpoch();
            IdentityCapture.PrepareCombat();
        }
        catch (Exception ex) { CaptureRuntime.Fail("combat-setup-entry", ex); }
    }
    internal static void SetupPostfix(CombatState state, bool __runOriginal)
    {
        try
        {
            if (!__runOriginal || !CaptureRuntime.OnThread || !ReferenceEquals(CaptureRuntime.Backend.CurrentCombat, state)) return;
            var encounter = state?.Encounter;
            ulong epoch = CaptureRuntime.Backend.CombatStarted(encounter?.Id?.Entry ?? "unknown", (encounter?.RoomType.ToString() ?? "normal").ToLowerInvariant());
            CaptureRuntime.Register(CaptureRuntime.Backend, epoch, state);
            if (!CaptureRuntime.Valid(CaptureRuntime.Epoch)) CaptureRuntime.Fail("combat-start");
        }
        catch (Exception ex) { CaptureRuntime.Fail("combat-start", ex); }
    }
    internal static void CombatEndedPostfix(CombatRoom __instance)
    {
        try
        {
            var epoch = CaptureRuntime.Epoch;
            if (!CaptureRuntime.Valid(epoch) || !ReferenceEquals(epoch.Combat, __instance.CombatState)) return;
            try { if (CaptureRuntime.Backend.CombatEnded(epoch.Sequence) != 1) CaptureRuntime.Fail("combat-end"); }
            finally { CaptureRuntime.InvalidateEpoch(); }
        }
        catch (Exception ex) { CaptureRuntime.Fail("combat-end", ex); }
    }
    internal static void TurnPrefix(object combatState, CombatSide side)
    {
        PoisonAudit.Checkpoint("turn", combatState, side);
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (side == CombatSide.Player && CaptureRuntime.Valid(epoch) && ReferenceEquals(epoch.Combat, combatState)
                && CaptureRuntime.Backend.TurnStarted(epoch.Sequence) != 1) CaptureRuntime.Fail("turn-start");
        }
        catch (Exception ex) { CaptureRuntime.Fail("turn-start", ex); }
    }
    internal static void ClearBlockPostfix(bool __result, object combatState, object creature)
    {
        try
        {
            if (!__result) return;
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch) || !ReferenceEquals(epoch.Combat, combatState)) return;
            var target = CaptureRuntime.Backend.DescribeCreature(creature);
            if (target.Player && ReferenceEquals(target.Combat, epoch.Combat) && CaptureRuntime.Backend.BlockCleared(epoch.Sequence, target.Slot) != 1)
                CaptureRuntime.Fail("block-clear");
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-clear", ex); }
    }
    internal static void BlockLossPrefix(Creature __instance, out BlockLossState __state)
    {
        __state = default;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch)) return;
            var target = CaptureRuntime.Backend.DescribeCreature(__instance);
            if (target.Player && ReferenceEquals(target.Combat, epoch.Combat))
                __state = new(epoch, target.Slot, __instance.Block);
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-loss-entry", ex); }
    }
    internal static void BlockLossFinalizer(Creature __instance, BlockLossState __state)
    {
        try
        {
            if (!CaptureRuntime.Valid(__state.Epoch)) return;
            int lost = __state.Before - __instance.Block;
            if (lost > 0 && CaptureRuntime.Backend.BlockLost(__state.Epoch.Sequence, __state.Slot, lost) != 1)
                CaptureRuntime.Fail("block-loss");
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-loss", ex); }
    }
    internal static void PotionPostfix(object combatState)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (CaptureRuntime.Valid(epoch) && ReferenceEquals(epoch.Combat, combatState) && CaptureRuntime.Backend.PotionUsed(epoch.Sequence) != 1)
                CaptureRuntime.Fail("potion-used");
        }
        catch (Exception ex) { CaptureRuntime.Fail("potion-used", ex); }
    }
    internal static void KillPrefix(Creature creature) { Killed(creature); }
    internal static void KillManyPrefix(IReadOnlyCollection<Creature> creatures)
    {
        try
        {
            if (creatures == null) return;
            foreach (var creature in creatures) Killed(creature);
        }
        catch (Exception ex) { CaptureRuntime.Fail("creature-killed-list", ex); }
    }
    internal static void Killed(object creature)
    {
        try
        {
            if (creature == null) return;
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch)) return;
            var target = CaptureRuntime.Backend.DescribeCreature(creature);
            if (!ReferenceEquals(target.Combat, epoch.Combat)) return;
            if (target.Osty)
            {
                var play = PlayCapture.Current;
                ulong token = play != null && play.Slot == target.Slot && CaptureRuntime.Valid(play.Epoch) ? play.Token : 0;
                if (CaptureRuntime.Backend.OstyKilled(epoch.Sequence, target.Slot, token) != 1) CaptureRuntime.Fail("osty-killed");
            }
            else if (target.Player && CaptureRuntime.Backend.PlayerDied(epoch.Sequence, target.Slot) != 1) CaptureRuntime.Fail("player-died");
        }
        catch (Exception ex) { CaptureRuntime.Fail("creature-killed", ex); }
    }
    internal static void PatchCommand(Harmony harmony, MethodInfo method)
    {
        CapturePatches.Patch(harmony, method, prefix: new HarmonyMethod(typeof(CommandCapture), nameof(Prefix)), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(Postfix)), finalizer: new HarmonyMethod(typeof(CommandCapture), nameof(Finalizer)));
        if (method.Name == "GainBlock") CapturePatches.Patch(harmony, TemporalPowerCapture.Body(method), transpiler: new HarmonyMethod(typeof(CommandCapture), nameof(BlockTranspiler)));
    }
    internal static void Install(Harmony harmony, Action<string> report)
    {
        PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(CreatureCmd), "GainBlock", new[] { typeof(Creature), typeof(decimal), typeof(ValueProp), typeof(CardPlay), typeof(bool) }));
        PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(ForgeCmd), "Forge", new[] { typeof(decimal), typeof(Player), typeof(AbstractModel) }));
        PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(OstyCmd), "Summon", new[] { typeof(PlayerChoiceContext), typeof(Player), typeof(decimal), typeof(AbstractModel) }));
        foreach (var entry in new[] { (typeof(BufferPower), "ModifyHpLostAfterOstyLate"), (typeof(IntangiblePower), "ModifyHpLostAfterOsty"), (typeof(HardenedShellPower), "ModifyHpLostBeforeOstyLate") })
            CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(entry.Item1, entry.Item2), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(BuffPrefix)), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(BuffPostfix)));
        foreach (var entry in new[] { ("BlockGained", nameof(BlockGained)), ("OrbChanneled", nameof(OrbChanneled)), ("PotionUsed", nameof(PotionPostfix)) })
            CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatHistory), entry.Item1), postfix: new HarmonyMethod(typeof(CommandCapture), entry.Item2));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatManager), "SetUpCombat"), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(SetupPrefix)), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(SetupPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatRoom), "OnCombatEnded"), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(CombatEndedPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "AfterSideTurnStart"), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(TurnPrefix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "ShouldClearBlock"), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(ClearBlockPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Creature), "LoseBlockInternal", new[] { typeof(decimal) }),
            prefix: new HarmonyMethod(typeof(CommandCapture), nameof(BlockLossPrefix)), finalizer: new HarmonyMethod(typeof(CommandCapture), nameof(BlockLossFinalizer)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CreatureCmd), "Kill", new[] { typeof(Creature), typeof(bool) }), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(KillPrefix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CreatureCmd), "Kill", new[] { typeof(IReadOnlyCollection<Creature>), typeof(bool) }), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(KillManyPrefix)));
        report("COMMAND CAPTURE block=1 forge=1 summon=1 block_modifier=1 block_loss=1 buffs=3 histories=3 lifecycle=4 kill=2");
    }
}
