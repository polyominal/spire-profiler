using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
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
    internal static readonly CommandFrame Barrier = new(default, CommandKind.Unknown, SourceSnapshot.Unavailable, null, 0, 4);
}
internal readonly record struct CommandState(CommandFrame Previous, ProducerFrame Producer);
internal sealed record BuffCapture(CaptureEpoch Epoch, SourceSnapshot Source, decimal Amount, bool Buffer);
internal readonly record struct BlockModifierCapture(CaptureEpoch Epoch, decimal Amount);
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
                slot = receiver is Player player ? SpireProfilerMod.PlayerSlot(player) : 4;
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
                if (CaptureRuntime.Upload(frame.Epoch, frame.Source, transfer => CaptureRuntime.Backend.Forge(frame.Epoch.Sequence, transfer, amount)) != 1)
                    CaptureRuntime.Fail("forge-report");
            }
            else if (frame.Kind == CommandKind.Summon)
            {
                if (CaptureRuntime.Upload(frame.Epoch, frame.Source, transfer => CaptureRuntime.Backend.OstySummoned(frame.Epoch.Sequence, transfer, amount, frame.Slot)) != 1)
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
            var source = operation.Kind == CommandKind.Block && ReferenceEquals(operation.Receiver, receiver) && CaptureRuntime.Valid(operation.Epoch)
                ? operation.Source : SourceSnapshot.Unavailable;
            if (CaptureRuntime.Upload(epoch, source, transfer => CaptureRuntime.Backend.BlockGained(epoch.Sequence, amount, transfer, target.Slot)) != 1)
                CaptureRuntime.Fail("block-report");
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-report", ex); }
    }
    internal static void BlockModifierPrefix(decimal block, out BlockModifierCapture __state)
    {
        __state = default;
        try { __state = new(CaptureRuntime.EntryEpoch(), block); }
        catch (Exception ex) { CaptureRuntime.Fail("block-modifier-entry", ex); }
    }
    internal static void BlockModifierPostfix(decimal __result, BlockModifierCapture __state, Creature target, ValueProp props,
        CardModel cardSource, CardPlay cardPlay, IEnumerable<AbstractModel> modifiers)
    {
        try
        {
            if (!CaptureRuntime.Valid(__state.Epoch) || target == null || !target.IsPlayer || __result <= __state.Amount || modifiers == null) return;
            if (!ReferenceEquals(target.CombatState, __state.Epoch.Combat)) return;
            decimal start = __state.Amount;
            if (cardSource?.Enchantment is { } enchantment)
            {
                start += enchantment.EnchantBlockAdditive(start);
                start *= enchantment.EnchantBlockMultiplicative(start);
            }
            DecomposeBlock(modifiers, start, __result, target, props, cardSource, cardPlay, (model, amount) =>
            {
                var source = FlowCapture.Source(model, __state.Epoch);
                if (CaptureRuntime.Upload(__state.Epoch, source, transfer => CaptureRuntime.Backend.BlockModifier(__state.Epoch.Sequence, transfer, amount, SpireProfilerMod.PlayerSlot(target.Player))) != 1)
                    CaptureRuntime.Fail("block-modifier-report");
            });
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-modifier", ex); }
    }
    internal static void DecomposeBlock(IEnumerable<AbstractModel> modifiers, decimal start, decimal result, Creature target, ValueProp props,
        CardModel card, CardPlay play, Action<AbstractModel, int> contribute)
    {
        decimal running = start;
        var multiplicative = new List<AbstractModel>();
        foreach (var model in modifiers)
        {
            decimal addition = model switch
            {
                PowerModel power => power.ModifyBlockAdditive(target, running, props, card, play),
                RelicModel relic => relic.ModifyBlockAdditive(target, running, props, card, play),
                _ => 0m
            };
            running += addition;
            if (addition > 0) contribute(model, checked((int)addition));
            else if (model is PowerModel or RelicModel)
            {
                if (multiplicative.Count == 64) { CaptureRuntime.Fail("block-modifier-cap"); return; }
                multiplicative.Add(model);
            }
        }
        foreach (var model in multiplicative)
        {
            decimal multiplier = model is PowerModel power ? power.ModifyBlockMultiplicative(target, running, props, card, play)
                : ((RelicModel)model).ModifyBlockMultiplicative(target, running, props, card, play);
            decimal before = Math.Min(running, result);
            running *= multiplier;
            if (multiplier <= 1) continue;
            int amount = checked((int)(before * (multiplier - 1)));
            if (amount > 0) contribute(model, amount);
        }
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
            if (prevented > 0 && CaptureRuntime.Upload(__state.Epoch, __state.Source, transfer => CaptureRuntime.Backend.BuffMitigation(__state.Epoch.Sequence, transfer, prevented)) != 1)
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
            if (CaptureRuntime.Upload(epoch, source, transfer => CaptureRuntime.Backend.OrbChanneled(epoch.Sequence, identity.Identity, transfer)) == 1) identity.Dirty = false;
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
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (side == CombatSide.Player && CaptureRuntime.Valid(epoch) && ReferenceEquals(epoch.Combat, combatState)
                && CaptureRuntime.Backend.TurnStarted(epoch.Sequence) != 1) CaptureRuntime.Fail("turn-start");
        }
        catch (Exception ex) { CaptureRuntime.Fail("turn-start", ex); }
    }
    internal static void ClearBlockPrefix(object combatState, object creature)
    {
        try
        {
            var epoch = CaptureRuntime.EntryEpoch();
            if (!CaptureRuntime.Valid(epoch) || !ReferenceEquals(epoch.Combat, combatState)) return;
            var target = CaptureRuntime.Backend.DescribeCreature(creature);
            if (target.Player && ReferenceEquals(target.Combat, epoch.Combat) && CaptureRuntime.Backend.BlockCleared(epoch.Sequence, target.Slot) != 1)
                CaptureRuntime.Fail("block-clear");
        }
        catch (Exception ex) { CaptureRuntime.Fail("block-clear", ex); }
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
        => CapturePatches.Patch(harmony, method, prefix: new HarmonyMethod(typeof(CommandCapture), nameof(Prefix)), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(Postfix)), finalizer: new HarmonyMethod(typeof(CommandCapture), nameof(Finalizer)));
    internal static void Install(Harmony harmony, Action<string> report)
    {
        PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(CreatureCmd), "GainBlock", new[] { typeof(Creature), typeof(decimal), typeof(ValueProp), typeof(CardPlay), typeof(bool) }));
        PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(ForgeCmd), "Forge", new[] { typeof(decimal), typeof(Player), typeof(AbstractModel) }));
        PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(OstyCmd), "Summon", new[] { typeof(PlayerChoiceContext), typeof(Player), typeof(decimal), typeof(AbstractModel) }));
        var prefix = new HarmonyMethod(typeof(CommandCapture), nameof(BlockModifierPrefix));
        var postfix = new HarmonyMethod(typeof(CommandCapture), nameof(BlockModifierPostfix));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "ModifyBlock"), prefix: prefix, postfix: postfix);
        foreach (var entry in new[] { (typeof(BufferPower), "ModifyHpLostAfterOstyLate"), (typeof(IntangiblePower), "ModifyHpLostAfterOsty"), (typeof(HardenedShellPower), "ModifyHpLostBeforeOstyLate") })
            CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(entry.Item1, entry.Item2), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(BuffPrefix)), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(BuffPostfix)));
        foreach (var entry in new[] { ("BlockGained", nameof(BlockGained)), ("OrbChanneled", nameof(OrbChanneled)), ("PotionUsed", nameof(PotionPostfix)) })
            CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatHistory), entry.Item1), postfix: new HarmonyMethod(typeof(CommandCapture), entry.Item2));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatManager), "SetUpCombat"), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(SetupPrefix)), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(SetupPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CombatRoom), "OnCombatEnded"), postfix: new HarmonyMethod(typeof(CommandCapture), nameof(CombatEndedPostfix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "AfterSideTurnStart"), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(TurnPrefix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(Hook), "AfterBlockCleared"), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(ClearBlockPrefix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CreatureCmd), "Kill", new[] { typeof(Creature), typeof(bool) }), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(KillPrefix)));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(CreatureCmd), "Kill", new[] { typeof(IReadOnlyCollection<Creature>), typeof(bool) }), prefix: new HarmonyMethod(typeof(CommandCapture), nameof(KillManyPrefix)));
        report("COMMAND CAPTURE block=1 forge=1 summon=1 block_modifier=1 buffs=3 histories=3 lifecycle=4 kill=2");
    }
}
