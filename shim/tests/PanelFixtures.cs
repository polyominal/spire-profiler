using System;
using System.Linq;

namespace SpireProfiler;

internal static class PanelFixtures
{
    internal static void Run()
    {
        var cards = new[]
        {
            new StatRow { Id = "SHARED", Player = 0, DamageDealt = 30, DmgDirect = 10, DmgAttributed = 12, DmgModifier = 8, BlockEffective = 5, BlkModifier = 3, SelfDamage = 20, Forge = 4 },
            new StatRow { Id = "SHARED", Player = 1, Kind = 1, DamageDealt = 15, DmgDirect = 15, BlockEffective = 7, MitigateDebuff = 2, MitigateBuff = 1, MitigateStr = 3 },
            new StatRow { Id = "COST", Player = 0, SelfDamage = 50 },
        };
        var rows = ChartProjection.Rows(cards);
        var damage = rows.Where(row => row.Section == ChartSection.Damage).ToArray();
        if (damage.Length != 2 || damage[0].ShareX10 != 666 || damage[1].ShareX10 != 333)
            throw new InvalidOperationException("Damage shares must use the baseline's truncated tenths");
        var own = rows.Single(row => row.Section == ChartSection.Defense && row.Source.Player == 0 && row.Name == "SHARED" && !row.SelfDamage);
        if (own.Value != 8 || own.SegMilli[(int)ChartSegment.Direct] != 100 || own.SegMilli[(int)ChartSegment.Modifier] != 60)
            throw new InvalidOperationException("Split defense bars must retain the net-defense section scale");
        var filtered = ChartProjection.Rows(cards, 1);
        if (filtered.Count != 2 || filtered.Any(row => row.Source.Player != 1) || filtered[0].ShareX10 != 1000)
            throw new InvalidOperationException("Player selection must retain source ownership and independent shares");
        int hanging = rows.Select((row, index) => (row, index)).Single(item => item.row.SelfDamage && !item.row.SoloSelf).index;
        var terse = ChartProjection.Detail(rows, hanging, cards);
        if (terse.Stats.Count != 1 || terse.Stats[0].Label != "self dmg" || terse.Stats[0].Value != "20")
            throw new InvalidOperationException("Hanging self-damage detail must stay terse");
        var detail = ChartProjection.Detail(rows, 0, cards);
        if (!detail.Stats.Any(stat => stat.Label == "forge" && stat.Value == "4") || detail.Title != "SHARED x0")
            throw new InvalidOperationException("Raw source IDs and non-chart statistics must remain in tooltips");
        if (PanelGeometry.DragState(false, true, true, true) || !PanelGeometry.DragState(true, true, true, false))
            throw new InvalidOperationException("A held cursor must not start a drag, and active drags must survive leaving the track");
        Console.WriteLine("MANAGED PANEL PROJECTION FIXTURES PASS");
    }
}
