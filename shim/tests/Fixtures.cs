using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Models.Cards;
using MegaCrit.Sts2.Core.Models.Potions;
using MegaCrit.Sts2.Core.Models.Relics;
using MegaCrit.Sts2.Core.Models.Orbs;
using MegaCrit.Sts2.Core.Localization.DynamicVars;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Hooks;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Runs;
using MegaCrit.Sts2.Core.Rooms;
using MegaCrit.Sts2.Core.ValueProps;

#pragma warning disable CA1822 // Harmony fixtures preserve the game's instance method shapes.
#pragma warning disable CA1861 // Expected arrays stay beside the assertions they describe.

namespace SpireProfiler;

internal sealed class FixtureContext : SynchronizationContext
{
    private readonly Queue<(SendOrPostCallback Callback, object State)> queue = new();
    public override void Post(SendOrPostCallback callback, object state) { lock (queue) queue.Enqueue((callback, state)); }
    internal void Complete(Task task)
    {
        var deadline = DateTime.UtcNow.AddSeconds(10);
        while (!task.IsCompleted)
        {
            (SendOrPostCallback Callback, object State) item = default;
            lock (queue) if (queue.Count != 0) item = queue.Dequeue();
            if (item.Callback != null) item.Callback(item.State);
            else if (DateTime.UtcNow > deadline) throw new TimeoutException("Fixture continuation did not return to its registered thread");
            else Thread.Yield();
        }
        task.GetAwaiter().GetResult();
    }
}
internal class ProbeModel
{
    internal string Name;
    internal ProducerRole Role;
    internal int Slot;
    internal bool Temporary;
    internal bool FailCapture;
    internal ProbeModel(string name, ProducerRole role = ProducerRole.Card, int slot = 0) { Name = name; Role = role; Slot = slot; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal SourceSnapshot Synchronous() => FlowCapture.Current.Source;
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal Task<int> ReturnTask(Task<int> result) => result;
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void Throw(Exception exception) { throw exception; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal async Task<SourceSnapshot> Suspended(Task pause) { await pause; return FlowCapture.Current.Source; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal SourceSnapshot Nested(ProbeModel other, Action<SourceSnapshot> inside)
    { inside(other.Synchronous()); return FlowCapture.Current.Source; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal Task<SourceSnapshot> DoDamage(Task pause) => Suspended(pause);
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal async Task OnPlayWrapper(Func<Task> body)
    { try { await body(); } finally { ProbeCommands.EndCardOrPotionEffect(); } }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal async Task OnUseWrapper(Func<Task> body)
    { try { await body(); } finally { ProbeCommands.EndCardOrPotionEffect(); } }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal SourceSnapshot Passive() => FlowCapture.Current.Source;
}
internal sealed class ProbePower : ProbeModel
{
    internal ProbeCreature Owner;
    internal int Amount;
    internal Exception NotificationException;
    internal ProbePower(string name) : base(name, ProducerRole.Power) { }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void SetAmount(int amount, bool silent = false)
    { Amount = Math.Max(amount, 0); if (NotificationException != null) throw NotificationException; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void BeforeApplied(object target, decimal amount, object applier, object cardSource)
    { ManagedFixtures.TemporaryObserved = FlowCapture.Current.Source; ManagedFixtures.DuringBeforeApplied?.Invoke(); }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void AfterPowerAmountChanged(object choice, ProbePower power, decimal amount, object applier, object cardSource)
    { ManagedFixtures.TemporaryObserved = FlowCapture.Current.Source; }
}
internal sealed class ProbeCreature
{
    internal readonly List<ProbePower> Powers = new();
    internal Exception RemovalException;
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void ApplyPowerInternal(ProbePower power)
    { Powers.Add(power); if (power.NotificationException != null) throw power.NotificationException; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void RemovePowerInternal(ProbePower power)
    { if (RemovalException != null) throw RemovalException; Powers.Remove(power); if (power.NotificationException != null) throw power.NotificationException; }
}
internal static class ProbeCommands
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static async Task Apply(object choice, ProbePower power, ProbeCreature target, decimal amount, object applier, object cardSource, bool silent = false)
    {
        if (ManagedFixtures.CommandPause != null) await ManagedFixtures.CommandPause;
        power.BeforeApplied(target, amount, applier, cardSource);
        power.Owner = target;
        power.SetAmount((int)amount);
        target.ApplyPowerInternal(power);
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static async Task<int> ModifyAmount(object choice, ProbePower power, decimal offset, object applier, object cardSource, bool silent = false)
    {
        if (ManagedFixtures.CommandPause != null) await ManagedFixtures.CommandPause;
        ManagedFixtures.HistoryAmounts.Add(power.Amount);
        power.SetAmount(power.Amount + (int)offset);
        power.AfterPowerAmountChanged(choice, power, offset, applier, cardSource);
        return power.Amount;
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static void EndCardOrPotionEffect() { }
}
internal sealed class RupturePowerFixture : ProbeModel
{
    internal readonly Dictionary<CardModel, int> Amounts = new();
    internal RupturePowerFixture() : base("TEMPORAL", ProducerRole.Power) { }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal Task BeforeCardPlayed(CardModel card, int amount) { Amounts.Add(card, amount); return Task.CompletedTask; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal async Task<SourceSnapshot> AfterCardPlayed(CardModel card, Task pause)
    { if (!Amounts.Remove(card, out var amount)) return SourceSnapshot.Unavailable; await pause; return amount > 0 ? FlowCapture.Current.Source : SourceSnapshot.Unavailable; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal void AfterDamageReceived(CardModel card, int amount) { Amounts[card] += amount; }
}
internal static class DamageFixture
{
    internal static Task Pause = Task.CompletedTask;
    internal static List<DamageResult> Results = new();
    internal static Dictionary<Creature, List<DamageResult>> ResultsByTarget;
    internal static Dictionary<Creature, (IEnumerable<AbstractModel> Modifiers, decimal Bonus)> ModifiersByTarget;
    internal static Exception Error;
    internal static int HookCalls, Enumerated, LateEnumerated;
    internal static bool PreviewDuringDamage;
    internal static Action DuringLive;
    internal static Func<Task> Nested;
    internal static Action AfterGroup;
    internal static IEnumerable<AbstractModel> Modifiers = Array.Empty<AbstractModel>();
    internal static decimal Bonus;
    internal static Task<IEnumerable<DamageResult>> OriginalTask;
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static async Task<IEnumerable<DamageResult>> Damage(PlayerChoiceContext choice, IEnumerable<Creature> targets, decimal amount, ValueProp props, Creature dealer, CardModel cardSource, CardPlay cardPlay)
    {
        var byTarget = ResultsByTarget;
        var aggregate = byTarget == null ? Results : new List<DamageResult>();
        var pause = Pause;
        var nested = Nested;
        foreach (var target in targets)
        {
            var results = byTarget == null ? aggregate : byTarget[target];
            Hook.ModifyDamage(null, (ICombatState)CaptureRuntime.Epoch.Combat, target, dealer, amount, props, cardSource, cardPlay, ModifyDamageHookType.All, CardPreviewMode.None, out var modifiers);
            DuringLive?.Invoke();
            if (nested != null) await nested();
            await pause;
            if (PreviewDuringDamage) Preview(target, dealer, cardSource);
            if (Error != null) throw Error;
            foreach (var result in results) Enumerated += result.TotalDamage;
            AfterGroup?.Invoke();
            if (byTarget != null) aggregate.AddRange(results);
        }
        foreach (var result in aggregate) LateEnumerated += result.TotalDamage;
        return aggregate;
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static decimal Preview(Creature target, Creature dealer, CardModel card)
        => Hook.ModifyDamage(null, (ICombatState)CaptureRuntime.Epoch.Combat, target, dealer, 10, ValueProp.Move, card, null, ModifyDamageHookType.All, CardPreviewMode.None, out _);
    internal static bool PreviewHookPrefix(decimal damage, ref decimal __result, ref IEnumerable<AbstractModel> modifiers)
    { HookCalls++; modifiers = Array.Empty<AbstractModel>(); __result = damage; return false; }
    internal static void ObserveTask(Task<IEnumerable<DamageResult>> __result) { OriginalTask = __result; }
    internal static decimal Modify(IRunState runState, ICombatState combatState, Creature target, Creature dealer, decimal damage, ValueProp props, CardModel cardSource, CardPlay cardPlay,
        ModifyDamageHookType hookType, CardPreviewMode previewMode, out IEnumerable<AbstractModel> modifiers)
    {
        HookCalls++;
        var values = ModifiersByTarget != null ? ModifiersByTarget[target] : (Modifiers, Bonus);
        modifiers = values.Item1;
        return damage + values.Item2;
    }
}
internal static class GameProducerFixture
{
    internal static SleightOfFleshPower Listener;
    internal static IReadOnlyList<Creature> Enemies = Array.Empty<Creature>();
    internal static Task WaitTask = Task.CompletedTask;
    internal static int Blocked;
    internal static Action<PowerModel, Creature, SourceSnapshot> Applied;
    internal static Action<PowerModel> Decremented;
    internal static Action<CardModel> Created;
    internal static readonly List<CardModel> Generated = new();
    internal static readonly List<(PowerModel Power, decimal Amount, CardModel Card, ProducerFrame Producer)> Applications = new();
    internal static readonly List<(decimal Amount, ValueProp Props, Creature Dealer, ProducerFrame Producer, PlayFrame Play)> Emissions = new();

    // Only command, targeting and presentation boundaries change; pinned producer
    // conditions, amounts, order, awaits and actual production scope patches run.
    internal static IEnumerable<CodeInstruction> Boundaries(IEnumerable<CodeInstruction> instructions)
    {
        foreach (var instruction in instructions)
        {
            if (instruction.operand is MethodInfo call)
            {
                MethodInfo replacement = null;
                if (call.DeclaringType == typeof(PowerCmd) && call.Name == "Apply" && call.IsGenericMethod)
                {
                    string name = call.GetParameters()[1].ParameterType == typeof(Creature) ? nameof(Apply) : nameof(ApplyMany);
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), name).MakeGenericMethod(call.GetGenericArguments());
                }
                else if (call.DeclaringType == typeof(PowerCmd) && call.Name == "Decrement")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(Decrement));
                else if (call.DeclaringType == typeof(ICombatState) && call.Name == "CreateCard" && call.IsGenericMethod)
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(CreateCard)).MakeGenericMethod(call.GetGenericArguments());
                else if (call.DeclaringType == typeof(CardPileCmd) && call.Name == "AddGeneratedCardToCombat")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(AddGenerated));
                else if (call.DeclaringType == typeof(CreatureCmd))
                    replacement = call.Name switch
                    {
                        "TriggerAnim" => AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(Animation)),
                        "GainBlock" => call.GetParameters()[1].ParameterType == typeof(decimal)
                            ? AccessTools.DeclaredMethod(typeof(CommandFixture), nameof(CommandFixture.GainBlock)) : AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(Block)),
                        "Damage" => AccessTools.DeclaredMethod(typeof(GameProducerFixture), call.GetParameters()[1].ParameterType == typeof(IEnumerable<Creature>)
                            ? nameof(EmitManyDamage) : call.GetParameters().Length == 6 ? nameof(EmitCardDamage) : nameof(EmitDamage)),
                        _ => null
                    };
                else if (call.DeclaringType == typeof(Cmd) && call.Name == "Wait")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(Wait));
                else if (call.DeclaringType == typeof(Cmd) && call.Name == "CustomScaledWait")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(ScaledWait));
                else if (call.DeclaringType == typeof(ICombatState) && call.Name == "get_HittableEnemies")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(Targets));
                else if (call.DeclaringType == typeof(CardModel) && call.Name == "get_CombatState")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(CardCombat));
                else if (call.DeclaringType == typeof(MegaCrit.Sts2.Core.Nodes.Rooms.NCombatRoom) && call.Name == "get_Instance")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(NoCombatRoom));
                else if (call.DeclaringType == typeof(MegaCrit.Sts2.Core.Nodes.Vfx.NPoisonImpactVfx) && call.Name == "Create")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(NoPoisonVfx));
                else if (call.DeclaringType == typeof(OrbModel) && call.Name == "PlayPassiveSfx")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(NoPassiveSound));
                else if (call.DeclaringType == typeof(OrbModel) && call.Name == "ModifyOrbValue")
                    replacement = AccessTools.DeclaredMethod(typeof(GameProducerFixture), nameof(UnmodifiedOrbValue));
                if (replacement != null)
                {
                    var expected = call.GetParameters().Select(parameter => parameter.ParameterType);
                    if (!call.IsStatic) expected = expected.Prepend(call.DeclaringType);
                    if (replacement.ReturnType != call.ReturnType || !replacement.GetParameters().Select(parameter => parameter.ParameterType).SequenceEqual(expected))
                        throw new InvalidOperationException("Pinned producer boundary signature drift: " + call);
                    instruction.opcode = OpCodes.Call;
                    instruction.operand = replacement;
                }
            }
            yield return instruction;
        }
    }
    internal static Task Animation(Creature creature, string name, float delay) => Task.CompletedTask;
    internal static Task<decimal> Block(Creature creature, BlockVar amount, CardPlay play, bool fast)
        => Task.FromResult(amount.BaseValue);
    internal static Task Wait(float seconds, bool ignoreCombatEnd) => WaitTask;
    internal static Task ScaledWait(float fast, float standard, bool ignoreCombatEnd, CancellationToken cancellation) => WaitTask;
    internal static MegaCrit.Sts2.Core.Nodes.Rooms.NCombatRoom NoCombatRoom() => null;
    internal static MegaCrit.Sts2.Core.Nodes.Vfx.NPoisonImpactVfx NoPoisonVfx(Creature creature) => null;
    internal static void NoPassiveSound(OrbModel orb) { }
    internal static decimal UnmodifiedOrbValue(OrbModel orb, decimal amount) => amount;
    internal static IReadOnlyList<Creature> Targets(ICombatState combat) => Enemies;
    internal static ICombatState CardCombat(CardModel card) => card.Owner.Creature.CombatState;
    internal static async Task<T> Apply<T>(PlayerChoiceContext choice, Creature target, decimal amount, Creature applier, CardModel cardSource, bool silent) where T : PowerModel
    {
        var power = (T)RuntimeHelpers.GetUninitializedObject(typeof(T));
        AccessTools.Field(typeof(AbstractModel), "<IsMutable>k__BackingField").SetValue(power, true);
        AccessTools.Field(typeof(AbstractModel), "<Id>k__BackingField").SetValue(power, new ModelId("TEST", typeof(T).Name));
        AccessTools.Field(typeof(PowerModel), "_owner").SetValue(power, target);
        AccessTools.Field(typeof(PowerModel), "_amount").SetValue(power, checked((int)amount));
        Applications.Add((power, amount, cardSource, FlowCapture.Current));
        Applied?.Invoke(power, target, FlowCapture.Supplied(cardSource, CaptureRuntime.Epoch));
        if (Listener != null) await Listener.AfterPowerAmountChanged(choice, power, amount, applier, cardSource);
        return power;
    }
    internal static async Task<IReadOnlyList<T>> ApplyMany<T>(PlayerChoiceContext choice, IEnumerable<Creature> targets, decimal amount, Creature applier, CardModel cardSource, bool silent) where T : PowerModel
    {
        var powers = new List<T>();
        foreach (var target in targets) powers.Add(await Apply<T>(choice, target, amount, applier, cardSource, silent));
        return powers;
    }
    internal static Task Decrement(PowerModel power)
    {
        AccessTools.Field(typeof(PowerModel), "_amount").SetValue(power, power.Amount - 1);
        Decremented?.Invoke(power);
        return Task.CompletedTask;
    }
    internal static T CreateCard<T>(ICombatState scope, Player owner) where T : CardModel
    {
        var card = ManagedFixtures.Mutable<T>(typeof(T).Name);
        card.Owner = owner;
        Created?.Invoke(card);
        return card;
    }
    internal static async Task<CardPileAddResult> AddGenerated(CardModel card, PileType pile, Player creator, CardPilePosition position)
    {
        Generated.Add(card);
        ProvenanceCapture.Generated(card);
        await WaitTask;
        return new CardPileAddResult { success = true, cardAdded = card, targetPile = pile };
    }
    internal static Task<IEnumerable<DamageResult>> EmitDamage(PlayerChoiceContext choice, Creature target, decimal amount, ValueProp props, Creature dealer)
    {
        Emissions.Add((amount, props, dealer, FlowCapture.Current, PlayCapture.Current));
        int total = checked((int)amount);
        DamageFixture.Results = new() { new(target, props) { UnblockedDamage = total - Blocked, BlockedDamage = Blocked } };
        return DamageFixture.Damage(choice, new[] { target }, amount, props, dealer, null, null);
    }
    internal static Task<IEnumerable<DamageResult>> EmitCardDamage(PlayerChoiceContext choice, Creature target, decimal amount, ValueProp props, CardModel card, CardPlay play)
    {
        Emissions.Add((amount, props, card?.Owner.Creature, FlowCapture.Current, PlayCapture.Current));
        DamageFixture.Results = new() { new(target, props) { UnblockedDamage = checked((int)amount) } };
        return DamageFixture.Damage(choice, new[] { target }, amount, props, card?.Owner.Creature, card, play);
    }
    internal static Task<IEnumerable<DamageResult>> EmitManyDamage(PlayerChoiceContext choice, IEnumerable<Creature> targets, decimal amount, ValueProp props, Creature dealer)
    {
        Emissions.Add((amount, props, dealer, FlowCapture.Current, PlayCapture.Current));
        var captured = targets.ToArray();
        DamageFixture.ResultsByTarget = captured.ToDictionary(target => target, target => new List<DamageResult> { new(target, props) { UnblockedDamage = checked((int)amount) } });
        return DamageFixture.Damage(choice, captured, amount, props, dealer, null, null);
    }
}
internal static class CommandFixture
{
    internal static Task Pause = Task.CompletedTask;
    internal static Action During;
    internal static int Calls;
    internal static SourceSnapshot Frozen;
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static async Task<decimal> GainBlock(Creature creature, decimal amount, ValueProp props, CardPlay cardPlay, bool fast = false)
    {
        Calls++;
        await Pause;
        Frozen = CommandCapture.Current.Source;
        During?.Invoke();
        CommandCapture.BlockGained(CaptureRuntime.Epoch.Combat, creature, (int)amount);
        return amount;
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static async Task Forge(decimal amount, Player player, AbstractModel source)
    { Calls++; await Pause; Frozen = FlowCapture.Current.Source; During?.Invoke(); }
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static async Task Summon(PlayerChoiceContext choice, Player summoner, decimal amount, AbstractModel source)
    { Calls++; await Pause; Frozen = FlowCapture.Current.Source; During?.Invoke(); }
}
internal static class DoomFixture
{
    internal static Task OriginalTask = Task.CompletedTask;
    internal static Action During;
    internal static Exception Error;
    internal static int Calls;
    [MethodImpl(MethodImplOptions.NoInlining)]
    internal static Task DoomKill(IReadOnlyList<Creature> creatures)
    { Calls++; During?.Invoke(); if (Error != null) throw Error; return OriginalTask; }
}
internal sealed class FakeBackend : AttributionBackend
{
    internal object Combat;
    internal override object CurrentCombat => Combat;
    internal readonly Dictionary<object, ModelDescriptor> Descriptors = new(ReferenceEqualityComparer.Instance);
    internal readonly Dictionary<object, CreatureDescriptor> Creatures = new(ReferenceEqualityComparer.Instance);
    internal readonly Dictionary<ulong, SourceSnapshot> Sources = new();
    internal readonly Dictionary<string, SourceSnapshot> NamedSources = new();
    private readonly Dictionary<ulong, List<SourceShare>> leases = new();
    private readonly Dictionary<ulong, (SourceSnapshot Source, List<ResultPacket> Packets)> calculations = new();
    internal readonly Dictionary<ulong, (ulong Execution, int Slot, SourceSnapshot Source, bool Triggered)> Plays = new();
    internal readonly List<(string Kind, ulong Identity, int Before, int After, SourceSnapshot Source)> PowerEvents = new();
    internal readonly List<(SourceSnapshot Source, ProducerRole Role, DamageSegment Segment)> Begun = new();
    internal readonly List<(ulong Calculation, SourceSnapshot Source, int Amount)> DamageModifiers = new();
    internal readonly List<(ulong Calculation, SourceSnapshot Source, ResultPacket[] Packets)> DamageGroups = new();
    internal readonly List<ResultPacket> Committed = new(), Fallback = new();
    internal readonly List<GenerationState> PlayGenerations = new();
    internal readonly List<(string Kind, int Amount, int Slot, SourceSnapshot Source)> CommandEvents = new();
    internal readonly List<(ulong Batch, ulong Creature, ulong Power, int Hp)> DoomTargets = new();
    internal readonly List<ulong> DoomCommitted = new();
    internal readonly HashSet<ulong> DoomBatches = new();
    internal readonly List<string> Calls = new();
    internal int Released, Started, Finished, ExecutionEnds;
    internal string Failure;
    internal int FailureCount = 1;
    internal int FailObservationAfter = -1;
    private ulong serial;
    internal int OpenLeases => leases.Count;
    internal int OpenCalculations => calculations.Count;
    private bool Reject(string category)
    {
        Calls.Add(category);
        if (Failure != category || FailureCount == 0) return false;
        FailureCount--;
        return true;
    }
    internal override ModelDescriptor Describe(object model)
    {
        if (Descriptors.TryGetValue(model, out var recorded)) return recorded;
        if (model is ProbeModel probe)
        {
            if (probe.FailCapture) throw new InvalidOperationException("fixture descriptor failure");
            CaptureKind kind = probe.Role switch { ProducerRole.Card => CaptureKind.CardInstance, ProducerRole.Power => CaptureKind.PowerInstance, ProducerRole.Orb => CaptureKind.OrbInstance, ProducerRole.Relic or ProducerRole.Potion => CaptureKind.DirectModel, _ => CaptureKind.Unknown };
            return new(kind, probe.Role, probe.Name, probe.Role == ProducerRole.Card ? 0 : probe.Role == ProducerRole.Relic ? 1 : probe.Role == ProducerRole.Potion ? 3 : 2, probe.Slot);
        }
        return Descriptors.TryGetValue(model, out var descriptor) ? descriptor : new(CaptureKind.Unknown, ProducerRole.Unknown, "", 5, 4);
    }
    internal override CreatureDescriptor DescribeCreature(object creature) => Creatures[creature];
    internal override bool TemporaryPower(object power) => power is ProbeModel probe && probe.Temporary;
    internal override PowerObservation ObservePower(object power, object owner = null)
    {
        if (FailObservationAfter == 0)
        {
            FailObservationAfter = -1;
            throw new InvalidOperationException("fixture observation failure");
        }
        if (FailObservationAfter > 0) FailObservationAfter--;
        if (power is PowerModel) return new NativeAttributionBackend().ObservePower(power, owner);
        var p = (ProbePower)power;
        var c = owner as ProbeCreature ?? p.Owner;
        return new(p, c, p.Name, 0, p.Slot, p.Amount, c?.Powers.Contains(p) == true);
    }
    internal override ulong Capture(ulong epoch, CaptureKind kind, ulong instance, string id, int sourceKind, int slot, GenerationState generation)
    {
        if (Reject("Capture")) throw new InvalidOperationException("fixture capture failure");
        SourceSnapshot source = Sources.GetValueOrDefault(instance) ?? NamedSources.GetValueOrDefault(id) ?? SourceSnapshot.Unknown(epoch);
        if (generation is GenerationState.GeneratedUnavailable or GenerationState.Unclassified && kind == CaptureKind.CardInstance) source = SourceSnapshot.Unknown(epoch);
        ulong token = ++serial;
        leases[token] = Enumerable.Range(0, source.Count).Select(i => source[i]).ToList();
        return token;
    }
    internal override int SourceCount(ulong transfer) => Reject("SourceCount") ? -1 : leases[transfer].Count;
    internal override ulong SourceDestination(ulong transfer, int index) => Reject("SourceDestination") ? 0 : leases[transfer][index].Destination;
    internal override ulong SourceWeight(ulong transfer, int index) => Reject("SourceWeight") ? 0 : leases[transfer][index].Weight;
    internal override ulong TransferBegin(ulong epoch) { if (Reject("TransferBegin")) return 0; ulong token = ++serial; leases[token] = new(); return token; }
    internal override int TransferAdd(ulong transfer, ulong destination, ulong weight) { if (Reject("TransferAdd")) return 0; leases[transfer].Add(new(destination, weight)); return 1; }
    internal override int TransferSeal(ulong transfer) => Reject("TransferSeal") ? 0 : 1;
    internal override int TransferRelease(ulong transfer) { leases.Remove(transfer); Released++; Calls.Add("Release"); return 1; }
    private SourceSnapshot Read(ulong transfer) => transfer == 0 ? SourceSnapshot.Unknown(CaptureRuntime.Epoch.Sequence) : SourceSnapshot.Create(CaptureRuntime.Epoch.Sequence, leases[transfer]);
    internal override int PowerAttached(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, ulong source)
    {
        if (Reject("PowerAttached")) return 0;
        Sources[identity] = Read(source);
        PowerEvents.Add(("attach", identity, 0, observed.Amount, Read(source)));
        return 1;
    }
    internal override int PowerChanged(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, int before, ulong source)
    {
        if (Reject("PowerChanged")) return 0;
        PowerEvents.Add(("change", identity, before, observed.Amount, Read(source)));
        Sources[identity] = observed.Amount == before ? Sources.GetValueOrDefault(identity) ?? SourceSnapshot.Unknown(epoch.Sequence) : Read(source);
        return 1;
    }
    internal override int PowerRemoved(ulong epoch, ulong identity) { if (Reject("PowerRemoved")) return 0; Sources.Remove(identity); return 1; }
    internal override int PowerInvalidate(ulong epoch, ulong identity)
    {
        if (Reject("PowerInvalidate")) return 0;
        Sources[identity] = SourceSnapshot.Unknown(epoch);
        return 1;
    }
    internal override int CardGenerated(ulong epoch, ulong identity, ulong source, ProducerRole role)
    { if (Reject("CardGenerated")) return 0; if (identity != 0) Sources[identity] = Read(source); return 1; }
    internal override ulong PlayStarted(ulong epoch, ulong execution, ulong card, string id, int slot, int index, int count, GenerationState generation, ulong source)
    {
        Started++;
        PlayGenerations.Add(generation);
        if (execution == 0 || Reject("PlayStarted")) return 0;
        ulong token = ++serial;
        Plays[token] = (execution, slot, Read(source), false);
        return token;
    }
    internal override int PlayFinished(ulong play) { Plays.Remove(play); Finished++; return 1; }
    internal override int ExecutionEnded(ulong epoch, ulong execution)
    { foreach (var key in Plays.Where(p => p.Value.Execution == execution).Select(p => p.Key).ToArray()) Plays.Remove(key); ExecutionEnds++; return 1; }
    internal override int OrbBegin(ulong epoch, ulong orb, ulong play, int ownerSlot)
    {
        if (play == 0) return 1;
        if (!Plays.TryGetValue(play, out var frame) || frame.Slot != ownerSlot) return 0;
        Plays[play] = (frame.Execution, frame.Slot, frame.Source, true);
        return frame.Triggered ? 2 : 1;
    }
    internal override ulong DamageBegin(ulong epoch, ulong source, ProducerRole role, DamageSegment segment, ulong target)
    {
        if (Reject("DamageBegin")) return 0;
        ulong token = ++serial;
        calculations[token] = (Read(source), new());
        Begun.Add((Read(source), role, segment));
        return token;
    }
    internal override int DamageModifier(ulong calculation, ulong source, int amount)
    { if (Reject("DamageModifier")) return 0; DamageModifiers.Add((calculation, Read(source), amount)); return 1; }
    internal override int DamageEnemyHit(ulong calculation, ulong dealer, int baseDamage, int strength) => Reject("DamageEnemyHit") ? 0 : 1;
    internal override int DamageWeak(ulong calculation, ulong source) => Reject("DamageWeak") ? 0 : 1;
    internal override int DamageAppend(ulong calculation, ResultPacket packet)
    { if (Reject("DamageAppend")) return 0; calculations[calculation].Packets.Add(packet); return 1; }
    internal override int DamageCommit(ulong calculation)
    {
        if (Reject("DamageCommit")) return 0;
        DamageGroups.Add((calculation, calculations[calculation].Source, calculations[calculation].Packets.ToArray()));
        Committed.AddRange(calculations[calculation].Packets);
        calculations.Remove(calculation);
        return 1;
    }
    internal override int DamageAbort(ulong calculation) { Calls.Add("DamageAbort"); calculations.Remove(calculation); return 1; }
    internal override int DamageFallback(ulong epoch, ResultPacket packet) { Calls.Add("DamageFallback"); Fallback.Add(packet); return 1; }
    internal override int OrbChanneled(ulong epoch, ulong orb, ulong source)
    { if (Reject("OrbChanneled")) return 0; Sources[orb] = Read(source); return 1; }
    internal override int BlockGained(ulong epoch, int amount, ulong source, int slot)
    { CommandEvents.Add(("block", amount, slot, Read(source))); return 1; }
    internal override int BlockModifier(ulong epoch, ulong source, int amount, int slot)
    { CommandEvents.Add(("block-modifier", amount, slot, Read(source))); return 1; }
    internal override int Forge(ulong epoch, ulong source, int amount)
    { CommandEvents.Add(("forge", amount, 4, Read(source))); return 1; }
    internal override int OstySummoned(ulong epoch, ulong source, int hp, int slot)
    { CommandEvents.Add(("summon", hp, slot, Read(source))); return 1; }
    internal override int OstyKilled(ulong epoch, int slot, ulong play)
    { Calls.Add("OstyKilled:" + play); return 1; }
    internal override int BuffMitigation(ulong epoch, ulong source, int prevented)
    { CommandEvents.Add(("buff", prevented, 4, Read(source))); return 1; }
    internal override ulong DoomBegin(ulong epoch)
    { if (Reject("DoomBegin")) return 0; ulong token = ++serial; DoomBatches.Add(token); return token; }
    internal override int DoomTarget(ulong batch, ulong creature, ulong power, int hp)
    { if (Reject("DoomTarget")) return 0; DoomTargets.Add((batch, creature, power, hp)); return 1; }
    internal override int DoomComplete(ulong batch)
    { if (Reject("DoomComplete")) return 0; DoomCommitted.Add(batch); DoomBatches.Remove(batch); return 1; }
    internal override int DoomAbort(ulong batch) { Calls.Add("DoomAbort"); DoomBatches.Remove(batch); return 1; }
    internal override int TurnStarted(ulong epoch) { Calls.Add("TurnStarted"); return 1; }
    internal override int BlockCleared(ulong epoch, int slot) { Calls.Add("BlockCleared:" + slot); return 1; }
    internal override int PlayerDied(ulong epoch, int slot) { Calls.Add("PlayerDied:" + slot); return 1; }
    internal override int PotionUsed(ulong epoch) { Calls.Add("PotionUsed"); return 1; }
    internal override ulong CombatStarted(string encounter, string type) { Calls.Add("CombatStarted"); return CaptureRuntime.Epoch.Sequence + 1; }
    internal override int CombatEnded(ulong epoch) { Calls.Add("CombatEnded"); return 1; }
    internal override void Diagnostic(string category, Exception error) { Calls.Add("diagnostic:" + category); }
}

internal static class ManagedFixtures
{
    private static FakeBackend backend;
    private static FixtureContext context;
    private static ulong epoch;
    private static int assertions, cases;
    internal static SourceSnapshot TemporaryObserved;
    internal static Action DuringBeforeApplied;
    private static bool skipPrefix;
    internal static bool EarlierPrefix() => !skipPrefix;
    internal static Task CommandPause;
    internal static readonly List<int> HistoryAmounts = new();
    private static SourceSnapshot A, B;
    private static Harmony harmony;
    internal static void Run(string generatedDirectory)
    {
        context = new FixtureContext();
        SynchronizationContext.SetSynchronizationContext(context);
        var nativeDelegates = typeof(ProfilerNative).GetNestedTypes(BindingFlags.Public | BindingFlags.NonPublic)
            .Where(type => typeof(MulticastDelegate).IsAssignableFrom(type)).ToArray();
        Check(nativeDelegates.Length == 52, "Complete compiled native delegate inventory");
        int utf8Parameters = 0;
        foreach (var type in nativeDelegates)
        {
            var convention = type.GetCustomAttributes(typeof(System.Runtime.InteropServices.UnmanagedFunctionPointerAttribute), false)
                .Cast<System.Runtime.InteropServices.UnmanagedFunctionPointerAttribute>().ToArray();
            Check(convention.Length == 1 && convention[0].CallingConvention == System.Runtime.InteropServices.CallingConvention.Cdecl,
                "Compiled framework Cdecl attribute: " + type.Name);
            var invoke = type.GetMethod("Invoke", BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly);
            Check(invoke != null && !invoke.ReturnParameter.IsDefined(typeof(System.Runtime.InteropServices.MarshalAsAttribute), false),
                "No compiled return marshalling override: " + type.Name);
            foreach (var parameter in invoke.GetParameters())
            {
                var marshalling = parameter.GetCustomAttribute<System.Runtime.InteropServices.MarshalAsAttribute>();
                if (parameter.ParameterType == typeof(string))
                {
                    Check(marshalling?.Value == System.Runtime.InteropServices.UnmanagedType.LPUTF8Str,
                        "Compiled UTF-8 string marshalling: " + type.Name + "." + parameter.Name);
                    utf8Parameters++;
                }
                else Check(marshalling == null, "No scalar parameter marshalling override: " + type.Name + "." + parameter.Name);
            }
        }
        Console.WriteLine($"DELEGATE METADATA Cdecl={nativeDelegates.Length} UTF8-string-parameters={utf8Parameters} return-marshalling=none");
        harmony = new Harmony("spire-profiler.capture-proof");
        using (var inventory = new StreamWriter(Path.Combine(generatedDirectory, "installed-capture.log")))
        {
            var targets = FlowCapture.ProducerTargets(typeof(AbstractModel).Assembly);
            Check(targets.Count == 1726, "Exact producer definition inventory");
            Check(targets.All(target => target.ReflectedType == target.DeclaringType), "Every producer target is reflected from its actual declaring type");
            var expectedRoots = new Dictionary<string, int> { ["AbstractModel"] = 934, ["CardModel"] = 564, ["PotionModel"] = 65, ["PowerModel"] = 57, ["RelicModel"] = 106 };
            foreach (var group in targets.GroupBy(m => m.GetBaseDefinition().DeclaringType.Name)) Check(expectedRoots[group.Key] == group.Count(), "Exact producer root count");
            FlowCapture.Install(harmony, line => { inventory.WriteLine(line); if (!line.StartsWith("PRODUCER ", StringComparison.Ordinal)) Console.WriteLine(line); });
        }
        foreach (var name in new[] { "Synchronous", "ReturnTask", "Throw", "Suspended", "Nested", "DoDamage" }) FlowCapture.PatchProducer(harmony, AccessTools.Method(typeof(ProbeModel), name));
        foreach (var name in new[] { "BeforeApplied", "AfterPowerAmountChanged" }) FlowCapture.PatchProducer(harmony, AccessTools.Method(typeof(ProbePower), name));
        foreach (var name in new[] { "OnPlayWrapper", "OnUseWrapper" }) PlayCapture.PatchWrapper(harmony, AccessTools.Method(typeof(ProbeModel), name));
        harmony.Patch(AccessTools.Method(typeof(ProbeModel), "Passive"), prefix: new HarmonyMethod(typeof(PlayCapture), nameof(PlayCapture.OrbPrefix)), finalizer: new HarmonyMethod(typeof(PlayCapture), nameof(PlayCapture.OrbFinalizer)));
        harmony.Patch(AccessTools.Method(typeof(ProbeCommands), "EndCardOrPotionEffect"), prefix: new HarmonyMethod(typeof(PlayCapture), nameof(PlayCapture.EndEffectPrefix)));
        foreach (var name in new[] { "Apply", "ModifyAmount" })
            harmony.Patch(AccessTools.Method(typeof(ProbeCommands), name), prefix: new HarmonyMethod(typeof(ProvenanceCapture), nameof(ProvenanceCapture.CommandPrefix)), finalizer: new HarmonyMethod(typeof(ProvenanceCapture), nameof(ProvenanceCapture.CommandFinalizer)));
        ProvenanceCapture.PatchMutation(harmony, AccessTools.Method(typeof(ProbePower), "SetAmount"));
        foreach (var name in new[] { "ApplyPowerInternal", "RemovePowerInternal" }) ProvenanceCapture.PatchMutation(harmony, AccessTools.Method(typeof(ProbeCreature), name));
        foreach (var name in new[] { "BeforeCardPlayed", "AfterCardPlayed", "AfterDamageReceived" })
        {
            var method = AccessTools.Method(typeof(RupturePowerFixture), name);
            FlowCapture.PatchProducer(harmony, method);
            harmony.Patch(TemporalPowerCapture.Body(method), transpiler: new HarmonyMethod(typeof(TemporalPowerCapture), nameof(TemporalPowerCapture.Transpiler)));
        }
        var installedDamage = PatchProcessor.GetCurrentInstructions(TemporalPowerCapture.Body(DamageCapture.Canonical)).ToList();
        foreach (var bridge in new[] { nameof(DamageCapture.ModifyDamage), nameof(DamageCapture.ReportResultGroup), nameof(DamageCapture.SetResult), nameof(DamageCapture.SetException) })
            Check(installedDamage.Count(c => c.Calls(AccessTools.Method(typeof(DamageCapture), bridge))) == 1, "Installed exact damage bridge: " + bridge);
        Check(installedDamage.Count(c => c.Calls(typeof(List<DamageResult>).GetMethod("GetEnumerator", BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly, null, Type.EmptyTypes, null))) == 1, "Installed aggregate enumerator remains original");
        foreach (var type in new[] { typeof(MegaCrit.Sts2.Core.Models.Powers.StormPower), typeof(MegaCrit.Sts2.Core.Models.Powers.StranglePower), typeof(MegaCrit.Sts2.Core.Models.Powers.SerpentFormPower), typeof(MegaCrit.Sts2.Core.Models.Powers.GravityPower), typeof(MegaCrit.Sts2.Core.Models.Powers.AfterimagePower), typeof(MegaCrit.Sts2.Core.Models.Powers.OblivionPower), typeof(MegaCrit.Sts2.Core.Models.Powers.RupturePower) })
            foreach (var item in new[] { ("BeforeCardPlayed", "Add"), ("AfterCardPlayed", "Remove") })
            {
                var installed = PatchProcessor.GetCurrentInstructions(TemporalPowerCapture.Body(AccessTools.DeclaredMethod(type, item.Item1)));
                Check(installed.Count(c => c.Calls(AccessTools.Method(typeof(TemporalPowerCapture), item.Item2))) == 1, "Installed temporal bridge: " + type.Name + "." + item.Item1);
            }
        DamageCapture.Patch(harmony, AccessTools.Method(typeof(DamageFixture), "Damage"));
        harmony.Patch(AccessTools.Method(typeof(DamageFixture), "Damage"), postfix: new HarmonyMethod(typeof(DamageFixture), nameof(DamageFixture.ObserveTask)));
        harmony.Patch(DamageCapture.HookMethod, prefix: new HarmonyMethod(typeof(DamageFixture), nameof(DamageFixture.PreviewHookPrefix)));
        DamageCapture.OriginalModifyDamage = DamageFixture.Modify;
        DamageCapture.Inspect = (_, _, _) => default;
        foreach (var item in new (Type Type, string Method, int Boundaries)[]
        {
            (typeof(Defy), "OnPlay", 3), (typeof(SleightOfFlesh), "OnPlay", 2),
            (typeof(Deathbringer), "OnPlay", 7), (typeof(WeakPotion), "OnUse", 2),
            (typeof(RedMask), "BeforeSideTurnStart", 2), (typeof(CountdownPower), "AfterSideTurnStart", 2),
            (typeof(SleightOfFleshPower), "AfterPowerAmountChanged", 1),
            (typeof(CacophonyPower), "AfterCardDrawn", 3), (typeof(FlameBarrierPower), "AfterDamageReceived", 1),
            (typeof(RollingBoulderPower), "DoDamage", 1), (typeof(PoisonPower), "Trigger", 3), (typeof(Outbreak), "OnPlay", 8),
            (typeof(SentryModePower), "BeforeHandDraw", 2), (typeof(FrostOrb), "Passive", 3), (typeof(FrostOrb), "get_PassiveVal", 1)
        })
        {
            var body = TemporalPowerCapture.Body(AccessTools.DeclaredMethod(item.Type, item.Method));
            harmony.Patch(body, transpiler: new HarmonyMethod(typeof(GameProducerFixture), nameof(GameProducerFixture.Boundaries)));
            int count = PatchProcessor.GetCurrentInstructions(body).Count(instruction => instruction.operand is MethodInfo call
                && (call.DeclaringType == typeof(GameProducerFixture) || call.DeclaringType == typeof(CommandFixture)));
            Check(count == item.Boundaries, $"Exact fixture boundary substitutions: {item.Type.Name}.{item.Method}, expected {item.Boundaries}, got {count}");
            Console.WriteLine($"ACTUAL PRODUCER {item.Type.Name}.{item.Method}; injected command/target boundaries={count}");
        }
        foreach (var name in new[] { "GainBlock", "Forge", "Summon" }) CommandCapture.PatchCommand(harmony, AccessTools.DeclaredMethod(typeof(CommandFixture), name));
        CapturePatches.Patch(harmony, AccessTools.DeclaredMethod(typeof(DoomFixture), "DoomKill"), prefix: new HarmonyMethod(typeof(DoomCapture), nameof(DoomCapture.Prefix)),
            postfix: new HarmonyMethod(typeof(DoomCapture), nameof(DoomCapture.Postfix)), finalizer: new HarmonyMethod(typeof(DoomCapture), nameof(DoomCapture.Finalizer)));
        foreach (var target in new[]
        {
            AccessTools.Method(typeof(ProbeModel), "Synchronous"), AccessTools.Method(typeof(ProbeModel), "Passive"),
            AccessTools.Method(typeof(ProbeModel), "OnPlayWrapper"), AccessTools.Method(typeof(ProbeModel), "OnUseWrapper"),
            AccessTools.Method(typeof(ProbeCommands), "Apply"), AccessTools.Method(typeof(ProbeCommands), "ModifyAmount"),
            AccessTools.Method(typeof(ProbePower), "SetAmount"), AccessTools.Method(typeof(ProbeCreature), "ApplyPowerInternal"),
            AccessTools.Method(typeof(ProbeCreature), "RemovePowerInternal"),
            AccessTools.Method(typeof(DamageFixture), "Damage")
        })
            harmony.Patch(target, prefix: new HarmonyMethod(typeof(ManagedFixtures), nameof(EarlierPrefix)) { priority = Priority.First });
        Test("Prefix/Finalizer synchronous restoration, barrier and original Task identity", ScopeBasics);
        Test("suspended, nested and overlapping flows on registered thread", AsyncScopes);
        Test("earlier Harmony skip preserves producer, pending, wrapper/orb and damage callers", SkippedPrefixes);
        Test("source-copy and upload finally release on every failure", TransferFailures);
        Test("accepted first attachment, stack ordering and notification exceptions", PowerMutations);
        Test("Misery clone zero-delta attachment and temporary null-card forwarding", TemporaryAndClone);
        Test("dirty provenance invalidation ordering and Unknown recovery", DirtyRecovery);
        Test("failed observations and removal preserve provenance failure ordering", MutationFailures);
        Test("stale mutation finalizers leave replacement combat untouched", StaleMutation);
        Test("detached callback provenance and death removal batches", DetachedPowers);
        Test("generation ancestry, cross-player supplier, regeneration and identity budget", Generation);
        Test("saved dictionary sources exclude later stacks; Rupture exact increments", Temporal);
        Test("nested card/potion cleanup and outer orb flag restoration", NestedPlays);
        Test("canonical live bridge, complete group seal and original Task/result identity", DamageGroups);
        Test("capture/append/commit fallback, exception abort and overlapping damage", DamageFailures);
        Test("fault and cancellation preserve original Tasks and cannot contaminate a later modifier-bearing hit", FailureThenCleanHit);
        Test("actual Sleight and Defy bodies: base 9, upgrade 13 and stacked 22 supplier capture", SleightAndDefy);
        Test("actual potion/relic/power triggers and Deathbringer's separate Doom/Weak procs", SleightTriggerKinds);
        Test("actual Cacophony draws from another player preserve the supplier across await", CacophonyDraws);
        Test("nested canonical parent/child awaits preserve distinct sources and modifier batches", NestedDamage);
        Test("multiple original targets retain independent modifiers and redirected result groups", MultipleDamageTargets);
        Test("actual Poison turn/Outbreak ticks refresh suppliers and actual Rolling Boulder callback captures without ambient scope", ActualPoisonAndRolling);
        Test("actual Sentry generation and Frost/Hibernate block retain suppliers across awaits", SentryAndFrost);
        Test("completed groups precede later fault/combat replacement and exclude overkill", DamageGroupTiming);
        Test("exact independent result classification precedence", Classifications);
        Test("actual model ownership and accepted signed amount adapters", ActualModelAdapters);
        Test("modifier/enemy/Weak rejection invalidates whole groups with evidence intact", CaptureStatusFailures);
        Test("block/Forge/summon immutable scopes, original kickoff timing and inheritance", CommandSources);
        Test("Doom nested batches, original Task and synthetic fallback remainder", DoomBatches);
        Test("orb channel failure and absent explicit Osty card source", OrbAndOsty);
        Test("Poison per-tick refresh rejects missing duration and unrelated targets", PoisonTicks);
        Test("stale suspended command cannot enter a replacement combat", StaleCommand);
        Test("generation before setup epoch remains unavailable", PreSetupGeneration);
        Test("stale combat and wrong-thread rejection without native calls", EpochAndThread);
        Console.WriteLine($"FIXTURE CASES {cases}; ASSERTIONS {assertions}");
        Console.WriteLine("Engine/native boundaries are injected. Installed-game transpilers are installed and checked; real gameplay remains an integration/manual gate.");
    }
    private static void Check(bool value, string reason) { assertions++; if (!value) throw new InvalidOperationException("Assertion failed: " + reason); }
    private static void Same(SourceSnapshot actual, SourceSnapshot expected, string reason)
    {
        Check(actual.Epoch == expected.Epoch && actual.Count == expected.Count
            && Enumerable.Range(0, actual.Count).All(i => actual[i] == expected[i]), reason);
    }
    private static TaskCompletionSource<int> Pause() => new(TaskCreationOptions.RunContinuationsAsynchronously);
    private static void Test(string name, Action body)
    {
        backend = new FakeBackend { Combat = RuntimeHelpers.GetUninitializedObject(typeof(CombatState)) };
        CaptureRuntime.Register(backend, ++epoch, backend.Combat);
        FlowCapture.Current = ProducerFrame.Barrier;
        A = SourceSnapshot.Create(epoch, new[] { new SourceShare(epoch << 32, 1) });
        B = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 8, 1) });
        backend.NamedSources["A"] = A;
        backend.NamedSources["B"] = B;
        CommandPause = null;
        DuringBeforeApplied = null;
        DamageFixture.DuringLive = null;
        DamageFixture.Nested = null;
        DamageFixture.AfterGroup = null;
        DamageFixture.ResultsByTarget = null;
        DamageFixture.ModifiersByTarget = null;
        GameProducerFixture.Listener = null;
        GameProducerFixture.Enemies = Array.Empty<Creature>();
        GameProducerFixture.WaitTask = Task.CompletedTask;
        GameProducerFixture.Blocked = 0;
        GameProducerFixture.Applied = null;
        GameProducerFixture.Decremented = null;
        GameProducerFixture.Created = null;
        GameProducerFixture.Generated.Clear();
        GameProducerFixture.Applications.Clear();
        GameProducerFixture.Emissions.Clear();
        skipPrefix = false;
        HistoryAmounts.Clear();
        DamageFixture.Pause = Task.CompletedTask;
        DamageFixture.Error = null;
        DamageFixture.PreviewDuringDamage = false;
        DamageFixture.Modifiers = Array.Empty<AbstractModel>();
        DamageFixture.Bonus = 0;
        DamageCapture.Inspect = (_, _, _) => default;
        CommandFixture.Pause = Task.CompletedTask;
        CommandFixture.During = null;
        CommandFixture.Calls = 0;
        DoomFixture.OriginalTask = Task.CompletedTask;
        DoomFixture.Error = null;
        DoomFixture.During = null;
        DoomFixture.Calls = 0;
        DamageFixture.Enumerated = DamageFixture.LateEnumerated = DamageFixture.HookCalls = 0;
        body();
        Check(backend.OpenLeases == 0, "No source transfer survives fixture");
        Check(backend.OpenCalculations == 0, "No unfinished calculation survives fixture");
        Check(backend.DoomBatches.Count == 0, "No unfinished Doom batch survives fixture");
        cases++;
        Console.WriteLine("PASS " + name);
    }
    private static ProducerFrame Frame(object model, SourceSnapshot source, ProducerRole role = ProducerRole.Card)
        => new(CaptureRuntime.Epoch, model, source, role, role == ProducerRole.Power ? DamageSegment.Attributed : DamageSegment.Direct);
    private static void ScopeBasics()
    {
        var a = new ProbeModel("A");
        var b = new ProbeModel("B", ProducerRole.Power);
        FlowCapture.Current = Frame(a, A);
        Same(b.Synchronous(), B, "Independent power supplants outer card");
        Same(FlowCapture.Current.Source, A, "Synchronous Finalizer restores outer source");
        b.FailCapture = true;
        Check(b.Synchronous().Epoch == 0, "Capture failure installs Unknown barrier before descriptor work");
        Same(FlowCapture.Current.Source, A, "Failed capture restores outer source");
        b.FailCapture = false;
        var pending = Pause();
        Check(ReferenceEquals(pending.Task, b.ReturnTask(pending.Task)), "Original Task identity is unchanged");
        pending.SetResult(37);
        Check(pending.Task.Result == 37, "Original result unchanged");
        var error = new InvalidOperationException("original game failure");
        try { b.Throw(error); Check(false, "Original throw must propagate"); } catch (InvalidOperationException actual) { Check(ReferenceEquals(actual, error), "Finalizer preserves original exception object"); }
        Same(FlowCapture.Current.Source, A, "Throw restores outer source");
        Same(a.Nested(b, source => Same(source, B, "Nested supplier")), A, "Nested return restores caller");
    }
    private static void AsyncScopes()
    {
        var a = new ProbeModel("A"); var b = new ProbeModel("B", ProducerRole.Power);
        var first = Pause(); var second = Pause();
        Task<SourceSnapshot> one = a.Suspended(first.Task), two = b.Suspended(second.Task);
        Check(FlowCapture.Current.Epoch.Sequence == 0, "Async kickoff restores caller immediately");
        second.SetResult(1); context.Complete(two); Same(two.Result, B, "Second overlapping operation owns B");
        first.SetResult(1); context.Complete(one); Same(one.Result, A, "First overlapping operation still owns A");
        var rollingPause = Pause();
        var rolling = b.DoDamage(rollingPause.Task);
        rollingPause.SetResult(1); context.Complete(rolling);
        Same(rolling.Result, B, "Independent Rolling Boulder-style entry captures actual power without inherited context");
    }
    private static void SkippedPrefixes()
    {
        var outer = new ProbeModel("A"); var inner = new ProbeModel("B");
        var producer = Frame(outer, A);
        FlowCapture.Current = producer;
        skipPrefix = true;
        try
        {
            Check(inner.Synchronous() == null, "Earlier false Prefix suppresses original producer body");
            Check(ReferenceEquals(FlowCapture.Current, producer), "Never-entered producer Finalizer preserves caller frame");
        }
        finally { skipPrefix = false; }
        var owner = new ProbeCreature(); var power = new ProbePower("SKIPPED");
        DuringBeforeApplied = () =>
        {
            var prior = ProvenanceCapture.Pending;
            skipPrefix = true;
            try
            {
                Check(ProbeCommands.ModifyAmount(null, power, 1, null, inner) == null, "Earlier Prefix suppresses nested command body");
                Check(ReferenceEquals(ProvenanceCapture.Pending, prior), "Never-entered pending Finalizer preserves exact caller scope");
            }
            finally { skipPrefix = false; }
        };
        context.Complete(ProbeCommands.Apply(null, power, owner, 2, null, outer));
        DuringBeforeApplied = null;
        int calls = backend.Calls.Count;
        skipPrefix = true;
        try
        {
            power.SetAmount(9);
            owner.RemovePowerInternal(power);
            owner.ApplyPowerInternal(new ProbePower("SKIPPED_ATTACHMENT"));
            Check(power.Amount == 2 && owner.Powers.Count == 1 && owner.Powers.Contains(power), "Earlier Prefix suppresses all mutation bodies");
            Check(backend.Calls.Count == calls, "Never-entered mutation Finalizers make no native calls");
        }
        finally { skipPrefix = false; }
        context.Complete(outer.OnPlayWrapper(() =>
        {
            PlayCapture.Started(outer, 0, 0, 1);
            var execution = PlayCapture.Execution; var play = PlayCapture.Current; var prior = FlowCapture.Current;
            var orb = new ProbeModel("B", ProducerRole.Orb);
            skipPrefix = true;
            try
            {
                Check(inner.OnPlayWrapper(() => Task.CompletedTask) == null, "Earlier Prefix suppresses nested card wrapper");
                Check(inner.OnUseWrapper(() => Task.CompletedTask) == null, "Earlier Prefix suppresses nested potion wrapper");
                Check(ReferenceEquals(PlayCapture.Execution, execution) && ReferenceEquals(PlayCapture.Current, play), "Never-entered wrappers preserve caller execution and exact play");
                Check(orb.Passive() == null && ReferenceEquals(FlowCapture.Current, prior), "Never-entered orb Finalizer preserves caller producer");
            }
            finally { skipPrefix = false; }
            PlayCapture.Finished(outer);
            return Task.CompletedTask;
        }));
        var enemy = Creature(); var player = Creature(true, false, 0);
        DamageFixture.Results = new() { new(enemy, ValueProp.Move) { UnblockedDamage = 2 } };
        DamageFixture.DuringLive = () =>
        {
            var prior = DamageCapture.Current;
            skipPrefix = true;
            try
            {
                Check(Damage(enemy, player) == null, "Earlier Prefix suppresses nested canonical damage");
                Check(ReferenceEquals(DamageCapture.Current, prior) && prior.Calculation != 0, "Never-entered damage Finalizer preserves caller calculation");
            }
            finally { skipPrefix = false; }
        };
        context.Complete(Damage(enemy, player));
        DamageFixture.DuringLive = null;
        Check(backend.Committed.Count == 1 && backend.OpenCalculations == 0, "Caller damage still commits exactly once after skipped nested call");
    }
    private static void TransferFailures()
    {
        var a = new ProbeModel("A");
        foreach (var failure in new[] { "SourceCount", "SourceDestination", "SourceWeight" })
        {
            backend.Failure = failure; backend.FailureCount = 1;
            int released = backend.Released;
            Check(FlowCapture.Source(a, CaptureRuntime.Epoch).Epoch == 0, "Malformed copy returns unavailable: " + failure);
            Check(backend.Released == released + 1 && backend.OpenLeases == 0, "Read lease released in finally: " + failure);
        }
        foreach (var failure in new[] { "TransferBegin", "TransferAdd", "TransferSeal" })
        {
            backend.Failure = failure; backend.FailureCount = 1;
            int calls = 0;
            int accepted = CaptureRuntime.Upload(CaptureRuntime.Epoch, A, lease => { calls++; Check(lease == 0, "Upload failure becomes explicit Unknown"); return 1; });
            Check(accepted == 1 && calls == 1 && backend.OpenLeases == 0, "Upload consumer runs exactly once: " + failure);
        }
        backend.Failure = null;
        var error = new InvalidOperationException("consumer failed");
        try { CaptureRuntime.Upload<int>(CaptureRuntime.Epoch, A, _ => throw error); } catch (InvalidOperationException actual) { Check(ReferenceEquals(error, actual), "Consumer exception preserved"); }
        Check(backend.OpenLeases == 0, "Consumer exception releases sealed lease");
    }
    private static void PowerMutations()
    {
        var owner = new ProbeCreature(); var power = new ProbePower("POWER"); var card = new ProbeModel("A");
        context.Complete(ProbeCommands.Apply(null, power, owner, 3, null, card));
        Check(backend.PowerEvents.Count == 1 && backend.PowerEvents[0].Kind == "attach" && backend.PowerEvents[0].After == 3, "Initial SetAmount is not double booked");
        Same(backend.PowerEvents[0].Source, A, "First attachment inherits supplied source");
        context.Complete(ProbeCommands.ModifyAmount(null, power, 2, null, new ProbeModel("B")));
        Check(HistoryAmounts.Single() == 3 && backend.PowerEvents.Last().Before == 3 && backend.PowerEvents.Last().After == 5, "History-before-SetAmount cannot duplicate accepted stack");
        Same(backend.PowerEvents.Last().Source, B, "Accepted change freezes incoming supplier");
        var error = new InvalidOperationException("amount notification"); power.NotificationException = error;
        try { power.SetAmount(8); } catch (InvalidOperationException actual) { Check(ReferenceEquals(error, actual), "Notification throw preserved"); }
        Check(backend.PowerEvents.Last().After == 8, "Mutation preceding notification throw is captured");
        power.NotificationException = null;
        var pause = Pause(); CommandPause = pause.Task;
        var pending = ProbeCommands.ModifyAmount(null, power, 1, null, card);
        Check(ProvenanceCapture.Pending.Epoch.Sequence == 0, "Async power command restores parent pending scope");
        pause.SetResult(1); context.Complete(pending);
        Same(backend.PowerEvents.Last().Source, A, "Accepted mutation after await retains command supplier");
    }
    private static void TemporaryAndClone()
    {
        var owner = new ProbeCreature();
        var clone = new ProbePower("MISERY_CLONE") { Amount = 4 };
        context.Complete(ProbeCommands.Apply(null, clone, owner, 4, null, new ProbeModel("B")));
        Check(backend.PowerEvents.Single().After == 4, "Misery attachment books full amount despite zero initial SetAmount delta");
        Same(backend.PowerEvents.Single().Source, B, "Misery explicit source replaces former clone lineage");
        var temp = new ProbePower("TEMPORARY") { Temporary = true };
        FlowCapture.Current = Frame(new ProbeModel("A", ProducerRole.Potion), A, ProducerRole.Potion);
        context.Complete(ProbeCommands.Apply(null, temp, owner, 2, null, null));
        Same(TemporaryObserved, A, "Temporary BeforeApplied forwards matching pending potion source before attachment");
        FlowCapture.Current = Frame(new ProbeModel("B", ProducerRole.Potion), B, ProducerRole.Potion);
        context.Complete(ProbeCommands.ModifyAmount(null, temp, 1, null, null));
        Same(TemporaryObserved, B, "Temporary self-change forwards new null-card incoming source");
        var sleight = new ProbePower("A");
        Same(sleight.Synchronous(), A, "Unrelated listener retains its own provenance");
    }
    private static void DirtyRecovery()
    {
        var owner = new ProbeCreature(); var power = new ProbePower("DIRTY");
        context.Complete(ProbeCommands.Apply(null, power, owner, 2, null, new ProbeModel("A")));
        var metadata = IdentityCapture.Get(power, CaptureRuntime.Epoch);
        backend.Failure = "PowerChanged"; backend.FailureCount = 1;
        power.SetAmount(3);
        Check(metadata.Dirty, "Rejected accepted mutation leaves managed instance dirty");
        Check(FlowCapture.Source(power, CaptureRuntime.Epoch).Epoch == 0, "Dirty power blocks all managed source capture");
        backend.Calls.Clear(); backend.Failure = "PowerInvalidate"; backend.FailureCount = 2;
        power.SetAmount(4);
        Check(!backend.Calls.Contains("PowerChanged"), "Prior dirty invalidation must succeed before any recovery update");
        backend.Failure = null; backend.Calls.Clear();
        power.SetAmount(4);
        Check(!metadata.Dirty && backend.Calls.IndexOf("PowerInvalidate") < backend.Calls.IndexOf("PowerChanged"), "Correctly ordered accepted recovery clears dirty");
        Same(FlowCapture.Source(power, CaptureRuntime.Epoch), SourceSnapshot.Unknown(epoch), "Same-amount recovery cannot reclaim old balance");
    }
    private static void MutationFailures()
    {
        var owner = new ProbeCreature(); var power = new ProbePower("OBSERVED");
        context.Complete(ProbeCommands.Apply(null, power, owner, 2, null, new ProbeModel("A")));
        var metadata = IdentityCapture.Get(power, CaptureRuntime.Epoch);
        foreach (int observation in new[] { 0, 1 })
        {
            backend.Calls.Clear(); int events = backend.PowerEvents.Count;
            backend.FailObservationAfter = observation;
            var notification = new InvalidOperationException("mutation notification");
            power.NotificationException = notification;
            Exception thrown = null;
            try { power.SetAmount(power.Amount + 1); } catch (InvalidOperationException actual) { thrown = actual; }
            Check(ReferenceEquals(notification, thrown), "Failed observation preserves the original game exception after mutation");
            power.NotificationException = null;
            Check(metadata.Dirty && backend.Calls.Contains("PowerInvalidate") && backend.PowerEvents.Count == events,
                "Failure before or after the mutation invalidates without guessing an observed delta");
            Check(FlowCapture.Source(power, CaptureRuntime.Epoch).Epoch == 0, "Failed observation cannot expose old provenance");
            backend.Calls.Clear(); power.SetAmount(power.Amount);
            Check(!metadata.Dirty && backend.Calls.IndexOf("PowerInvalidate") < backend.Calls.IndexOf("PowerChanged"),
                "Observation failure recovery invalidates before accepting the current amount");
        }
        var error = new InvalidOperationException("removal rejected"); owner.RemovalException = error;
        backend.Calls.Clear();
        Exception observed = null;
        try { owner.RemovePowerInternal(power); } catch (InvalidOperationException actual) { observed = actual; }
        Check(ReferenceEquals(error, observed), "Pre-removal failure preserves the game exception");
        Check(owner.Powers.Contains(power) && metadata.Detached == null && !backend.Calls.Contains("PowerRemoved"),
            "A rejected removal clears its speculative detached snapshot without retiring attached provenance");
        owner.RemovalException = null;
        TemporalPowerCapture.Save(power, null, 1, A, CaptureRuntime.Epoch);
        backend.Failure = "PowerRemoved"; backend.FailureCount = 1;
        owner.RemovePowerInternal(power);
        Check(!owner.Powers.Contains(power) && metadata.Dirty && metadata.Detached != null,
            "Rejected native retirement leaves the actual detached power dirty");
        Check(FlowCapture.Source(power, CaptureRuntime.Epoch).Epoch == 0, "Dirty detached metadata cannot bypass capture rejection");
        Check(TemporalPowerCapture.Take(power, null, CaptureRuntime.Epoch).Epoch == 0, "Actual removal clears temporal entries even when native retirement is rejected");

        var unavailable = new ProbePower("UNAVAILABLE");
        context.Complete(ProbeCommands.Apply(null, unavailable, owner, 1, null, new ProbeModel("A")));
        var unavailableMetadata = IdentityCapture.Get(unavailable, CaptureRuntime.Epoch);
        unavailable.FailCapture = true;
        unavailable.NotificationException = new InvalidOperationException("detached notification");
        observed = null; backend.Calls.Clear();
        try { owner.RemovePowerInternal(unavailable); } catch (InvalidOperationException actual) { observed = actual; }
        Check(ReferenceEquals(unavailable.NotificationException, observed), "Removal after source failure preserves the notification exception");
        Check(backend.Calls.Contains("PowerRemoved") && !unavailableMetadata.Dirty && unavailableMetadata.Detached?.Epoch == 0,
            "Unavailable source still allows observed removal and cannot restore the old source");
    }
    private static void StaleMutation()
    {
        var owner = new ProbeCreature(); var power = new ProbePower("STALE");
        context.Complete(ProbeCommands.Apply(null, power, owner, 2, null, new ProbeModel("A")));
        var method = AccessTools.Method(typeof(ProbePower), "SetAmount");
        ProvenanceCapture.MutationPrefix(power, method, new object[] { 3 }, out var observed);
        backend.FailObservationAfter = 0;
        ProvenanceCapture.MutationPrefix(power, method, new object[] { 3 }, out var failed);
        var nextCombat = RuntimeHelpers.GetUninitializedObject(typeof(CombatState)); backend.Combat = nextCombat;
        CaptureRuntime.Register(backend, ++epoch, nextCombat);
        var metadata = IdentityCapture.Get(power, CaptureRuntime.Epoch);
        int calls = backend.Calls.Count;
        ProvenanceCapture.MutationFinalizer(observed);
        ProvenanceCapture.MutationFinalizer(failed);
        Check(backend.Calls.Count == calls && !metadata.Dirty, "Stale observed and failed mutations cannot update or invalidate the replacement combat");
    }
    private static void DetachedPowers()
    {
        var owner = new ProbeCreature(); var first = new ProbePower("FIRST"); var second = new ProbePower("SECOND");
        context.Complete(ProbeCommands.Apply(null, first, owner, 2, null, new ProbeModel("A")));
        context.Complete(ProbeCommands.Apply(null, second, owner, 2, null, new ProbeModel("B")));
        foreach (var power in owner.Powers.ToArray()) owner.RemovePowerInternal(power);
        Same(FlowCapture.Source(first, CaptureRuntime.Epoch), A, "Detached first power keeps copied source for AfterRemoved");
        Same(FlowCapture.Source(second, CaptureRuntime.Epoch), B, "Death batch preserves each detached instance");
        Check(owner.Powers.Count == 0 && backend.Calls.Count(c => c == "PowerRemoved") == 2, "Each actual removal retires native balances once");
    }
    private static void Generation()
    {
        var generated = new ProbeModel("GENERATED", ProducerRole.Card, 1);
        FlowCapture.Current = Frame(new ProbeModel("A"), A);
        ProvenanceCapture.Generated(generated);
        var metadata = IdentityCapture.Get(generated, CaptureRuntime.Epoch);
        Check(metadata.Generation == GenerationState.GeneratedRecorded, "Generation marked recorded only after acceptance");
        Same(FlowCapture.Source(generated, CaptureRuntime.Epoch), A, "Generated recipient slot cannot replace creditor source");
        FlowCapture.Current = Frame(generated, FlowCapture.Source(generated, CaptureRuntime.Epoch));
        var child = new ProbeModel("CHILD", ProducerRole.Card, 2);
        ProvenanceCapture.Generated(child);
        Same(FlowCapture.Source(child, CaptureRuntime.Epoch), A, "Generated-card ancestry preserves root supplier");
        backend.Failure = "CardGenerated"; backend.FailureCount = 1; ProvenanceCapture.Generated(generated);
        Check(metadata.Generation == GenerationState.GeneratedUnavailable, "Failed regeneration overrides older successful record");
        Same(FlowCapture.Source(generated, CaptureRuntime.Epoch), SourceSnapshot.Unknown(epoch), "Failed regeneration cannot revive old native source");
        var a = new CollisionModel(); var b = new CollisionModel();
        Check(IdentityCapture.Get(a, CaptureRuntime.Epoch).Identity != IdentityCapture.Get(b, CaptureRuntime.Epoch).Identity, "Weak identity allocator ignores hash collisions");
        for (int i = 0; i < IdentityCapture.PerCombat; i++) IdentityCapture.Get(new object(), CaptureRuntime.Epoch);
        var unavailable = new ProbeModel("A");
        Check(IdentityCapture.Get(unavailable, CaptureRuntime.Epoch) == null, "Identity capacity has no unbounded failure registry");
        PlayCapture.Started(unavailable, 0, 0, 1);
        Check(backend.PlayGenerations.Last() == GenerationState.Unclassified, "Unidentified played object cannot become named ordinary card");
        PlayCapture.Finished(unavailable);
    }
    private sealed class CollisionModel { public override int GetHashCode() => 7; }
    private static CardModel Card()
    {
        var card = (CardModel)RuntimeHelpers.GetUninitializedObject(typeof(MegaCrit.Sts2.Core.Models.Cards.StrikeIronclad));
        backend.Descriptors[card] = new(CaptureKind.CardInstance, ProducerRole.Card, "A", 0, 0);
        return card;
    }
    private static void Temporal()
    {
        var power = new RupturePowerFixture(); var card = Card();
        backend.NamedSources["TEMPORAL"] = A;
        context.Complete(power.BeforeCardPlayed(card, 2));
        backend.NamedSources["TEMPORAL"] = B;
        var pause = Pause(); var emission = power.AfterCardPlayed(card, pause.Task);
        pause.SetResult(1); context.Complete(emission);
        Same(emission.Result, A, "Saved Add/Remove source excludes newly added Storm-family stacks across await");
        var missing = power.AfterCardPlayed(card, Task.CompletedTask); context.Complete(missing);
        Check(missing.Result.Epoch == 0, "Missing saved entry never substitutes current mixture");
        context.Complete(power.BeforeCardPlayed(card, 0));
        backend.NamedSources["TEMPORAL"] = A; power.AfterDamageReceived(card, 2);
        backend.NamedSources["TEMPORAL"] = B; power.AfterDamageReceived(card, 1);
        var accumulated = power.AfterCardPlayed(card, Task.CompletedTask); context.Complete(accumulated);
        var expected = SourceSnapshot.Create(epoch, new[] { new SourceShare(A[0].Destination, 2), new SourceShare(B[0].Destination, 1) });
        Same(accumulated.Result, expected, "Rupture accumulates actual per-increment suppliers at weights 2:1");
        var mixture = SourceSnapshot.Create(epoch, new[] { new SourceShare(A[0].Destination, 1), new SourceShare(B[0].Destination, 1) });
        Same(TemporalPowerCapture.Combine(CaptureRuntime.Epoch, mixture, 1, mixture, 1), mixture, "One-unit mixed suppliers are not rounded to one root");
        TemporalPowerCapture.Save(power, null, 2, A, CaptureRuntime.Epoch);
        Same(TemporalPowerCapture.TurnSource(power, CaptureRuntime.Epoch), A, "HelloWorld turn source is readable by the first callback");
        Same(TemporalPowerCapture.TurnSource(power, CaptureRuntime.Epoch), A, "Another player's callback cannot consume HelloWorld's turn source");
        TemporalPowerCapture.Save(power, null, 3, B, CaptureRuntime.Epoch);
        Same(TemporalPowerCapture.TurnSource(power, CaptureRuntime.Epoch), B, "Next AmountOnTurnStart assignment replaces saved turn provenance");
        var originalError = new ArgumentException();
        try { power.Amounts.Add(card, 1); power.BeforeCardPlayed(card, 1).GetAwaiter().GetResult(); } catch (ArgumentException) { originalError = null; }
        Check(originalError == null, "Dictionary Add exception preserved by exact bridge");
    }
    private static void NestedPlays()
    {
        var outer = new ProbeModel("A"); var inner = new ProbeModel("B"); var potion = new ProbeModel("B", ProducerRole.Potion); var orb = new ProbeModel("B", ProducerRole.Orb);
        var run = outer.OnPlayWrapper(async () =>
        {
            PlayCapture.Started(outer, 0, 0, 1);
            ulong outerToken = PlayCapture.Current.Token;
            Same(orb.Passive(), B, "Outer first orb trigger uses channel source");
            await inner.OnPlayWrapper(() =>
            {
                PlayCapture.Started(inner, 0, 0, 1);
                Same(orb.Passive(), B, "Inner first orb trigger uses own channel decision");
                return Task.CompletedTask;
            });
            Check(PlayCapture.Current.Token == outerToken && backend.Plays.ContainsKey(outerToken), "Nested wrapper cleanup preserves exact outer play");
            Same(orb.Passive(), A, "Outer later orb trigger retains first-trigger flag and saved source");
            await potion.OnUseWrapper(() => Task.CompletedTask);
            Check(backend.Plays.ContainsKey(outerToken), "Potion execution cleanup does not close outer card");
            PlayCapture.Finished(outer);
        });
        context.Complete(run);
        Check(backend.Plays.Count == 0 && backend.Started == 2 && backend.ExecutionEnds == 3, "All exact wrapper executions cleaned without count retry");
        var failure = new InvalidOperationException("nested autoplay death/exception");
        var failed = outer.OnPlayWrapper(async () =>
        {
            PlayCapture.Started(outer, 0, 0, 1);
            await inner.OnPlayWrapper(() => { PlayCapture.Started(inner, 0, 0, 1); throw failure; });
        });
        try { context.Complete(failed); } catch (InvalidOperationException actual) { Check(ReferenceEquals(actual, failure), "Nested exception preserved"); }
        Check(backend.Plays.Count == 0, "Guaranteed finally closes inner and outer orphan plays");
    }
    private static Creature Creature(bool player = false, bool osty = false, int slot = 4)
    {
        var creature = (Creature)RuntimeHelpers.GetUninitializedObject(typeof(Creature));
        backend.Creatures[creature] = new(player, osty, slot, backend.Combat);
        return creature;
    }
    private static Task<IEnumerable<DamageResult>> Damage(Creature target, Creature dealer = null, CardModel card = null)
        => DamageFixture.Damage(null, new[] { target }, 10, ValueProp.Move, dealer, card, null);
    private static void DamageGroups()
    {
        var enemy = Creature(); var player = Creature(true, false, 0); var card = Card();
        DamageFixture.PreviewDuringDamage = true;
        DamageFixture.Results = new() { new(enemy, ValueProp.Move) { UnblockedDamage = 1, BlockedDamage = 3 }, new(enemy, ValueProp.Move) { UnblockedDamage = 2 } };
        var pause = Pause(); DamageFixture.Pause = pause.Task;
        var task = Damage(enemy, player, card);
        Check(ReferenceEquals(task, DamageFixture.OriginalTask), "Canonical patch returns original game Task object");
        Check(backend.OpenCalculations == 1 && backend.Committed.Count == 0, "Live calculation awaits complete finalized result group");
        int begins = backend.Begun.Count;
        DamageFixture.Preview(enemy, player, card);
        Check(backend.Begun.Count == begins, "Preview hook outside canonical bridge cannot overwrite live calculations");
        pause.SetResult(1); context.Complete(task);
        Check(ReferenceEquals(task.Result, DamageFixture.Results), "Builder completion bridge preserves original result object");
        Check(backend.Committed.Count == 2 && backend.Committed.Sum(p => p.Total) == 6 && backend.Fallback.Count == 0, "Complete redirected group appended and committed once");
        Check(backend.Begun.Count == begins, "CardPreviewMode.None preview inside resumed live operation cannot begin capture");
        Check(DamageFixture.Enumerated == 6 && DamageFixture.LateEnumerated == 6, "Both original enumerations execute unchanged");
        Check(backend.Begun[0].Role == ProducerRole.Card && backend.Begun[0].Segment == DamageSegment.Direct, "Canonical card source overrides incidental producer");
        Same(backend.Begun[0].Source, A, "Canonical card uses instance source");
    }
    private static void DamageFailures()
    {
        var enemy = Creature(); var player = Creature(true, false, 1);
        foreach (var failure in new[] { "DamageBegin", "DamageAppend", "DamageCommit" })
        {
            backend.Failure = failure; backend.FailureCount = 1;
            int before = backend.Fallback.Count, committed = backend.Committed.Count;
            DamageFixture.Results = new() { new(enemy, ValueProp.Move) { UnblockedDamage = 2, BlockedDamage = 2 }, new(enemy, ValueProp.Move) { UnblockedDamage = 1 } };
            context.Complete(Damage(enemy, player));
            Check(backend.Fallback.Count == before + 2 && backend.Committed.Count == committed && backend.OpenCalculations == 0, "Failure aborts whole group and falls back once per complete result: " + failure);
        }
        backend.Failure = null;
        var pause = Pause(); DamageFixture.Pause = pause.Task;
        var first = Damage(enemy, player);
        var second = Damage(enemy, player);
        Check(backend.OpenCalculations == 2, "Overlapping canonical operations own distinct calculation tokens");
        pause.SetResult(1); context.Complete(first); context.Complete(second);
        var error = new InvalidOperationException("game fails before completed group");
        DamageFixture.Pause = Task.CompletedTask; DamageFixture.Error = error;
        int beforeFallback = backend.Fallback.Count;
        try { context.Complete(Damage(enemy, player)); } catch (InvalidOperationException actual) { Check(ReferenceEquals(actual, error), "Builder SetException preserves game exception"); }
        Check(backend.OpenCalculations == 0 && backend.Fallback.Count == beforeFallback, "No invented partial result after game exception");
    }
    private static void FailureThenCleanHit()
    {
        var world = GameWorld();
        var failedCard = GameCard<StrikeIronclad>(world.Owner, A);
        var cleanCard = GameCard<StrikeIronclad>(world.Other, B);
        var laterModifier = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 16, 1) });
        var firstStrength = GamePower<StrengthPower>(world.Owner.Creature, 3, A);
        var nextStrength = GamePower<StrengthPower>(world.Other.Creature, 2, laterModifier);
        DamageCapture.Inspect = (_, _, _) => new(true, false, 0, null, false);
        foreach (bool cancel in new[] { false, true })
        {
            int before = backend.DamageGroups.Count;
            int fallbacks = backend.Fallback.Count;
            var error = new InvalidOperationException("game fault before result group");
            var pause = Pause();
            using var cancellation = new CancellationTokenSource();
            DamageFixture.Pause = cancel ? pause.Task : Task.CompletedTask;
            DamageFixture.Error = cancel ? null : error;
            DamageFixture.Modifiers = new[] { firstStrength }; DamageFixture.Bonus = 3;
            DamageFixture.Results = new() { new(world.Enemy, ValueProp.Move) { UnblockedDamage = 7, BlockedDamage = 1 } };
            var failed = Damage(world.Enemy, world.Owner.Creature, failedCard);
            Check(ReferenceEquals(failed, DamageFixture.OriginalTask), "Fault/cancellation keeps the original canonical Task");
            if (cancel) { cancellation.Cancel(); pause.SetCanceled(cancellation.Token); }
            bool observed = false;
            try { context.Complete(failed); }
            catch (Exception actual)
            {
                observed = cancel ? actual is OperationCanceledException canceled && canceled.CancellationToken == cancellation.Token
                    : ReferenceEquals(actual, error);
            }
            Check(observed && failed.IsCanceled == cancel && backend.OpenCalculations == 0
                && backend.DamageGroups.Count == before && backend.Fallback.Count == fallbacks,
                "Original fault/cancellation and token survive cleanup without publishing an incomplete group");
            ulong abandoned = backend.DamageModifiers.Last().Calculation;
            DamageFixture.Pause = Task.CompletedTask; DamageFixture.Error = null;
            DamageFixture.Modifiers = new[] { nextStrength }; DamageFixture.Bonus = 2;
            DamageFixture.Results = new() { new(world.Enemy, ValueProp.Move) { UnblockedDamage = 4, BlockedDamage = 2 } };
            context.Complete(Damage(world.Enemy, world.Other.Creature, cleanCard));
            var group = backend.DamageGroups.Last();
            Check(backend.DamageGroups.Count == before + 1 && backend.Fallback.Count == fallbacks
                && group.Calculation != abandoned && group.Packets.Single().Total == 6 && group.Packets.Single().Blocked == 2,
                "The later hit commits its own six points once, without adopting the abandoned result");
            Same(group.Source, B, "The later card source excludes the abandoned card supplier");
            var contribution = backend.DamageModifiers.Single(modifier => modifier.Calculation == group.Calculation);
            Same(contribution.Source, laterModifier, "The later modifier uses only its new captured supplier");
            Check(contribution.Amount == 2 && backend.OpenCalculations == 0, "The abandoned three-point modifier cannot leak into the later two-point budget");
        }
    }
    private static (Player Owner, Player Other, Creature Enemy) GameWorld()
    {
        var players = new[] { (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player)), (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player)) };
        var run = Roster(players);
        AccessTools.Field(typeof(RunState), "<Rng>k__BackingField").SetValue(run, new RunRngSet("managed-producer-111"));
        for (int slot = 0; slot < players.Length; slot++)
        {
            var creature = Creature(true, false, slot);
            creature.CombatState = (ICombatState)backend.Combat;
            AccessTools.Field(typeof(Creature), "<Player>k__BackingField").SetValue(creature, players[slot]);
            AccessTools.Field(typeof(Creature), "<Side>k__BackingField").SetValue(creature, CombatSide.Player);
            AccessTools.Field(typeof(Creature), "_powers").SetValue(creature, new List<PowerModel>());
            AccessTools.Field(typeof(Creature), "_currentHp").SetValue(creature, 100);
            AccessTools.Field(typeof(Player), "<Creature>k__BackingField").SetValue(players[slot], creature);
            AccessTools.Field(typeof(Player), "<Character>k__BackingField").SetValue(players[slot], Mutable<MegaCrit.Sts2.Core.Models.Characters.Necrobinder>("NECROBINDER"));
            var playerCombat = RuntimeHelpers.GetUninitializedObject(typeof(PlayerCombatState));
            AccessTools.Field(typeof(PlayerCombatState), "<TurnNumber>k__BackingField").SetValue(playerCombat, 1);
            AccessTools.Field(typeof(Player), "<PlayerCombatState>k__BackingField").SetValue(players[slot], playerCombat);
            AccessTools.Field(typeof(Player), "_runState").SetValue(players[slot], run);
        }
        var enemy = Creature();
        enemy.CombatState = (ICombatState)backend.Combat;
        AccessTools.Field(typeof(Creature), "<Side>k__BackingField").SetValue(enemy, CombatSide.Enemy);
        AccessTools.Field(typeof(Creature), "_powers").SetValue(enemy, new List<PowerModel>());
        AccessTools.Field(typeof(Creature), "_currentHp").SetValue(enemy, 100);
        AccessTools.Field(typeof(CombatState), "_allies").SetValue(backend.Combat, players.Select(player => player.Creature).ToList());
        AccessTools.Field(typeof(CombatState), "_enemies").SetValue(backend.Combat, new List<Creature> { enemy });
        GameProducerFixture.Enemies = new[] { enemy };
        return (players[0], players[1], enemy);
    }
    private static void RecordModel(AbstractModel model, SourceSnapshot source)
    {
        backend.Descriptors[model] = new NativeAttributionBackend().Describe(model);
        if (model is CardModel or PowerModel or OrbModel)
            backend.Sources[IdentityCapture.Get(model, CaptureRuntime.Epoch).Identity] = source;
        else backend.NamedSources[model.Id.Entry] = source;
    }
    private static T GameCard<T>(Player owner, SourceSnapshot source) where T : CardModel
    {
        var card = Mutable<T>(typeof(T).Name);
        card.Owner = owner;
        RecordModel(card, source);
        return card;
    }
    private static T GamePower<T>(Creature owner, int amount, SourceSnapshot source) where T : PowerModel
    {
        var power = Mutable<T>(typeof(T).Name);
        AccessTools.Field(typeof(PowerModel), "_owner").SetValue(power, owner);
        AccessTools.Field(typeof(PowerModel), "_amount").SetValue(power, amount);
        RecordModel(power, source);
        return power;
    }
    private static Task PlayBody(CardModel card, Creature target)
    {
        var play = (CardPlay)RuntimeHelpers.GetUninitializedObject(typeof(CardPlay));
        AccessTools.Field(typeof(CardPlay), "<Card>k__BackingField").SetValue(play, card);
        AccessTools.Field(typeof(CardPlay), "<Target>k__BackingField").SetValue(play, target);
        AccessTools.Field(typeof(CardPlay), "<Player>k__BackingField").SetValue(play, card.Owner);
        return (Task)AccessTools.DeclaredMethod(card.GetType(), "OnPlay").Invoke(card, new object[] { null, play });
    }
    private static void SleightAndDefy()
    {
        var world = GameWorld();
        var supplied = GameCard<SleightOfFlesh>(world.Owner, A);
        var upgraded = GameCard<SleightOfFlesh>(world.Other, B);
        context.Complete(PlayBody(supplied, world.Owner.Creature));
        var initial = GameProducerFixture.Applications.Single();
        Check(initial.Power is SleightOfFleshPower && initial.Amount == 9, "Pinned base Sleight OnPlay supplies nine stacks");
        Same(initial.Producer.Source, A, "Actual supplying Sleight card enters its own source scope");
        AccessTools.DeclaredMethod(typeof(SleightOfFlesh), "OnUpgrade").Invoke(upgraded, null);
        context.Complete(PlayBody(upgraded, world.Other.Creature));
        Check(GameProducerFixture.Applications.Last().Amount == 13, "Pinned Sleight OnUpgrade and OnPlay supply thirteen stacks");
        var defySource = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 16, 1) });
        var defy = GameCard<Defy>(world.Owner, defySource);
        var mixture = SourceSnapshot.Create(epoch, new[] { new SourceShare(A[0].Destination, 9), new SourceShare(B[0].Destination, 13) });
        foreach (var variant in new[] { (Amount: 9, Source: A), (Amount: 13, Source: B), (Amount: 22, Source: mixture) })
        {
            var sleight = GamePower<SleightOfFleshPower>(world.Owner.Creature, variant.Amount, variant.Source);
            GameProducerFixture.Listener = sleight;
            GameProducerFixture.Blocked = 2;
            int before = backend.DamageGroups.Count;
            var pause = Pause(); DamageFixture.Pause = pause.Task;
            var wrapper = new ProbeModel("actual Defy wrapper");
            var task = wrapper.OnPlayWrapper(async () =>
            {
                PlayCapture.Started(defy, 0, 0, 1);
                await PlayBody(defy, world.Enemy);
                PlayCapture.Finished(defy);
            });
            var emitted = GameProducerFixture.Emissions.Last();
            Check(!task.IsCompleted && backend.OpenCalculations == 1, "Sleight result suspends inside actual Defy OnPlay");
            Check(ReferenceEquals(emitted.Play?.Card, defy) && emitted.Play.Token != 0, "Defy remains the active triggering play during Sleight's independent damage");
            Check(ReferenceEquals(emitted.Producer.Model, sleight) && emitted.Amount == variant.Amount && emitted.Props == ValueProp.Unpowered,
                "Actual Sleight listener emits its own entire amount as Unpowered damage");
            Same(GameProducerFixture.Applications.Last().Producer.Source, defySource, "Weak application occurs inside Defy's distinct source scope");
            RecordModel(sleight, defySource);
            pause.SetResult(1); context.Complete(task);
            Check(backend.DamageGroups.Count == before + 1 && backend.DamageGroups.Last().Packets.Single().Total == variant.Amount,
                "Each base/upgraded/stacked emission reports one complete actual result group");
            Check(backend.DamageGroups.Last().Packets.Single().Blocked == 2 && backend.DamageGroups.Last().Packets.Single().Unblocked == variant.Amount - 2,
                "Blocked plus HP damage conserves the emitted amount");
            Same(backend.DamageGroups.Last().Source, variant.Source, "Frozen Sleight suppliers exclude the active Defy and later source mutation");
            Check(backend.Begun.Last().Role == ProducerRole.Power && backend.Begun.Last().Segment == DamageSegment.Attributed && backend.DamageModifiers.Count == 0,
                "Independent Sleight output retains Power/Attributed with no attack modifiers");
        }
        Check(backend.Committed.Sum(packet => packet.Total) == 44 && backend.Fallback.Count == 0, "Base 9, upgraded 13 and stacked 22 emit exactly 44 captured points");
    }
    private static void SleightTriggerKinds()
    {
        var world = GameWorld();
        var sleight = GamePower<SleightOfFleshPower>(world.Owner.Creature, 9, A);
        GameProducerFixture.Listener = sleight;
        var potion = Mutable<WeakPotion>("WEAK_POTION"); potion.Owner = world.Owner; RecordModel(potion, B);
        context.Complete((Task)AccessTools.DeclaredMethod(typeof(WeakPotion), "OnUse").Invoke(potion, new object[] { null, world.Enemy }));
        var relic = Mutable<RedMask>("RED_MASK"); relic.Owner = world.Owner; RecordModel(relic, B);
        context.Complete(relic.BeforeSideTurnStart(null, CombatSide.Player, new[] { world.Owner.Creature }, (ICombatState)backend.Combat));
        var countdown = GamePower<CountdownPower>(world.Owner.Creature, 2, B);
        context.Complete(countdown.AfterSideTurnStart(CombatSide.Player, new[] { world.Owner.Creature }, (ICombatState)backend.Combat));
        var deathbringer = GameCard<Deathbringer>(world.Owner, B);
        context.Complete(PlayBody(deathbringer, null));
        Check(GameProducerFixture.Applications.Select(application => application.Power.GetType()).SequenceEqual(new[] { typeof(WeakPower), typeof(WeakPower), typeof(DoomPower), typeof(DoomPower), typeof(WeakPower) }),
            "Pinned potion, relic, Countdown and Deathbringer bodies deliver their real ordered debuffs");
        Check(GameProducerFixture.Applications.Select(application => application.Amount).SequenceEqual(new decimal[] { 3, 1, 2, 21, 1 }),
            "Pinned trigger amounts are independent of Sleight's nine-point output");
        Check(GameProducerFixture.Applications.Select(application => application.Producer.Role).SequenceEqual(new[] { ProducerRole.Potion, ProducerRole.Relic, ProducerRole.Power, ProducerRole.Card, ProducerRole.Card }),
            "Actual triggering model kinds establish distinct producer scopes");
        Check(GameProducerFixture.Applications.Take(3).All(application => application.Card == null)
            && GameProducerFixture.Applications.Skip(3).All(application => ReferenceEquals(application.Card, deathbringer)), "Null-card triggers and explicit Deathbringer source both reach the actual listener");
        foreach (var application in GameProducerFixture.Applications) Same(application.Producer.Source, B, "Triggering model source is B");
        Check(backend.DamageGroups.Count == 5 && backend.DamageGroups.Select(group => group.Calculation).Distinct().Count() == 5,
            "Deathbringer's Doom and Weak generate separate successful calculation groups alongside the other triggers");
        foreach (var group in backend.DamageGroups)
        {
            Same(group.Source, A, "Sleight supplier excludes potion/relic/power/Deathbringer trigger source");
            Check(group.Packets.Single().Total == 9 && group.Packets.Single().Kind == ResultKind.Outgoing, "Every actual independent Sleight proc captures nine outgoing points");
        }
        Check(GameProducerFixture.Emissions.All(emission => ReferenceEquals(emission.Producer.Model, sleight)) && backend.Committed.Sum(packet => packet.Total) == 45,
            "Five independent actual Sleight emissions capture exactly 45 points");
        int before = backend.Committed.Count;
        var weak = GamePower<WeakPower>(world.Enemy, 1, B);
        var buff = GamePower<StrengthPower>(world.Enemy, 1, B);
        var temporary = GamePower<DarkShacklesPower>(world.Enemy, 1, B);
        Check(temporary.GetTypeForAmount(1) == MegaCrit.Sts2.Core.Entities.Powers.PowerType.Debuff,
            "The temporary control is an actual debuff, so exclusion reaches the temporary-power condition");
        context.Complete(sleight.AfterPowerAmountChanged(null, weak, 0, world.Owner.Creature, deathbringer));
        context.Complete(sleight.AfterPowerAmountChanged(null, weak, 1, world.Other.Creature, deathbringer));
        context.Complete(sleight.AfterPowerAmountChanged(null, buff, 1, world.Owner.Creature, deathbringer));
        context.Complete(sleight.AfterPowerAmountChanged(null, temporary, 1, world.Owner.Creature, deathbringer));
        Check(backend.Committed.Count == before, "Actual listener rejects zero change, another applier, buff and temporary debuff");
    }
    private static void CacophonyDraws()
    {
        var world = GameWorld();
        var supplierCard = GameCard<Defy>(world.Owner, A);
        var drawn = GameCard<Defy>(world.Other, B);
        var cacophony = GamePower<CacophonyPower>(world.Owner.Creature, 7, A);
        Check(new NativeAttributionBackend().Describe(supplierCard).Slot == 0 && new NativeAttributionBackend().Describe(drawn).Slot == 1,
            "Cacophony's supplied source and drawn card belong to different roster players");
        FlowCapture.Current = Frame(drawn, B);
        for (int draw = 0; draw < 32; draw++) context.Complete(cacophony.AfterCardDrawn(null, drawn, draw % 2 == 0));
        Check(backend.Committed.Count == 0 && cacophony.DynamicVars.Cards.IntValue == 1, "Actual Cacophony counts all 32 other-player draws without an early emission");
        var wait = Pause(); GameProducerFixture.WaitTask = wait.Task;
        var task = cacophony.AfterCardDrawn(null, drawn, false);
        Check(!task.IsCompleted && cacophony.DynamicVars.Cards.IntValue == 33 && backend.Committed.Count == 0, "Thirty-third actual draw resets counter before the game's awaited delay");
        RecordModel(cacophony, B);
        wait.SetResult(1); context.Complete(task);
        Check(backend.Committed.Single().Total == 7 && backend.Committed.Single().Kind == ResultKind.Outgoing, "Actual Cacophony emits exactly seven outgoing points after its delay");
        Same(backend.DamageGroups.Single().Source, A, "Cacophony freezes its supplier before wait, excluding drawer and later supplier changes");
        Check(GameProducerFixture.Emissions.Single().Dealer == world.Owner.Creature
            && ReferenceEquals(GameProducerFixture.Emissions.Single().Producer.Model, cacophony), "Actual Cacophony dealer remains its owner while the drawn card belongs to the other player");
        Check(backend.Begun.Single().Role == ProducerRole.Power && backend.Begun.Single().Segment == DamageSegment.Attributed, "Cacophony's captured source remains attributed power damage");
        Same(FlowCapture.Current.Source, B, "Actual Cacophony async kickoff and completion preserve drawing caller");
    }
    private static void NestedDamage()
    {
        var world = GameWorld();
        var parentCard = GameCard<Defy>(world.Owner, A);
        var childCard = GameCard<Defy>(world.Other, B);
        var parentModifier = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 16, 1) });
        var childModifier = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 24, 1) });
        var retaliationSource = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 32, 1) });
        var parentStrength = GamePower<StrengthPower>(world.Owner.Creature, 3, parentModifier);
        var childStrength = GamePower<StrengthPower>(world.Other.Creature, 2, childModifier);
        var retaliation = GamePower<FlameBarrierPower>(world.Enemy, 4, retaliationSource);
        DamageCapture.Inspect = (_, dealer, props) => new(props == ValueProp.Move, false, dealer == world.Owner.Creature ? 3 : 2, null, false);
        var parentPause = Pause(); var childPause = Pause();
        Task nestedAttack = null;
        DamageOperation parentOperation = null, childOperation = null;
        DamageFixture.Results = new() { new(world.Enemy, ValueProp.Move) { UnblockedDamage = 9, BlockedDamage = 2 } };
        DamageFixture.Pause = parentPause.Task;
        DamageFixture.Modifiers = new[] { parentStrength }; DamageFixture.Bonus = 3;
        DamageFixture.DuringLive = () => parentOperation = DamageCapture.Current;
        DamageFixture.Nested = async () =>
        {
            DamageFixture.Results = new() { new(world.Enemy, ValueProp.Move) { UnblockedDamage = 6, BlockedDamage = 1 } };
            DamageFixture.Pause = childPause.Task;
            DamageFixture.Modifiers = new[] { childStrength }; DamageFixture.Bonus = 2;
            DamageFixture.DuringLive = () => childOperation = DamageCapture.Current;
            DamageFixture.Nested = async () =>
            {
                DamageFixture.Nested = null;
                DamageFixture.DuringLive = null;
                DamageFixture.Modifiers = Array.Empty<AbstractModel>(); DamageFixture.Bonus = 0;
                var hit = new DamageResult(world.Enemy, ValueProp.Move) { UnblockedDamage = 6, BlockedDamage = 1 };
                var task = retaliation.AfterDamageReceived(null, world.Enemy, hit, ValueProp.Move, world.Other.Creature, childCard);
                Check(ReferenceEquals(DamageCapture.Current, childOperation), "Actual retaliation kickoff restores exact child calculation before its await");
                await task;
                Check(ReferenceEquals(DamageCapture.Current, childOperation), "Awaited actual retaliation restores the child's immutable calculation");
            };
            var child = DamageFixture.Damage(null, new[] { world.Enemy }, 5, ValueProp.Move, world.Other.Creature, childCard, null);
            nestedAttack = child;
            Check(ReferenceEquals(DamageCapture.Current, parentOperation), "Nested canonical child kickoff restores exact parent operation");
            await child;
            Check(ReferenceEquals(DamageCapture.Current, parentOperation), "Awaited canonical child preserves parent calculation until its own result group");
        };
        var caller = DamageCapture.Current;
        var parent = DamageFixture.Damage(null, new[] { world.Enemy }, 8, ValueProp.Move, world.Owner.Creature, parentCard, null);
        Check(ReferenceEquals(DamageCapture.Current, caller) && backend.OpenCalculations == 3 && backend.Committed.Count == 0,
            "Parent, nested attack and actual retaliation suspend with three independent live calculations and restore outer caller");
        RecordModel(parentStrength, B); RecordModel(childStrength, A); RecordModel(retaliation, B);
        DamageFixture.Preview(world.Enemy, world.Owner.Creature, parentCard);
        Check(backend.DamageModifiers.Count == 2 && backend.Begun.Count == 3, "Nested preview cannot replace or supplement either live modifier batch");
        childPause.SetResult(1);
        context.Complete(nestedAttack);
        Check(!parent.IsCompleted && backend.OpenCalculations == 1 && backend.DamageGroups.Count == 2,
            "Both descendants finish while the parent retains its own suspended calculation");
        parentPause.SetResult(1); context.Complete(parent);
        Check(backend.DamageGroups.Count == 3 && backend.DamageGroups.Select(group => group.Calculation).Distinct().Count() == 3 && backend.Fallback.Count == 0,
            "Retaliation, child and parent seal three distinct complete groups exactly once");
        var groups = backend.DamageGroups;
        Same(groups[0].Source, retaliationSource, "Retaliation retains its actual power source across await");
        Same(groups[1].Source, B, "Child canonical explicit card retains its source");
        Same(groups[2].Source, A, "Parent canonical explicit card retains its source");
        Check(groups.Select(group => group.Packets.Single().Total).SequenceEqual(new[] { 4, 7, 11 })
            && groups[0].Packets.Single().Kind == ResultKind.Incoming, "Nested game-order results retain incoming retaliation and both outgoing actual amounts");
        var parentContribution = backend.DamageModifiers.Single(modifier => modifier.Calculation == groups[2].Calculation);
        var childContribution = backend.DamageModifiers.Single(modifier => modifier.Calculation == groups[1].Calculation);
        Same(parentContribution.Source, parentModifier, "Parent modifier's actual Strength instance source freezes before await");
        Same(childContribution.Source, childModifier, "Child modifier's actual Strength instance source freezes independently");
        Check(parentContribution.Amount == 3 && childContribution.Amount == 2
            && !backend.DamageModifiers.Any(modifier => modifier.Calculation == groups[0].Calculation), "Independent modifier amounts stay with their calculation; Unpowered retaliation has none");
    }
    private static void MultipleDamageTargets()
    {
        var world = GameWorld();
        var second = Creature(); var redirect = Creature();
        var card = GameCard<Defy>(world.Owner, A);
        var otherModifierSource = SourceSnapshot.Create(epoch, new[] { new SourceShare((epoch << 32) | 16, 1) });
        var firstStrength = GamePower<StrengthPower>(world.Owner.Creature, 2, B);
        var secondStrength = GamePower<StrengthPower>(world.Owner.Creature, 5, otherModifierSource);
        DamageFixture.ResultsByTarget = new()
        {
            [world.Enemy] = new() { new(world.Enemy, ValueProp.Move) { UnblockedDamage = 5, BlockedDamage = 2 }, new(redirect, ValueProp.Move) { UnblockedDamage = 1 } },
            [second] = new() { new(second, ValueProp.Move) { UnblockedDamage = 7, BlockedDamage = 4 } }
        };
        DamageFixture.ModifiersByTarget = new()
        {
            [world.Enemy] = (new[] { firstStrength }, 2),
            [second] = (new[] { secondStrength }, 5)
        };
        DamageCapture.Inspect = (_, _, _) => new(true, false, 0, null, false);
        var pause = Pause(); DamageFixture.Pause = pause.Task;
        var task = DamageFixture.Damage(null, new[] { world.Enemy, second }, 6, ValueProp.Move, world.Owner.Creature, card, null);
        Check(backend.Begun.Count == 1 && backend.OpenCalculations == 1 && backend.DamageGroups.Count == 0,
            "First original target suspends without prematurely enumerating or capturing the next target");
        DamageFixture.Preview(second, world.Owner.Creature, card);
        Check(backend.DamageModifiers.Count == 1, "Preview of the next target cannot enter the first target's modifier group");
        pause.SetResult(1); context.Complete(task);
        Check(backend.DamageGroups.Count == 2 && backend.DamageGroups[0].Packets.Length == 2 && backend.DamageGroups[1].Packets.Length == 1,
            "Two original targets create two groups while first-target redirection stays in one group");
        Check(backend.DamageGroups.Select(group => group.Packets.Sum(packet => packet.Total)).SequenceEqual(new[] { 8, 11 })
            && task.Result.Sum(result => result.TotalDamage) == 19 && DamageFixture.Enumerated == 19 && DamageFixture.LateEnumerated == 19,
            "Original target amounts, redirected results and aggregate late enumeration conserve nineteen points exactly once");
        Check(backend.DamageGroups[0].Calculation != backend.DamageGroups[1].Calculation && backend.DamageModifiers.Count == 2,
            "Each original target owns its distinct modifier calculation");
        var first = backend.DamageModifiers.Single(modifier => modifier.Calculation == backend.DamageGroups[0].Calculation);
        var last = backend.DamageModifiers.Single(modifier => modifier.Calculation == backend.DamageGroups[1].Calculation);
        Same(first.Source, B, "First target retains its actual modifier source");
        Same(last.Source, otherModifierSource, "Second target retains a different actual modifier source");
        Check(first.Amount == 2 && last.Amount == 5 && backend.Fallback.Count == 0, "Different target modifiers stay in their own complete accepted groups");
    }
    private static void ActualPoisonAndRolling()
    {
        var world = GameWorld();
        var trigger = GameCard<Defy>(world.Owner, B);
        var poison = GamePower<PoisonPower>(world.Enemy, 3, A);
        var accelerant = GamePower<AccelerantPower>(world.Owner.Creature, 1, B);
        var ownerPowers = (List<PowerModel>)AccessTools.Field(typeof(Creature), "_powers").GetValue(world.Owner.Creature);
        ownerPowers.Add(accelerant);
        var remaining = new List<int>();
        GameProducerFixture.Decremented = power => { remaining.Add(power.Amount); RecordModel(power, B); };
        FlowCapture.Current = Frame(trigger, B);
        context.Complete(poison.AfterSideTurnStart(CombatSide.Enemy, new[] { world.Enemy }, (ICombatState)backend.Combat));
        Check(GameProducerFixture.Emissions.Select(emission => emission.Amount).SequenceEqual(new decimal[] { 3, 2 })
            && remaining.SequenceEqual(new[] { 2, 1 }), "Actual Poison turn callback and Trigger loop use Accelerant and decrement between two ticks");
        Same(backend.DamageGroups[0].Source, A, "First actual Poison tick captures its first supplied snapshot");
        Same(backend.DamageGroups[1].Source, B, "Second actual Poison tick refreshes the changed snapshot instead of reusing Trigger entry source");
        Check(backend.Committed.Take(2).All(packet => packet.Kind == ResultKind.Outgoing)
            && backend.Begun.Take(2).All(begin => begin.Role == ProducerRole.Power && begin.Segment == DamageSegment.Attributed), "Actual Poison ticks preserve outgoing attributed power classification");
        ownerPowers.Clear();
        GameProducerFixture.Applied = (power, target, source) =>
        {
            ((List<PowerModel>)AccessTools.Field(typeof(Creature), "_powers").GetValue(target)).Add(power);
            RecordModel(power, source);
        };
        var outbreak = GameCard<Outbreak>(world.Owner, B);
        context.Complete(PlayBody(outbreak, null));
        Check(GameProducerFixture.Applications.Single().Power is PoisonPower && GameProducerFixture.Applications.Single().Amount == 9
            && ReferenceEquals(GameProducerFixture.Applications.Single().Card, outbreak), "Actual Outbreak applies its nine-point Poison with the explicit supplying card");
        Check(backend.DamageGroups.Count == 3 && backend.DamageGroups.Last().Packets.Single().Total == 9
            && GameProducerFixture.Emissions.Last().Producer.Model is PoisonPower, "Actual Outbreak directly invokes the installed Poison Trigger producer after application");
        Same(backend.DamageGroups.Last().Source, B, "Outbreak-supplied Poison retains its captured supplier");
        Check(backend.Begun.Last().Segment == DamageSegment.Attributed && world.Enemy.GetPower<PoisonPower>().Amount == 8,
            "Actual Poison emitted during Outbreak remains attributed and decrements after the hit");
        DamageFixture.Results = new() { new(world.Enemy, ValueProp.Move) { UnblockedDamage = 2 } };
        var unrelated = GameCard<Defy>(world.Owner, A);
        context.Complete(DamageFixture.Damage(null, new[] { world.Enemy }, 2, ValueProp.Move, world.Owner.Creature, unrelated, null));
        Same(backend.DamageGroups.Last().Source, A, "An unrelated hit against the actually poisoned target retains its canonical card source");
        Check(backend.Begun.Last().Role == ProducerRole.Card && backend.Begun.Last().Segment == DamageSegment.Direct, "Target Poison cannot replace unrelated explicit card provenance");
        var rolling = GamePower<RollingBoulderPower>(world.Owner.Creature, 12, A);
        var second = Creature();
        var pause = Pause(); DamageFixture.Pause = pause.Task;
        FlowCapture.Current = ProducerFrame.Barrier;
        int before = backend.DamageGroups.Count;
        var task = (Task<IEnumerable<DamageResult>>)AccessTools.DeclaredMethod(typeof(RollingBoulderPower), "DoDamage")
            .Invoke(rolling, new object[] { null, new[] { world.Enemy, second } });
        Check(ReferenceEquals(task, DamageFixture.OriginalTask) && ReferenceEquals(FlowCapture.Current, ProducerFrame.Barrier),
            "Actual independent Rolling Boulder callback returns its boundary Task unchanged and restores the absent caller scope");
        RecordModel(rolling, B);
        pause.SetResult(1); context.Complete(task);
        Check(backend.DamageGroups.Count == before + 2 && task.Result.Sum(result => result.TotalDamage) == 24,
            "Actual Rolling Boulder callback emits twelve points to each original target");
        foreach (var group in backend.DamageGroups.Skip(before)) Same(group.Source, A, "Rolling Boulder freezes its own supplier without inherited context across target suspension");
        Check(backend.Begun.TakeLast(2).All(begin => begin.Role == ProducerRole.Power && begin.Segment == DamageSegment.Attributed)
            && backend.Fallback.Count == 0, "Independent callback retains attributed power capture for both complete groups");
    }
    private static void SentryAndFrost()
    {
        var world = GameWorld();
        var sentry = GamePower<SentryModePower>(world.Other.Creature, 2, A);
        GameProducerFixture.Created = card => backend.Descriptors[card] = new NativeAttributionBackend().Describe(card);
        context.Complete(sentry.BeforeHandDraw(world.Owner, null, (ICombatState)backend.Combat));
        Check(GameProducerFixture.Generated.Count == 0, "Actual Sentry rejects another player's before-draw event");
        var wait = Pause(); GameProducerFixture.WaitTask = wait.Task;
        var generation = sentry.BeforeHandDraw(world.Other, null, (ICombatState)backend.Combat);
        Check(!generation.IsCompleted && GameProducerFixture.Generated.Count == 1, "Actual Sentry creates its first Sweeping Gaze before awaiting pile insertion");
        RecordModel(sentry, B);
        wait.SetResult(1); context.Complete(generation);
        Check(GameProducerFixture.Generated.Count == 2 && GameProducerFixture.Generated.All(card => card is SweepingGaze && ReferenceEquals(card.Owner, world.Other)),
            "Actual Sentry creates two distinct Sweeping Gaze instances for its physical owner");
        Check(!ReferenceEquals(GameProducerFixture.Generated[0], GameProducerFixture.Generated[1]), "Separate Sentry loop iterations produce separate card identities");
        foreach (var card in GameProducerFixture.Generated)
        {
            Same(FlowCapture.Source(card, CaptureRuntime.Epoch), A, "Both generated cards preserve Sentry's earlier supplier across the awaited insertion");
            Check(IdentityCapture.Get(card, CaptureRuntime.Epoch).Generation == GenerationState.GeneratedRecorded, "Production generation capture records each actual generated card identity");
        }
        var supplyingCard = GameCard<Defy>(world.Owner, A);
        var triggeringCard = GameCard<Defy>(world.Other, B);
        var frost = Mutable<FrostOrb>("FROST_ORB"); frost.Owner = world.Other; RecordModel(frost, B);
        var hibernate = GamePower<HibernatePower>(world.Other.Creature, 1, B);
        ((List<PowerModel>)AccessTools.Field(typeof(Creature), "_powers").GetValue(world.Other.Creature)).Add(hibernate);
        FlowCapture.Current = Frame(supplyingCard, A);
        CommandCapture.OrbChanneled(backend.Combat, frost);
        var blockWait = Pause(); CommandFixture.Pause = blockWait.Task;
        FlowCapture.Current = Frame(triggeringCard, B);
        var passive = frost.Passive(null, null);
        Check(!passive.IsCompleted && backend.CommandEvents.Count == 0, "Actual Frost passive suspends at the first canonical-shaped block command");
        RecordModel(frost, B);
        blockWait.SetResult(1); context.Complete(passive);
        Check(backend.CommandEvents.Select(item => (item.Kind, item.Amount, item.Slot)).SequenceEqual(new[] { ("block", 2, 1), ("block", 2, 0) }),
            "Actual Frost/Hibernate passive produces its two-point block first for the owner, then the other roster player");
        foreach (var block in backend.CommandEvents) Same(block.Source, A, "Both receiver block events retain the channel supplier despite trigger and later orb-source change");
        Same(FlowCapture.Current.Source, B, "Actual Frost async scope restores the triggering caller after cross-player block commands");
    }
    private static void DamageGroupTiming()
    {
        var enemy = Creature(); var player = Creature(true, false, 0); var card = Card();
        var error = new InvalidOperationException("later game callback failure");
        DamageFixture.Results = new() { new(enemy, ValueProp.Move) { UnblockedDamage = 3, BlockedDamage = 2, OverkillDamage = 100, WasTargetKilled = true } };
        DamageFixture.AfterGroup = () =>
        {
            Check(backend.Committed.Count == 1 && backend.OpenCalculations == 0, "Complete killed-target group is already reported before a later callback runs");
            throw error;
        };
        bool observed = false;
        try { context.Complete(Damage(enemy, player, card)); }
        catch (InvalidOperationException actual) { observed = ReferenceEquals(actual, error); }
        Check(observed && backend.Committed.Single().Total == 5 && backend.Fallback.Count == 0 && DamageFixture.LateEnumerated == 0,
            "Later game exception retains the completed five-point result once; positive overkill is excluded");
        var result = DamageFixture.Results;
        DamageFixture.AfterGroup = () =>
        {
            Check(backend.Committed.Count == 2 && backend.OpenCalculations == 0, "Second complete group is reported before combat replacement");
            backend.Combat = RuntimeHelpers.GetUninitializedObject(typeof(CombatState));
            CaptureRuntime.Register(backend, ++epoch, backend.Combat);
        };
        var task = Damage(enemy, player, card);
        context.Complete(task);
        Check(ReferenceEquals(task.Result, result) && backend.Committed.Count == 2 && backend.Fallback.Count == 0 && DamageFixture.LateEnumerated == 5,
            "Later aggregate enumeration and builder completion preserve the original result after combat replacement without duplicate reporting");
    }
    private static void Classifications()
    {
        object player = new(), other = new(), enemy = new(), osty = new();
        var p = new CreatureDescriptor(true, false, 0, backend.Combat); var e = new CreatureDescriptor(false, false, 4, backend.Combat); var o = new CreatureDescriptor(false, true, 0, backend.Combat);
        Check(DamageCapture.Classify(o, osty, true, e, enemy) == ResultKind.OstyDealt, "Osty dealer precedence");
        Check(DamageCapture.Classify(o, osty, false, o, osty) == ResultKind.OstyDealt, "Osty dealer takes precedence even when receiver is Osty");
        Check(DamageCapture.Classify(p, player, true, o, osty) == ResultKind.OstyAbsorbed, "Osty receiver branch");
        Check(DamageCapture.Classify(p, player, false, p, player) == ResultKind.SelfDamage, "Actual same-creature damage is self damage");
        Check(DamageCapture.Classify(default, null, true, p, player) == ResultKind.SelfDamage, "Null dealer plus canonical card is self damage");
        Check(DamageCapture.Classify(default, null, false, p, player) == ResultKind.Incoming, "Royal Poison producer cannot turn incoming into self damage");
        Check(DamageCapture.Classify(p, other, true, p, player) == ResultKind.Incoming, "Another player dealer is incoming");
        Check(DamageCapture.Classify(default, null, false, e, enemy) == ResultKind.Outgoing, "Missing producer leaves outgoing classification");
    }
    internal static T Mutable<T>(string id) where T : AbstractModel
    {
        var model = (T)RuntimeHelpers.GetUninitializedObject(typeof(T));
        AccessTools.Field(typeof(AbstractModel), "<IsMutable>k__BackingField").SetValue(model, true);
        AccessTools.Field(typeof(AbstractModel), "<Id>k__BackingField").SetValue(model, new ModelId("TEST", id));
        return model;
    }
    private static RunState Roster(params Player[] players)
    {
        var run = (RunState)RuntimeHelpers.GetUninitializedObject(typeof(RunState));
        AccessTools.Field(typeof(RunState), "_players").SetValue(run, players.ToList());
        AccessTools.Field(typeof(RunState), "_currentRooms").SetValue(run, new List<AbstractRoom>());
        RunContext.CaptureRunPlayers(run);
        return run;
    }
    private static void ActualModelAdapters()
    {
        var first = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        var second = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        var stranger = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        var run = Roster(first, second);
        var adapter = new NativeAttributionBackend();
        Check(adapter.CurrentCombat == null, "No current room means no active combat reference");
        var room = (CombatRoom)RuntimeHelpers.GetUninitializedObject(typeof(CombatRoom));
        AccessTools.Field(typeof(CombatRoom), "<CombatState>k__BackingField").SetValue(room, backend.Combat);
        var rooms = new List<AbstractRoom> { room };
        AccessTools.Field(typeof(RunState), "_currentRooms").SetValue(run, rooms);
        Check(ReferenceEquals(adapter.CurrentCombat, backend.Combat), "Actual retained run resolves the exact CombatRoom.CombatState object");
        var replacement = RuntimeHelpers.GetUninitializedObject(typeof(CombatState));
        var replacementRoom = (CombatRoom)RuntimeHelpers.GetUninitializedObject(typeof(CombatRoom));
        AccessTools.Field(typeof(CombatRoom), "<CombatState>k__BackingField").SetValue(replacementRoom, replacement);
        rooms[0] = replacementRoom;
        Check(ReferenceEquals(adapter.CurrentCombat, replacement), "Room replacement is observed through the same actual RunState");
        rooms[0] = room;
        var card = Mutable<MegaCrit.Sts2.Core.Models.Cards.StrikeIronclad>("A");
        Check(adapter.Describe(card).Slot == 4, "Actual card with missing owner credits TEAM");
        card.Owner = stranger;
        Check(adapter.Describe(card).Slot == 4, "Actual card with unregistered owner credits TEAM");
        card.Owner = null;
        card.Owner = second;
        Check(adapter.Describe(card).Slot == 1, "Actual card preserves valid roster slot");
        var potion = Mutable<MegaCrit.Sts2.Core.Models.Potions.FlexPotion>("POTION");
        var relic = Mutable<MegaCrit.Sts2.Core.Models.Relics.Anchor>("RELIC");
        Check(adapter.Describe(potion).Slot == 4 && adapter.Describe(relic).Slot == 4, "Actual missing potion/relic owners credit TEAM");
        potion.Owner = first; relic.Owner = second;
        Check(adapter.Describe(potion).Slot == 0 && adapter.Describe(relic).Slot == 1, "Actual potion/relic roster ownership remains distinct");
        var creature = Creature(true, false, 0);
        AccessTools.Field(typeof(Creature), "<Player>k__BackingField").SetValue(creature, first);
        AccessTools.Field(typeof(Creature), "_powers").SetValue(creature, new List<PowerModel>());
        creature.CombatState = (ICombatState)backend.Combat;
        var strength = Mutable<StrengthPower>("STRENGTH_POWER");
        AccessTools.Field(typeof(PowerModel), "_owner").SetValue(strength, creature);
        backend.Descriptors[strength] = adapter.Describe(strength);
        FlowCapture.Current = Frame(card, A);
        strength.SetAmount(-5);
        Check(backend.PowerEvents.Count == 0, "Actual initial SetAmount before membership is not separately booked");
        creature.ApplyPowerInternal(strength);
        Check(backend.PowerEvents.Single().After == -5 && backend.PowerEvents.Single().Kind == "attach", "Actual signed first attachment records accepted amount");
        FlowCapture.Current = Frame(card, B);
        strength.SetAmount(1_500_000_000);
        Check(backend.PowerEvents.Last().Before == -5 && backend.PowerEvents.Last().After == 999_999_999, "Actual game clamp is observed instead of requested SetAmount");
        var error = new InvalidOperationException("actual presentation notification");
        AccessTools.Field(typeof(PowerModel), "DisplayAmountChanged").SetValue(strength, (Action)(() => throw error));
        try { strength.SetAmount(7); } catch (InvalidOperationException actual) { Check(ReferenceEquals(error, actual), "Actual notification exception object preserved"); }
        Check(backend.PowerEvents.Last().After == 7, "Actual accepted mutation preceding notification exception captured");
        AccessTools.Field(typeof(PowerModel), "DisplayAmountChanged").SetValue(strength, null);
        creature.RemovePowerInternal(strength);
        Same(FlowCapture.Source(strength, CaptureRuntime.Epoch), B, "Actual removal retains last immutable source");
    }
    private static void CaptureStatusFailures()
    {
        var target = Creature(); var dealer = Creature(true, false, 0);
        var strength = Mutable<StrengthPower>("A");
        AccessTools.Field(typeof(PowerModel), "_owner").SetValue(strength, dealer);
        AccessTools.Field(typeof(PowerModel), "_amount").SetValue(strength, 3);
        backend.Descriptors[strength] = new(CaptureKind.PowerInstance, ProducerRole.Power, "A", 2, 0);
        DamageFixture.Modifiers = new[] { strength }; DamageFixture.Bonus = 3;
        DamageCapture.Inspect = (_, _, _) => new(true, false, 3, null, false);
        backend.Failure = "DamageModifier"; backend.FailureCount = 1;
        DamageFixture.Results = new() { new(target, ValueProp.Move) { UnblockedDamage = 7, BlockedDamage = 2 } };
        context.Complete(Damage(target, dealer));
        Check(backend.Calls.Contains("DamageModifier") && backend.Fallback.Count == 1 && backend.Committed.Count == 0, "Actual modifier status zero invalidates entire group");
        Check(DamageFixture.HookCalls == 1, "Rejected modifier capture still calls original Hook exactly once");
        DamageFixture.Modifiers = Array.Empty<AbstractModel>(); DamageFixture.Bonus = 0;
        var incoming = Creature(true, false, 1); var enemy = Creature(); var weak = new ProbePower("A");
        foreach (var failure in new[] { "DamageEnemyHit", "DamageWeak", "DamageBegin" })
        {
            int before = backend.Fallback.Count;
            backend.Failure = failure; backend.FailureCount = 1;
            DamageCapture.Inspect = (_, _, _) => new(false, true, -2, weak, false);
            DamageFixture.Results = new() { new(incoming, ValueProp.Move) { UnblockedDamage = 6 } };
            context.Complete(Damage(incoming, enemy));
            Check(backend.Fallback.Count == before + 1 && backend.Fallback.Last().Kind == ResultKind.Incoming, "Capture rejection preserves incoming classification: " + failure);
            Check(backend.Fallback.Last().WeakPrevented == 2, "Capture rejection preserves independently observed Weak numerical evidence: " + failure);
        }
        backend.Failure = null;
        uint kraneSlots = 1u << 1;
        DamageCapture.Inspect = (_, _, _) => new(false, true, -2, weak, false, kraneSlots);
        var redirected = Creature(true, false, 0);
        DamageFixture.Results = new() { new(incoming, ValueProp.Move) { UnblockedDamage = 6 }, new(redirected, ValueProp.Move) { UnblockedDamage = 6 } };
        var pause = Pause(); DamageFixture.Pause = pause.Task;
        var task = Damage(incoming, enemy);
        kraneSlots = 1u << 0;
        pause.SetResult(1); context.Complete(task);
        Check(backend.Committed.TakeLast(2).Select(packet => packet.WeakPrevented).SequenceEqual(new[] { 4, 2 }), "Suspended Krane mutation cannot change frozen per-receiver-slot formula inputs");
        var dirtyStrength = new ProbePower("B");
        IdentityCapture.Get(dirtyStrength, CaptureRuntime.Epoch).Dirty = true;
        DamageCapture.Inspect = (_, _, _) => new(false, true, -2, weak, false, 0, dirtyStrength);
        DamageFixture.Pause = Task.CompletedTask;
        int hits = backend.Calls.Count(call => call == "DamageEnemyHit");
        context.Complete(Damage(incoming, enemy));
        Check(backend.Calls.Count(call => call == "DamageEnemyHit") == hits && backend.Fallback.Last().WeakPrevented == 2, "Dirty Strength blocks old native reduction capture without losing observed Weak evidence");
    }
    private static void CommandSources()
    {
        var player = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        var receiverPlayer = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        Roster(player, receiverPlayer);
        var receiver = Creature(true, false, 1);
        AccessTools.Field(typeof(Creature), "<Player>k__BackingField").SetValue(receiver, receiverPlayer);
        receiver.CombatState = (ICombatState)backend.Combat;
        var card = Card();
        var play = new CardPlay { Card = card, Player = player, Target = null, ResultPile = default, Resources = default, IsAutoPlay = false, PlayIndex = 0, PlayCount = 1 };
        FlowCapture.Current = Frame(new ProbeModel("B", ProducerRole.Power), B, ProducerRole.Power);
        var pause = Pause(); CommandFixture.Pause = pause.Task;
        var block = CommandFixture.GainBlock(receiver, 4, ValueProp.Move, play);
        Check(backend.CommandEvents.Count == 0 && CommandCapture.Current.Epoch.Sequence == 0, "Block kickoff restores caller while history remains pending");
        var generated = new ProbeModel("COMMAND_GENERATED");
        CommandFixture.During = () => ProvenanceCapture.Generated(generated);
        pause.SetResult(1); context.Complete(block);
        Same(CommandFixture.Frozen, A, "Actual block command card source survives suspension");
        Same(backend.CommandEvents.Single().Source, A, "Block history credits frozen source, not later listener");
        Check(backend.CommandEvents.Single().Slot == 1 && block.Result == 4, "Block pool receiver remains physical ally slot and original Task result unchanged");
        Same(FlowCapture.Source(generated, CaptureRuntime.Epoch), A, "Command scope forwards generated-card supplier");
        CommandFixture.During = null;
        var dexterity = Mutable<DexterityPower>("B");
        AccessTools.Field(typeof(PowerModel), "_owner").SetValue(dexterity, receiver);
        AccessTools.Field(typeof(PowerModel), "_amount").SetValue(dexterity, 2);
        backend.Descriptors[dexterity] = new(CaptureKind.PowerInstance, ProducerRole.Power, "B", 2, 1);
        CommandCapture.BlockModifierPostfix(12, new(CaptureRuntime.Epoch, 10), receiver, ValueProp.Move, null, null, new[] { dexterity });
        Same(backend.CommandEvents.Last().Source, B, "Actual block modifier instance source retained");
        Check(backend.CommandEvents.Last().Kind == "block-modifier" && backend.CommandEvents.Last().Amount == 2, "Original additive block decomposition preserved");
        pause = Pause(); CommandFixture.Pause = pause.Task;
        int count = backend.CommandEvents.Count;
        var forge = CommandFixture.Forge(3, player, card);
        Check(backend.CommandEvents.Count == count + 1 && backend.CommandEvents.Last().Kind == "forge" && !forge.IsCompleted, "Forge reporting retains original async kickoff timing");
        pause.SetResult(1); context.Complete(forge); Same(CommandFixture.Frozen, A, "Forge explicit source persists through internal await");
        pause = Pause(); CommandFixture.Pause = pause.Task;
        var summon = CommandFixture.Summon(null, player, 5, null);
        Same(backend.CommandEvents.Last().Source, B, "Null-source summon captures explicit current producer");
        Check(backend.CommandEvents.Last().Amount == 5 && backend.CommandEvents.Last().Slot == 0, "Summon keeps requested HP policy and physical owner");
        pause.SetResult(1); context.Complete(summon); Same(CommandFixture.Frozen, B, "Summon supplier persists to internal generated powers");
        CommandFixture.Pause = Task.CompletedTask; backend.Failure = "Capture"; backend.FailureCount = 1;
        context.Complete(CommandFixture.Forge(2, player, card));
        Same(backend.CommandEvents.Last().Source, SourceSnapshot.Unknown(epoch), "Failed explicit source capture retains positive Forge amount on Unknown");
        CommandCapture.BuffPrefix(new ProbePower("A"), 8, out var buff);
        CommandCapture.BuffPostfix(1, buff, receiver);
        Check(backend.CommandEvents.Last().Kind == "buff" && backend.CommandEvents.Last().Amount == 7, "Defensive buff delta keeps existing nonnegative formula");
        Same(backend.CommandEvents.Last().Source, A, "Defensive buff uses actual power snapshot");
    }
    private static void DoomBatches()
    {
        var first = Creature(); var second = Creature();
        var observations = new Dictionary<object, DoomObservation> { [first] = new(true, 11, null, backend.Combat), [second] = new(true, 7, null, backend.Combat) };
        DoomCapture.Inspect = creature => observations[creature];
        var pause = Pause(); DoomFixture.OriginalTask = pause.Task;
        bool nested = false;
        DoomFixture.During = () =>
        {
            if (nested) return;
            nested = true;
            Check(backend.DoomCommitted.Count == 0, "Doom cannot commit before invoking original kickoff");
            Check(ReferenceEquals(DoomFixture.DoomKill(new[] { second }), pause.Task), "Nested Doom preserves original Task identity");
        };
        var task = DoomFixture.DoomKill(new[] { first });
        Check(ReferenceEquals(task, pause.Task) && backend.DoomCommitted.Count == 2 && backend.DoomCommitted.Distinct().Count() == 2, "Nested Doom batches commit separately after original kickoff");
        Check(backend.DoomTargets.Select(t => t.Hp).SequenceEqual(new[] { 11, 7 }), "Doom freezes entry HP scalars in target order");
        pause.SetResult(1); context.Complete(task);
        DoomFixture.During = null; DoomFixture.OriginalTask = Task.CompletedTask;
        foreach (var failure in new[] { "DoomBegin", "DoomTarget", "DoomComplete" })
        {
            int before = backend.Fallback.Count, commits = backend.DoomCommitted.Count;
            backend.Failure = failure; backend.FailureCount = 1;
            context.Complete(DoomFixture.DoomKill(new[] { first, second }));
            Check(backend.Fallback.Skip(before).Sum(p => p.Total) == 18 && backend.DoomCommitted.Count == commits, "Incomplete Doom batch falls back once without partial commit: " + failure);
        }
        backend.Failure = null;
        var overflow = Enumerable.Range(0, DoomCapture.MaxTargets + 2).Select(_ => Creature()).ToArray();
        foreach (var creature in overflow) observations[creature] = new(true, int.MaxValue, null, backend.Combat);
        int baseline = backend.Fallback.Count;
        context.Complete(DoomFixture.DoomKill(overflow));
        Check(backend.Fallback.Skip(baseline).Sum(p => (long)p.Total) == (long)overflow.Length * int.MaxValue, "Doom target overflow preserves aggregate HP as representable Unknown chunks");
        var error = new InvalidOperationException("synchronous Doom kickoff failure"); DoomFixture.Error = error;
        baseline = backend.Fallback.Count; int priorCommits = backend.DoomCommitted.Count;
        try { DoomFixture.DoomKill(new[] { first }); } catch (InvalidOperationException actual) { Check(ReferenceEquals(error, actual), "Doom preserves original synchronous exception"); }
        Check(backend.Fallback.Count == baseline && backend.DoomCommitted.Count == priorCommits && backend.DoomBatches.Count == 0, "Synchronous original throw aborts Doom without synthetic commit");
        DoomFixture.Error = null; DoomFixture.OriginalTask = Task.FromException(error);
        task = DoomFixture.DoomKill(new[] { first });
        Check(backend.DoomCommitted.Count == priorCommits + 1, "Asynchronously faulted Doom Task retains approved synthetic kickoff limitation");
        try { context.Complete(task); } catch (InvalidOperationException actual) { Check(ReferenceEquals(error, actual), "Original async Doom exception preserved"); }
    }
    private static void OrbAndOsty()
    {
        var orb = new ProbeModel("ORB", ProducerRole.Orb, 1);
        FlowCapture.Current = Frame(new ProbeModel("A"), A);
        CommandCapture.OrbChanneled(backend.Combat, orb);
        Same(orb.Passive(), A, "Cross-player orb retains channeling supplier");
        backend.Failure = "OrbChanneled"; backend.FailureCount = 1;
        FlowCapture.Current = Frame(new ProbeModel("B"), B);
        CommandCapture.OrbChanneled(backend.Combat, orb);
        Check(orb.Passive().Epoch == 0, "Failed channel registration cannot read an older source for that orb");
        var osty = Creature(false, true, 0); var enemy = Creature();
        DamageFixture.Results = new() { new(enemy, ValueProp.Move) { UnblockedDamage = 5 } };
        FlowCapture.Current = Frame(new ProbePower("A"), A, ProducerRole.Power);
        context.Complete(Damage(enemy, osty));
        Same(backend.Begun.Last().Source, SourceSnapshot.Unknown(epoch), "OstyDealt without canonical card excludes ambient producer");
        Check(backend.Committed.Last().Kind == ResultKind.OstyDealt, "Missing canonical card leaves specialized Osty classification");
        var strength = GamePower<StrengthPower>(osty, 3, B);
        AccessTools.Field(typeof(Creature), "_powers").SetValue(osty, new List<PowerModel> { strength });
        DamageFixture.Modifiers = new[] { strength }; DamageFixture.Bonus = 3;
        DamageCapture.Inspect = AccessTools.DeclaredMethod(typeof(DamageCapture), "InspectGame").CreateDelegate<Func<Creature, Creature, ValueProp, DamageEvidence>>();
        context.Complete(Damage(enemy, osty, Card()));
        Same(backend.Begun.Last().Source, A, "Explicit Osty attack card uses canonical instance snapshot");
        Check(backend.Committed.Last().Kind == ResultKind.OstyDealt && backend.DamageModifiers.Count == 0,
            "Production damage eligibility excludes OstyDealt from decomposition despite explicit card and supplied modifier list");
        var card = new ProbeModel("A");
        context.Complete(card.OnPlayWrapper(() =>
        {
            PlayCapture.Started(card, 0, 0, 1);
            ulong token = PlayCapture.Current.Token;
            CommandCapture.Killed(osty);
            Check(backend.Calls.Last() == "OstyKilled:" + token, "Osty death subtracts only matching-owner explicit active play");
            CommandCapture.Killed(Creature(false, true, 1));
            Check(backend.Calls.Last() == "OstyKilled:0", "Another owner's play never substitutes for Osty cleanup");
            PlayCapture.Finished(card);
            return Task.CompletedTask;
        }));
    }
    private static void PoisonTicks()
    {
        var owner = Creature(); var other = Creature();
        var poison = new ProbePower("A") { Amount = 3 };
        backend.Descriptors[poison] = new(CaptureKind.PowerInstance, ProducerRole.Power, "A", 2, 4, owner, true);
        var identity = IdentityCapture.Get(poison, CaptureRuntime.Epoch);
        backend.Sources[identity.Identity] = A;
        FlowCapture.Current = new(CaptureRuntime.Epoch, poison, A, ProducerRole.Power, DamageSegment.Attributed, true);
        DamageFixture.Results = new() { new(owner, ValueProp.Unpowered) { UnblockedDamage = 3 } };
        context.Complete(Damage(owner));
        Same(backend.Begun.Last().Source, A, "First Poison canonical tick captures actual instance duration source");
        poison.Amount = 2; backend.Sources[identity.Identity] = B;
        context.Complete(Damage(owner));
        Same(backend.Begun.Last().Source, B, "Later Poison canonical tick refreshes mixture after decrement");
        poison.Amount = 0;
        context.Complete(Damage(owner));
        Same(backend.Begun.Last().Source, SourceSnapshot.Unknown(epoch), "Positive damage with zero actual Poison duration cannot borrow retained old source");
        poison.Amount = 2;
        DamageFixture.Results = new() { new(other, ValueProp.Unpowered) { UnblockedDamage = 3 } };
        context.Complete(Damage(other));
        Same(backend.Begun.Last().Source, SourceSnapshot.Unknown(epoch), "Poison source cannot attach to an unrelated target");
        backend.Failure = "SourceWeight"; backend.FailureCount = 1;
        DamageFixture.Results = new() { new(owner, ValueProp.Unpowered) { UnblockedDamage = 3 } };
        context.Complete(Damage(owner));
        Same(backend.Begun.Last().Source, SourceSnapshot.Unknown(epoch), "Missing positive Poison weights preserve outgoing damage on Unknown");
    }
    private static void StaleCommand()
    {
        var receiver = Creature(true, false, 0);
        var card = Card(); var player = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        var play = new CardPlay { Card = card, Player = player, Target = null, ResultPile = default, Resources = default, IsAutoPlay = false, PlayIndex = 0, PlayCount = 1 };
        var pause = Pause(); CommandFixture.Pause = pause.Task;
        var task = CommandFixture.GainBlock(receiver, 4, ValueProp.Move, play);
        CommandFixture.During = () =>
        {
            var child = new ProbeModel("A");
            Check(child.Synchronous().Epoch == 0, "Independent producer reached from stale operation retains unavailable old-epoch barrier");
            ProvenanceCapture.Generated(child);
        };
        var nextCombat = RuntimeHelpers.GetUninitializedObject(typeof(CombatState)); backend.Combat = nextCombat;
        CaptureRuntime.Register(backend, ++epoch, nextCombat);
        FlowCapture.Current = ProducerFrame.Barrier;
        int before = backend.Calls.Count;
        pause.SetResult(1); context.Complete(task);
        Check(backend.CommandEvents.Count == 0 && backend.Calls.Count == before, "Stale resumed command and nested generation make no native calls in replacement combat");
    }
    private static void PreSetupGeneration()
    {
        var generated = new ProbeModel("A");
        CommandCapture.SetupPrefix();
        int calls = backend.Calls.Count;
        ProvenanceCapture.GeneratedPrefix(new object[] { backend.Combat, generated, null });
        Check(backend.Calls.Count == calls, "Generation inside setup makes no pre-epoch native call");
        CaptureRuntime.Register(backend, ++epoch, backend.Combat);
        Check(IdentityCapture.Get(generated, CaptureRuntime.Epoch).Generation == GenerationState.GeneratedUnavailable, "Pre-epoch generated identity cannot become an ordinary named card");
        Same(FlowCapture.Source(generated, CaptureRuntime.Epoch), SourceSnapshot.Unknown(epoch), "Missing startup provenance remains explicit Unknown");
        Check(IdentityCapture.Get(new ProbeModel("A"), CaptureRuntime.Epoch).Generation == GenerationState.Ordinary, "Preparation does not misclassify an unrelated ordinary card");
    }
    private static void EpochAndThread()
    {
        var model = new ProbeModel("A"); var pause = Pause(); var pending = model.Suspended(pause.Task); var prior = CaptureRuntime.Epoch;
        var newCombat = RuntimeHelpers.GetUninitializedObject(typeof(CombatState)); backend.Combat = newCombat; CaptureRuntime.Register(backend, ++epoch, newCombat);
        pause.SetResult(1); context.Complete(pending);
        Check(pending.Result.Epoch == prior.Sequence && !CaptureRuntime.Valid(prior), "Old immutable source survives locally but is rejected after combat replacement");
        int calls = backend.Calls.Count;
        Task.Run(() => { Check(FlowCapture.Source(model, CaptureRuntime.Epoch).Epoch == 0, "Off-thread capture returns unavailable"); }).GetAwaiter().GetResult();
        Check(backend.Calls.Count == calls && CaptureRuntime.WrongThreadObserved, "Wrong-thread gameplay records release-blocking evidence without native STATE access");
    }
}
