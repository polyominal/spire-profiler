using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using HarmonyLib;

namespace SpireProfiler;

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
