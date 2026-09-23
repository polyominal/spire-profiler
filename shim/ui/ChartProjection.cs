using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;

namespace SpireProfiler;

internal enum ChartSegment { Direct, Indirect, Modifier, Weak, Buff, Strength, SelfDamage }
internal sealed record ChartRow(StatRow Source, bool Defense, bool SelfDamage, long Value, double Share, IReadOnlyList<long> Segments);

internal static class ChartProjection
{
    internal static IReadOnlyList<ChartRow> Rows(IReadOnlyList<StatRow> cards, bool defense, int? player)
    {
        var candidates = cards.Where(card => player == null || card.Player == player.Value)
            .Select(card => (Card: card, Segments: defense
                ? new long[] { card.BlockEffective, 0, card.BlkModifier, card.MitigateDebuff, card.MitigateBuff, card.MitigateStr, 0 }
                : new long[] { card.DmgDirect, card.DmgAttributed, card.DmgModifier, 0, 0, 0, 0 }))
            .Select(row => (row.Card, row.Segments, Value: row.Segments.Sum()))
            .Where(row => row.Value > 0 || (defense && row.Card.SelfDamage > 0))
            .OrderBy(row => row.Value == 0)
            .ThenByDescending(row => Math.Abs((double)row.Value - (defense ? row.Card.SelfDamage : 0)))
            .ToArray();
        double total = candidates.Sum(row => (double)row.Value);
        var rows = new List<ChartRow>();
        foreach (var row in candidates)
        {
            if (row.Value > 0)
                rows.Add(new ChartRow(row.Card, defense, false, row.Value, row.Value / total, row.Segments));
            if (defense && row.Card.SelfDamage > 0)
                rows.Add(new ChartRow(row.Card, true, true, -row.Card.SelfDamage, 0,
                    new long[] { 0, 0, 0, 0, 0, 0, row.Card.SelfDamage }));
        }
        return rows;
    }

    internal static string Detail(StatRow card, string name)
    {
        var text = new StringBuilder(name).Append(" ×").Append(card.Plays);
        text.AppendLine().Append(card.Id);
        if (card.DamageDealt != 0)
        {
            text.AppendLine().Append(CultureInfo.InvariantCulture, $"Damage: {card.DamageDealt} ({card.DamageDealt - card.DamageBlocked} unblocked)");
            text.AppendLine().Append(CultureInfo.InvariantCulture, $"  Direct: {card.DmgDirect}   Indirect: {card.DmgAttributed}   Modifier: {card.DmgModifier}");
        }
        if (card.BlockGained != 0 || card.BlockEffective != 0 || card.BlkModifier != 0)
            text.AppendLine().Append(CultureInfo.InvariantCulture, $"Block: {card.BlockGained} ({card.BlockEffective} effective), modifier: {card.BlkModifier}");
        if (card.MitigateDebuff != 0 || card.MitigateBuff != 0 || card.MitigateStr != 0)
            text.AppendLine().Append(CultureInfo.InvariantCulture, $"Prevented: weak {card.MitigateDebuff}, buff {card.MitigateBuff}, strength {card.MitigateStr}");
        if (card.SelfDamage != 0) text.AppendLine().Append(CultureInfo.InvariantCulture, $"Self damage: {card.SelfDamage}");
        if (card.Forge != 0) text.AppendLine().Append(CultureInfo.InvariantCulture, $"Forge: {card.Forge}");
        return text.ToString();
    }

    internal static string Coverage(CoverageSummary coverage)
    {
        if (coverage == null || !coverage.Known) return "Capture completeness unknown";
        if (coverage.Complete) return "";
        string reasons = string.Join("; ", coverage.Reasons);
        return string.IsNullOrEmpty(reasons) ? "Incomplete capture" : $"Incomplete capture: {reasons}";
    }
}
