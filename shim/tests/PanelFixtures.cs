using System;
using System.Linq;

namespace SpireProfiler;

#pragma warning disable CA1861 // Fixture expectations stay beside the assertions they explain.

internal static class PanelFixtures
{
    internal static void Run()
    {
        var rows = new[]
        {
            new StatRow { Id = "SHARED", Player = 0, Kind = 0, DamageDealt = 30, DmgDirect = 10, DmgAttributed = 12, DmgModifier = 8, BlockEffective = 5, BlkModifier = 3, SelfDamage = 20, Forge = 4 },
            new StatRow { Id = "SHARED", Player = 1, Kind = 1, DamageDealt = 15, DmgDirect = 15, BlockEffective = 7, MitigateDebuff = 2, MitigateBuff = 1, MitigateStr = 3 },
            new StatRow { Id = "COST", Player = 0, SelfDamage = 50 },
        };
        var damage = ChartProjection.Rows(rows, false, null);
        if (damage.Count != 2 || damage.Sum(row => row.Value) != 45 || Math.Abs(damage.Sum(row => row.Share) - 1) > 0.000001)
            throw new InvalidOperationException("Damage chart lost assigned credit or section shares");
        var filtered = ChartProjection.Rows(rows, false, 1);
        if (filtered.Count != 1 || filtered[0].Source.Kind != 1 || filtered[0].Share != 1)
            throw new InvalidOperationException("Player filtering conflated identical source IDs");
        var defense = ChartProjection.Rows(rows, true, null);
        if (defense.Count != 4 || defense.Where(row => !row.SelfDamage).Sum(row => row.Value) != 21
            || defense.Where(row => row.SelfDamage).Sum(row => row.Value) != -70)
            throw new InvalidOperationException("Defense chart conflated protection with self-damage costs");
        if (defense[^1].Source.Id != "COST" || defense[^1].Share != 0)
            throw new InvalidOperationException("Standalone self-damage must follow protection and have no positive share");
        var own = defense.Single(row => row.Source.Player == 0 && row.Source.Id == "SHARED" && !row.SelfDamage);
        if (own.Value != 8 || own.Segments[(int)ChartSegment.SelfDamage] != 0)
            throw new InvalidOperationException("Mixed defense source lost protection when its HP cost was larger");
        string detail = ChartProjection.Detail(rows[0], "Fixture");
        if (!detail.Contains("Forge: 4", StringComparison.Ordinal) || !detail.Contains("Self damage: 20", StringComparison.Ordinal)
            || !detail.Contains("Indirect: 12", StringComparison.Ordinal))
            throw new InvalidOperationException("Tooltip lost non-chart statistics or damage decomposition");
        if (!ChartProjection.Coverage(new CoverageSummary()).Contains("unknown", StringComparison.Ordinal)
            || !ChartProjection.Coverage(new CoverageSummary { Quality = CaptureQuality.Partial, Reasons = new[] { "capacity" } }).Contains("capacity", StringComparison.Ordinal))
            throw new InvalidOperationException("Unknown and incomplete captures must remain visible");
        Console.WriteLine("MANAGED PANEL PROJECTION FIXTURES PASS");
    }
}
