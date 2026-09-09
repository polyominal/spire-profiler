using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Hooks;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Models.Relics;
using MegaCrit.Sts2.Core.Runs;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

internal sealed record DamageOperation(CaptureEpoch Epoch, SourceSnapshot Source, ProducerRole Role, DamageSegment Segment, object Dealer,
    bool ExplicitCard, object PoisonOwner, ulong Calculation = 0, bool Weak = false, bool Debilitate = false, uint PaperKraneSlots = 0)
{
    internal static readonly DamageOperation Barrier = new(default, SourceSnapshot.Unavailable, ProducerRole.Unknown, DamageSegment.Attributed, null, false, null);
}
internal readonly record struct DamageEvidence(bool Modifiers, bool EnemyHit, int Strength, object WeakPower, bool Debilitate, uint PaperKraneSlots = 0, object StrengthPower = null);
internal static class DamageCapture
{
    // Canonical target-local groups have one receiver plus at most one redirect.
    internal const int MaxResults = 2;
    private static readonly AsyncLocal<DamageOperation> operation = new();
    internal static DamageOperation Current { get => operation.Value ?? DamageOperation.Barrier; private set => operation.Value = value; }
    internal delegate decimal ModifyDamageCall(IRunState runState, ICombatState combatState, Creature target, Creature dealer, decimal damage,
        ValueProp props, CardModel cardSource, CardPlay cardPlay, ModifyDamageHookType hookType, CardPreviewMode previewMode, out IEnumerable<AbstractModel> modifiers);
    internal static ModifyDamageCall OriginalModifyDamage = Hook.ModifyDamage;
    internal static MethodInfo HookMethod => ((ModifyDamageCall)Hook.ModifyDamage).Method;
    internal static Func<Creature, Creature, ValueProp, DamageEvidence> Inspect = InspectGame;
    internal static void Prefix(object[] __args, out DamageOperation __state)
    {
        __state = Current;
        Current = DamageOperation.Barrier;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Epoch);
            object dealer = __args[4], card = __args[5];
            // Classification metadata survives a later source-capture failure.
            Current = new(epoch, SourceSnapshot.Unavailable, ProducerRole.Unknown, DamageSegment.Attributed, dealer, card != null, null);
            if (!CaptureRuntime.Valid(epoch)) return;
            var producer = FlowCapture.Current;
            if (card != null)
                Current = Current with { Source = FlowCapture.Source(card, epoch), Role = ProducerRole.Card, Segment = DamageSegment.Direct };
            else if ((dealer == null || !CaptureRuntime.Backend.DescribeCreature(dealer).Osty) && CaptureRuntime.Valid(producer.Epoch))
            {
                SourceSnapshot source = producer.Poison ? FlowCapture.Source(producer.Model, epoch) : producer.Source;
                object owner = producer.Poison ? CaptureRuntime.Backend.Describe(producer.Model).Owner : null;
                if (producer.Poison && (owner == null || CaptureRuntime.Backend.ObservePower(producer.Model).Amount <= 0)) source = SourceSnapshot.Unavailable;
                Current = Current with { Source = source, Role = producer.Role, Segment = producer.Segment, PoisonOwner = owner };
            }
        }
        catch (Exception ex) { CaptureRuntime.Fail("damage-prefix", ex); }
    }
    internal static void Finalizer(DamageOperation __state) { if (__state != null) Current = __state; }
    private static DamageEvidence InspectGame(Creature target, Creature dealer, ValueProp props)
    {
        bool modifiers = dealer != null && dealer.IsPlayer && target != null && !target.IsPlayer && props.HasFlag(ValueProp.Move) && !props.HasFlag(ValueProp.Unpowered);
        bool enemy = dealer != null && !dealer.IsPlayer && target != null && target.IsPlayer;
        var weak = enemy ? dealer.GetPower<WeakPower>() : null;
        var strength = dealer?.GetPower<StrengthPower>();
        return new(modifiers, enemy && SpireProfilerMod.IsLocalPlayer(target.Player), strength?.Amount ?? 0,
            weak, enemy && dealer.GetPower<DebilitatePower>() != null, weak == null ? 0 : SpireProfilerMod.PaperKraneSlots(), strength);
    }
    internal static decimal ModifyDamage(IRunState runState, ICombatState combatState, Creature target, Creature dealer, decimal damage,
        ValueProp props, CardModel cardSource, CardPlay cardPlay, ModifyDamageHookType hookType, CardPreviewMode previewMode, out IEnumerable<AbstractModel> modifiers)
    {
        Abort();
        var captured = Current;
        Current = captured with { Calculation = 0, Weak = false, Debilitate = false };
        DamageEvidence evidence = default;
        ulong calculation = 0;
        try
        {
            if (CaptureRuntime.Valid(captured.Epoch) && ReferenceEquals(combatState, captured.Epoch.Combat)
                && ReferenceEquals(CaptureRuntime.Backend.DescribeCreature(target).Combat, captured.Epoch.Combat))
            {
                var targetIdentity = IdentityCapture.Get(target, captured.Epoch);
                var source = captured.PoisonOwner != null && !ReferenceEquals(captured.PoisonOwner, target) ? SourceSnapshot.Unavailable : captured.Source;
                if (targetIdentity != null)
                {
                    evidence = Inspect(target, dealer, props);
                    Current = captured with { Calculation = 0, Weak = evidence.WeakPower != null, Debilitate = evidence.Debilitate, PaperKraneSlots = evidence.PaperKraneSlots };
                    calculation = CaptureRuntime.Upload(captured.Epoch, source, transfer => CaptureRuntime.Backend.DamageBegin(captured.Epoch.Sequence, transfer, captured.Role, captured.Segment, targetIdentity.Identity));
                    if (calculation != 0)
                    {
                        if (evidence.EnemyHit)
                        {
                            var dealerIdentity = IdentityCapture.Get(dealer, captured.Epoch);
                            var strengthIdentity = evidence.StrengthPower == null ? null : IdentityCapture.Get(evidence.StrengthPower, captured.Epoch);
                            if (evidence.StrengthPower != null && (strengthIdentity == null || strengthIdentity.Dirty))
                                throw new InvalidOperationException("Enemy Strength provenance is unavailable");
                            if (dealerIdentity == null || CaptureRuntime.Backend.DamageEnemyHit(calculation, dealerIdentity.Identity, checked((int)damage), evidence.Strength) != 1)
                                throw new InvalidOperationException("Enemy hit capture failed");
                        }
                        if (evidence.WeakPower != null)
                        {
                            var weakIdentity = IdentityCapture.Get(evidence.WeakPower, captured.Epoch);
                            var weak = weakIdentity == null || weakIdentity.Dirty ? SourceSnapshot.Unavailable
                                : CaptureRuntime.Copy(captured.Epoch, CaptureKind.WeakHead, weakIdentity.Identity);
                            if (CaptureRuntime.Upload(captured.Epoch, weak, transfer => CaptureRuntime.Backend.DamageWeak(calculation, transfer)) != 1)
                                throw new InvalidOperationException("Weak capture failed");
                        }
                        Current = captured with { Calculation = calculation, Weak = evidence.WeakPower != null, Debilitate = evidence.Debilitate, PaperKraneSlots = evidence.PaperKraneSlots };
                    }
                }
            }
        }
        catch (Exception ex) { AbortToken(calculation); calculation = 0; CaptureRuntime.Fail("damage-live-begin", ex); }
        decimal result = OriginalModifyDamage(runState, combatState, target, dealer, damage, props, cardSource, cardPlay, hookType, previewMode, out modifiers);
        try
        {
            if (calculation != 0 && evidence.Modifiers && result > damage && modifiers != null)
            {
                Current = Current with { Calculation = 0 };
                decimal start = damage;
                if (cardSource?.Enchantment is { } enchantment)
                {
                    start += enchantment.EnchantDamageAdditive(start, props);
                    start *= enchantment.EnchantDamageMultiplicative(start, props);
                }
                Decompose(modifiers, start, result, target, dealer, props, cardSource, (model, amount) =>
                {
                    var source = FlowCapture.Source(model, captured.Epoch);
                    if (CaptureRuntime.Upload(captured.Epoch, source, transfer => CaptureRuntime.Backend.DamageModifier(calculation, transfer, amount)) != 1)
                        throw new InvalidOperationException("Damage modifier rejected");
                });
                Current = Current with { Calculation = calculation };
            }
        }
        catch (Exception ex) { AbortToken(calculation); Current = Current with { Calculation = 0 }; CaptureRuntime.Fail("damage-live-modifier", ex); }
        return result;
    }
    internal static void Decompose(IEnumerable<AbstractModel> modifiers, decimal start, decimal result, Creature target, Creature dealer,
        ValueProp props, CardModel card, Action<AbstractModel, int> contribute)
    {
        var multiplicative = new List<AbstractModel>();
        int observed = 0;
        decimal running = start;
        foreach (var model in modifiers)
        {
            if (model is PowerModel or RelicModel && ++observed > 64) throw new InvalidOperationException("Damage modifier capacity exceeded");
            decimal addition = model switch
            {
                PowerModel power => power.ModifyDamageAdditive(target, running, props, dealer, card, null),
                RelicModel relic => relic.ModifyDamageAdditive(target, running, props, dealer, card, null),
                _ => 0m
            };
            running += addition;
            if (addition != 0) contribute(model, Math.Abs(checked((int)addition)));
            else if (model is PowerModel or RelicModel) multiplicative.Add(model);
        }
        foreach (var model in multiplicative)
        {
            decimal multiplier = model is PowerModel power ? power.ModifyDamageMultiplicative(target, running, props, dealer, card, null)
                : ((RelicModel)model).ModifyDamageMultiplicative(target, running, props, dealer, card, null);
            decimal before = Math.Min(running, result);
            running *= multiplier;
            if (multiplier <= 1) continue;
            if (model is VulnerablePower vulnerable && vulnerable.DynamicVars.TryGetValue("DamageIncrease", out var increase))
            {
                decimal composite = increase.BaseValue;
                var parts = new List<(AbstractModel Model, decimal Delta)> { (vulnerable, composite - 1) };
                var nested = new AbstractModel[] { dealer.Player?.GetRelic<PaperPhrog>(), dealer.GetPower<CrueltyPower>() ?? dealer.PetOwner?.Creature.GetPower<CrueltyPower>(), target.GetPower<DebilitatePower>() };
                foreach (var part in nested)
                {
                    decimal after = part switch
                    {
                        PaperPhrog phrog => phrog.ModifyVulnerableMultiplier(target, composite, props, dealer, card),
                        CrueltyPower cruelty => cruelty.ModifyVulnerableMultiplier(target, composite, props, dealer, card),
                        DebilitatePower debilitate => debilitate.ModifyVulnerableMultiplier(target, composite, props, dealer, card),
                        _ => composite
                    };
                    if (after != composite) parts.Add((part, after - composite));
                    composite = after;
                }
                if (composite == multiplier && parts[0].Delta >= 0)
                {
                    foreach (var part in parts)
                    {
                        int value = checked((int)(before * part.Delta));
                        if (value > 0) contribute(part.Model, value);
                    }
                    continue;
                }
            }
            int contribution = checked((int)(before * (multiplier - 1)));
            if (contribution > 0) contribute(model, contribution);
        }
    }
    internal static ResultKind Classify(CreatureDescriptor dealer, object dealerObject, bool card, CreatureDescriptor receiver, object receiverObject)
    {
        if (dealer.Osty && !receiver.Player) return ResultKind.OstyDealt;
        if (receiver.Osty) return ResultKind.OstyAbsorbed;
        if (receiver.Player) return ReferenceEquals(dealerObject, receiverObject) || (dealerObject == null && card) ? ResultKind.SelfDamage : ResultKind.Incoming;
        return ResultKind.Outgoing;
    }
    internal static List<DamageResult>.Enumerator ReportResultGroup(List<DamageResult> results)
    {
        try
        {
            var captured = Current;
            Current = captured with { Calculation = 0 };
            if (!CaptureRuntime.Valid(captured.Epoch)) return results.GetEnumerator();
            ulong token = captured.Calculation;
            if (results.Count > MaxResults)
            {
                AbortToken(token);
                CaptureRuntime.Fail("damage-result-cap");
                foreach (var result in results)
                    if (CaptureRuntime.Backend.DamageFallback(captured.Epoch.Sequence, Packet(captured, result)) != 1) CaptureRuntime.Fail("damage-fallback");
                return results.GetEnumerator();
            }
            var packets = new ResultPacket[results.Count];
            try
            {
                for (int i = 0; i < results.Count; i++) packets[i] = Packet(captured, results[i]);
            }
            catch { AbortToken(token); throw; }
            bool committed = false;
            try
            {
                if (token != 0 && packets.Length <= MaxResults)
                {
                    foreach (var packet in packets)
                        if (CaptureRuntime.Backend.DamageAppend(token, packet) != 1) throw new InvalidOperationException("Damage result append rejected");
                    committed = CaptureRuntime.Backend.DamageCommit(token) == 1;
                }
            }
            catch (Exception ex) { CaptureRuntime.Fail("damage-group", ex); }
            if (!committed)
            {
                AbortToken(token);
                foreach (var packet in packets)
                    if (CaptureRuntime.Backend.DamageFallback(captured.Epoch.Sequence, packet) != 1) CaptureRuntime.Fail("damage-fallback");
            }
        }
        catch (Exception ex) { CaptureRuntime.Fail("damage-report", ex); }
        return results.GetEnumerator();
    }
    private static ResultPacket Packet(DamageOperation captured, DamageResult result)
    {
        var dealer = captured.Dealer == null ? default : CaptureRuntime.Backend.DescribeCreature(captured.Dealer);
        var receiver = CaptureRuntime.Backend.DescribeCreature(result.Receiver);
        if (!ReferenceEquals(receiver.Combat, captured.Epoch.Combat)) throw new InvalidOperationException("Result belongs to another combat");
        int total = checked(result.UnblockedDamage + result.BlockedDamage);
        if (result.UnblockedDamage < 0 || result.BlockedDamage < 0) throw new InvalidOperationException("Negative damage result");
        int weak = 0;
        if (captured.Weak && receiver.Player && total > 0)
        {
            bool krane = receiver.Slot >= 0 && receiver.Slot < 4 && (captured.PaperKraneSlots & (1u << receiver.Slot)) != 0;
            decimal multiplier = krane ? 0.60m : 0.75m;
            if (captured.Debilitate) multiplier = 1m - (1m - multiplier) * 2m;
            multiplier = Math.Max(multiplier, 0.1m);
            weak = Math.Max(checked((int)Math.Round(total / multiplier - total)), 0);
        }
        return new(total, result.UnblockedDamage, result.BlockedDamage,
            Classify(dealer, captured.Dealer, captured.ExplicitCard, receiver, result.Receiver), receiver.Slot, weak);
    }
    private static void AbortToken(ulong token)
    {
        if (token == 0) return;
        try { CaptureRuntime.Backend.DamageAbort(token); }
        catch (Exception ex) { CaptureRuntime.Fail("damage-abort", ex); }
    }
    internal static void Abort()
    {
        var captured = Current;
        Current = captured with { Calculation = 0 };
        try { if (CaptureRuntime.Valid(captured.Epoch)) AbortToken(captured.Calculation); }
        catch (Exception ex) { CaptureRuntime.Fail("damage-abort", ex); }
    }
    internal static void SetResult(ref AsyncTaskMethodBuilder<IEnumerable<DamageResult>> builder, IEnumerable<DamageResult> result)
    { Abort(); builder.SetResult(result); }
    internal static void SetException(ref AsyncTaskMethodBuilder<IEnumerable<DamageResult>> builder, Exception error)
    { Abort(); builder.SetException(error); }
    internal static MethodInfo Canonical => AccessTools.DeclaredMethod(typeof(CreatureCmd), "Damage", new[] { typeof(PlayerChoiceContext), typeof(IEnumerable<Creature>), typeof(decimal), typeof(ValueProp), typeof(Creature), typeof(CardModel), typeof(CardPlay) })
        ?? throw new InvalidOperationException("Missing exact canonical Damage overload");
    internal static IEnumerable<CodeInstruction> Transpiler(IEnumerable<CodeInstruction> instructions)
    {
        var code = instructions.Select(c => new CodeInstruction(c)).ToList();
        const BindingFlags instance = BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly;
        var modify = HookMethod;
        var enumerate = typeof(List<DamageResult>).GetMethod("GetEnumerator", instance, null, Type.EmptyTypes, null);
        var complete = typeof(AsyncTaskMethodBuilder<IEnumerable<DamageResult>>).GetMethod("SetResult", instance, null, new[] { typeof(IEnumerable<DamageResult>) }, null);
        var fail = typeof(AsyncTaskMethodBuilder<IEnumerable<DamageResult>>).GetMethod("SetException", instance, null, new[] { typeof(Exception) }, null);
        if (enumerate?.ReturnType != typeof(List<DamageResult>.Enumerator) || complete?.ReturnType != typeof(void) || fail?.ReturnType != typeof(void))
            throw new InvalidOperationException("Canonical damage bridge signature changed");
        if (code.Count(c => c.Calls(modify)) != 1 || code.Count(c => c.Calls(enumerate)) != 2 || code.Count(c => c.Calls(complete)) != 1 || code.Count(c => c.Calls(fail)) != 1)
            throw new InvalidOperationException("Canonical damage IL pattern changed");
        bool reported = false;
        foreach (var instruction in code)
        {
            string bridge = instruction.Calls(modify) ? nameof(ModifyDamage) : instruction.Calls(complete) ? nameof(SetResult)
                : instruction.Calls(fail) ? nameof(SetException) : instruction.Calls(enumerate) && !reported ? nameof(ReportResultGroup) : null;
            if (bridge == null) continue;
            if (bridge == nameof(ReportResultGroup)) reported = true;
            instruction.opcode = OpCodes.Call;
            instruction.operand = AccessTools.Method(typeof(DamageCapture), bridge);
        }
        return code;
    }
    internal static void Patch(Harmony harmony, MethodInfo canonical)
    {
        canonical = FlowCapture.DeclaredMethod(canonical);
        CapturePatches.Patch(harmony, TemporalPowerCapture.Body(canonical), transpiler: new HarmonyMethod(typeof(DamageCapture), nameof(Transpiler)));
        CapturePatches.Patch(harmony, canonical, prefix: new HarmonyMethod(typeof(DamageCapture), nameof(Prefix)), finalizer: new HarmonyMethod(typeof(DamageCapture), nameof(Finalizer)));
    }
    internal static void Install(Harmony harmony, Action<string> report)
    {
        Patch(harmony, Canonical);
        report($"DAMAGE BRIDGES {TemporalPowerCapture.Body(Canonical).DeclaringType.FullName}: ModifyDamage=1 ReportResultGroup=1 unchanged GetEnumerator=1 SetResult=1 SetException=1");
    }
}
