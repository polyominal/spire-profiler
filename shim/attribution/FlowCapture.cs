using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Reflection;
using System.Threading;
using HarmonyLib;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;

namespace SpireProfiler;

internal sealed record ProducerFrame(CaptureEpoch Epoch, object Model, SourceSnapshot Source, ProducerRole Role, DamageSegment Segment, bool Poison = false)
{
    internal static readonly ProducerFrame Barrier = new(default, null, SourceSnapshot.Unavailable, ProducerRole.Unknown, DamageSegment.Attributed);
}
internal static class FlowCapture
{
    private static readonly AsyncLocal<ProducerFrame> producer = new();
    internal static ProducerFrame Current { get => producer.Value ?? ProducerFrame.Barrier; set => producer.Value = value; }
    internal static SourceSnapshot Source(object model, CaptureEpoch epoch)
    {
        try
        {
            if (model == null || !CaptureRuntime.Valid(epoch)) return SourceSnapshot.Unavailable;
            var descriptor = CaptureRuntime.Backend.Describe(model);
            if (descriptor.Combat != null && !ReferenceEquals(descriptor.Combat, epoch.Combat)) return SourceSnapshot.Unavailable;
            IdentityMetadata metadata = null;
            if (descriptor.Kind is CaptureKind.CardInstance or CaptureKind.PowerInstance or CaptureKind.OrbInstance)
            {
                metadata = IdentityCapture.Get(model, epoch);
                if (metadata == null || metadata.Dirty) return SourceSnapshot.Unavailable;
                if (descriptor.Kind == CaptureKind.PowerInstance && metadata.Detached != null) return metadata.Detached;
            }
            return CaptureRuntime.Copy(epoch, descriptor.Kind, metadata?.Identity ?? 0, descriptor.Id, descriptor.SourceKind,
                descriptor.Slot, metadata?.Generation ?? GenerationState.Unclassified);
        }
        catch (Exception ex) { CaptureRuntime.Fail("model-source", ex); return SourceSnapshot.Unavailable; }
    }
    internal static SourceSnapshot Supplied(object explicitModel, CaptureEpoch epoch, ProducerFrame inherited = null)
    {
        inherited ??= Current;
        if (explicitModel == null) return CaptureRuntime.Valid(inherited.Epoch) ? inherited.Source : SourceSnapshot.Unavailable;
        if (ReferenceEquals(explicitModel, inherited.Model) && CaptureRuntime.Valid(inherited.Epoch)) return inherited.Source;
        return Source(explicitModel, epoch);
    }
    internal static void Prefix(object __instance, MethodBase __originalMethod, object[] __args, out ProducerFrame __state)
    {
        __state = Current;
        Current = ProducerFrame.Barrier;
        try
        {
            var epoch = CaptureRuntime.EntryEpoch(__state.Epoch);
            Current = ProducerFrame.Barrier with { Epoch = epoch };
            if (!CaptureRuntime.Valid(epoch)) return;
            var descriptor = CaptureRuntime.Backend.Describe(__instance);
            if (descriptor.Combat != null && !ReferenceEquals(descriptor.Combat, epoch.Combat))
            {
                Current = ProducerFrame.Barrier with { Epoch = epoch with { Combat = descriptor.Combat } };
                return;
            }
            SourceSnapshot source;
            if (CaptureRuntime.Backend.TemporaryPower(__instance) && ProvenanceCapture.Matching(__instance)
                && (__originalMethod.Name == "BeforeApplied" || (__originalMethod.Name == "AfterPowerAmountChanged" && __args.Length > 1 && ReferenceEquals(__args[1], __instance))))
                source = ProvenanceCapture.Pending.Source;
            else if (__originalMethod.Name == "BeforeHandDraw" && __instance is HelloWorldPower)
                source = TemporalPowerCapture.TurnSource(__instance, epoch);
            else source = Source(__instance, epoch);
            var segment = descriptor.Role is ProducerRole.Card or ProducerRole.Relic or ProducerRole.Potion ? DamageSegment.Direct : DamageSegment.Attributed;
            Current = new(epoch, __instance, source, descriptor.Role, segment, descriptor.Poison);
        }
        catch (Exception ex) { CaptureRuntime.Fail("producer-prefix", ex); }
    }
    internal static void Finalizer(ProducerFrame __state) { if (__state != null) Current = __state; }
    internal static bool SameMethod(MethodInfo a, MethodInfo b) => a.Module == b.Module && a.MetadataToken == b.MetadataToken;
    internal static MethodInfo DeclaredMethod(MethodInfo method)
    {
        if (method == null) throw new InvalidOperationException("Missing patch target");
        var declared = method.DeclaringType.GetMethods(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static | BindingFlags.DeclaredOnly)
            .Single(candidate => SameMethod(candidate, method));
        return method.IsGenericMethod && !method.IsGenericMethodDefinition
            ? declared.MakeGenericMethod(method.GetGenericArguments()) : declared;
    }
    internal static IReadOnlyList<MethodInfo> ProducerTargets(Assembly assembly)
    {
        var roots = new HashSet<Type> { typeof(AbstractModel), typeof(RelicModel), typeof(PowerModel) };
        var targets = new Dictionary<(Module, int), MethodInfo>();
        foreach (var type in assembly.GetTypes().Where(t => !t.IsAbstract && typeof(AbstractModel).IsAssignableFrom(t)))
        {
            foreach (var method in type.GetMethods(BindingFlags.Instance | BindingFlags.Public))
                if (method.IsVirtual && !method.IsSpecialName && method.Name != "CompareTo" && roots.Contains(method.GetBaseDefinition().DeclaringType)
                    && !SameMethod(method, method.GetBaseDefinition())) targets.TryAdd((method.Module, method.MetadataToken), method);
            foreach (var method in type.GetMethods(BindingFlags.Instance | BindingFlags.NonPublic))
                if ((method.Name == "OnPlay" || method.Name == "OnUse") && method.IsVirtual && !method.IsAbstract
                    && !SameMethod(method, method.GetBaseDefinition())) targets.TryAdd((method.Module, method.MetadataToken), method);
        }
        return targets.Values.Select(DeclaredMethod).OrderBy(m => m.DeclaringType.FullName, StringComparer.Ordinal).ThenBy(m => m.Name, StringComparer.Ordinal).ThenBy(m => m.ToString(), StringComparer.Ordinal).ToArray();
    }
    internal static void PatchProducer(Harmony harmony, MethodInfo target)
        => CapturePatches.Patch(harmony, DeclaredMethod(target), prefix: new HarmonyMethod(typeof(FlowCapture), nameof(Prefix)), finalizer: new HarmonyMethod(typeof(FlowCapture), nameof(Finalizer)));
    internal static void Install(Harmony harmony, Action<string> report)
    {
        var timer = Stopwatch.StartNew();
        var targets = ProducerTargets(typeof(AbstractModel).Assembly);
        if (targets.Count != 1726) throw new InvalidOperationException($"Producer definition inventory changed: {targets.Count}");
        var expected = new Dictionary<string, int> { ["AbstractModel"] = 934, ["CardModel"] = 564, ["PotionModel"] = 65, ["PowerModel"] = 57, ["RelicModel"] = 106 };
        foreach (var group in targets.GroupBy(method => method.GetBaseDefinition().DeclaringType.Name))
            if (!expected.TryGetValue(group.Key, out int count) || group.Count() != count) throw new InvalidOperationException("Producer root inventory changed: " + group.Key);
        report($"PRODUCER TARGET INVENTORY COUNT: {targets.Count}");
        foreach (var group in targets.GroupBy(m => m.GetBaseDefinition().DeclaringType.Name).OrderBy(g => g.Key)) report($"Producer root {group.Key}: {group.Count()}");
        foreach (var target in targets)
        {
            report($"PRODUCER {target.DeclaringType.FullName}::{target}");
            PatchProducer(harmony, target);
        }
        foreach (var pair in new[] { (typeof(PoisonPower), "Trigger"), (typeof(RollingBoulderPower), "DoDamage") })
        {
            var target = AccessTools.DeclaredMethod(pair.Item1, pair.Item2) ?? throw new InvalidOperationException("Missing independent power producer");
            if (!targets.Any(m => SameMethod(m, target))) { PatchProducer(harmony, target); report($"DIRECT PRODUCER {target.DeclaringType.FullName}::{target}"); }
        }
        ProvenanceCapture.Install(harmony);
        PlayCapture.Install(harmony);
        TemporalPowerCapture.Install(harmony, report);
        DamageCapture.Install(harmony, report);
        CommandCapture.Install(harmony, report);
        DoomCapture.Install(harmony, report);
        CapturePatches.Verify(harmony, report);
        report($"CAPTURE INSTALL MILLISECONDS: {timer.ElapsedMilliseconds}");
    }
}

internal abstract class GameAttributionBackend : AttributionBackend
{
    internal override ModelDescriptor Describe(object model) => model switch
    {
        CardModel card => new(CaptureKind.CardInstance, ProducerRole.Card, card.Id?.Entry ?? "", 0, RunContext.CreditorSlot(card.Owner), Combat: card.Owner?.Creature?.CombatState),
        PowerModel power => new(CaptureKind.PowerInstance, ProducerRole.Power, power.Id?.Entry ?? "", 2, 4, power.Owner, power is PoisonPower, power.Owner?.CombatState),
        RelicModel relic => new(CaptureKind.DirectModel, ProducerRole.Relic, relic.Id?.Entry ?? "", 1, RunContext.CreditorSlot(relic.Owner), Combat: relic.Owner?.Creature?.CombatState),
        PotionModel potion => new(CaptureKind.DirectModel, ProducerRole.Potion, potion.Id?.Entry ?? "", 3, RunContext.CreditorSlot(potion.Owner), Combat: potion.Owner?.Creature?.CombatState),
        OrbModel orb => new(CaptureKind.OrbInstance, ProducerRole.Orb, orb.Id?.Entry ?? "", 5, RunContext.PlayerSlot(orb.Owner), Combat: orb.Owner?.Creature?.CombatState),
        _ => new(CaptureKind.Unknown, ProducerRole.Unknown, "", 5, 4)
    };
    internal override CreatureDescriptor DescribeCreature(object model)
    {
        var creature = (MegaCrit.Sts2.Core.Entities.Creatures.Creature)model;
        return new(creature.IsPlayer, creature.Monster is MegaCrit.Sts2.Core.Models.Monsters.Osty,
            creature.IsPlayer ? RunContext.PlayerSlot(creature.Player) : creature.PetOwner != null ? RunContext.PlayerSlot(creature.PetOwner) : 4, creature.CombatState);
    }
    internal override PowerObservation ObservePower(object model, object owner = null)
    {
        var power = (PowerModel)model;
        var creature = owner as MegaCrit.Sts2.Core.Entities.Creatures.Creature ?? power.Owner;
        bool attached = creature != null && creature.Powers.Any(p => ReferenceEquals(p, power));
        int kind = creature == null ? 3 : creature.IsPlayer ? 0 : creature.IsPet ? 2 : creature.IsMonster ? 1 : 3;
        int slot = creature?.IsPlayer == true ? RunContext.PlayerSlot(creature.Player) : creature?.PetOwner != null ? RunContext.PlayerSlot(creature.PetOwner) : 4;
        return new(power, creature, power.Id?.Entry ?? "", kind, slot, power.Amount, attached);
    }
    internal override bool TemporaryPower(object power) => power is TemporaryStrengthPower or TemporaryFocusPower or TemporaryDexterityPower;
}

internal static class CapturePatches
{
    private static readonly Dictionary<string, List<(MethodInfo Original, MethodInfo Patch, string Kind)>> installed = new();
    internal static void Patch(Harmony harmony, MethodInfo original, HarmonyMethod prefix = null, HarmonyMethod postfix = null, HarmonyMethod transpiler = null, HarmonyMethod finalizer = null)
    {
        original = FlowCapture.DeclaredMethod(original);
        harmony.Patch(original, prefix: prefix, postfix: postfix, transpiler: transpiler, finalizer: finalizer);
        if (!installed.TryGetValue(harmony.Id, out var records)) installed[harmony.Id] = records = new();
        foreach (var entry in new[] { (prefix, "prefix"), (postfix, "postfix"), (transpiler, "transpiler"), (finalizer, "finalizer") })
            if (entry.Item1 != null) records.Add((original, entry.Item1.method, entry.Item2));
    }
    internal static void Verify(Harmony harmony, Action<string> report)
    {
        var records = installed[harmony.Id];
        foreach (var record in records)
        {
            var info = Harmony.GetPatchInfo(record.Original) ?? throw new InvalidOperationException("Capture patch disappeared");
            var patches = record.Kind switch { "prefix" => info.Prefixes, "postfix" => info.Postfixes, "transpiler" => info.Transpilers, _ => info.Finalizers };
            if (!patches.Any(patch => patch.owner == harmony.Id && FlowCapture.SameMethod(patch.PatchMethod, record.Patch)))
                throw new InvalidOperationException($"Missing owned capture {record.Kind}: {record.Original.DeclaringType.FullName}.{record.Original.Name}");
        }
        int targets = records.Select(record => (record.Original.Module, record.Original.MetadataToken)).Distinct().Count();
        report($"CAPTURE VERIFIED owner={harmony.Id} targets={targets} patches={records.Count} producers=1726 damage_bridges=4 temporal_bridges=16");
    }
}
