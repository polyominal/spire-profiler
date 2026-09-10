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
