using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;

namespace SpireProfiler;

internal enum UiTab { Combat, Run }
internal enum ChartSection { Damage, Defense }
internal enum ChartSegment { Direct, Attributed, Modifier, MitigateDebuff, MitigateBuff, MitigateStr, SelfDamage }
internal sealed record ChartRow(StatRow Source, ChartSection Section, int Flags, string Name, long Value, int ShareX10, IReadOnlyList<int> SegMilli)
{
    internal bool SelfDamage => (Flags & 2) != 0;
    internal bool SoloSelf => (Flags & 4) != 0;
}
internal sealed record ChartMeta(uint Turns = 0, uint Plays = 0, uint Combats = 0, long TotalDamage = 0, long DamageTaken = 0, int DpsX10 = 0, string Encounter = "");
internal sealed record DetailStat(string Label, string Value, UiColor Color);
internal sealed record RowDetail(string Title, IReadOnlyList<DetailStat> Stats)
{
    internal static readonly RowDetail Empty = new("", Array.Empty<DetailStat>());
    internal bool IsEmpty => Title.Length == 0 && Stats.Count == 0;
}

internal static class ChartProjection
{
    internal const int MaxRows = 256;
    internal const int MaxCandidates = 128;

    internal static IReadOnlyList<ChartRow> Rows(IReadOnlyList<StatRow> cards, int? player = null)
    {
        var selected = cards.Where(card => player == null || card.Player == player).ToArray();
        var output = new List<ChartRow>(Math.Min(MaxRows, selected.Length * 2));
        foreach (var section in new[] { ChartSection.Damage, ChartSection.Defense })
        {
            var candidates = selected.Select(card =>
            {
                var segments = section == ChartSection.Damage
                    ? new[] { card.DmgDirect, card.DmgAttributed, card.DmgModifier, 0, 0, 0, 0 }
                    : new[] { card.BlockEffective, 0, card.BlkModifier, card.MitigateDebuff, card.MitigateBuff, card.MitigateStr, card.SelfDamage };
                long positive = section == ChartSection.Damage ? card.DmgDirect + card.DmgAttributed + card.DmgModifier
                    : card.BlockEffective + card.BlkModifier + card.MitigateDebuff + card.MitigateBuff + card.MitigateStr;
                long value = positive - (section == ChartSection.Defense ? card.SelfDamage : 0);
                return (Card: card, Segments: segments, Positive: positive, Value: value);
            }).ToArray();
            var kept = candidates.Where(row => row.Value > 0 || (section == ChartSection.Defense && row.Card.SelfDamage > 0))
                .Take(MaxCandidates).OrderBy(row => section == ChartSection.Defense && row.Card.SelfDamage > 0 && row.Positive == 0)
                .ThenByDescending(row => Math.Abs(row.Value)).ToArray();
            long maximum = kept.Length == 0 ? 0 : kept.Max(row => Math.Abs(row.Value));
            long total = candidates.Sum(row => row.Positive);
            foreach (var item in kept)
            {
                bool self = section == ChartSection.Defense && item.Card.SelfDamage > 0;
                bool split = self && item.Positive > 0;
                var segments = item.Segments;
                if (split) segments[6] = 0;
                if (output.Count < MaxRows)
                    output.Add(MakeRow(item.Card, section, self && !split ? 6 : 0, split ? item.Positive : item.Value, segments, maximum, total));
                if (split && output.Count < MaxRows)
                    output.Add(MakeRow(item.Card, section, 2, -item.Card.SelfDamage, new[] { 0L, 0, 0, 0, 0, 0, item.Card.SelfDamage }, maximum, total));
            }
        }
        return output;
    }

    private static ChartRow MakeRow(StatRow source, ChartSection section, int flags, long value, long[] segments, long maximum, long total)
    {
        int share = (flags & 2) == 0 && total > 0 && value > 0 ? (int)((Int128)value * 1000 / total) : 0;
        var milli = new int[7];
        if (maximum > 0)
            for (int index = 0; index < milli.Length; index++)
                if (segments[index] > 0) milli[index] = (int)Int128.Min((Int128)segments[index] * 1000 / maximum, 1000);
        return new(source, section, flags, TruncateBytes(source.Id, 64), value, share, milli);
    }

    internal static ChartMeta Meta(SummaryView view, UiTab tab, int? historyPlayer = null)
    {
        if (view == null) return new();
        var cards = historyPlayer == null ? view.Cards : view.Cards.Where(card => card.Player == historyPlayer).ToArray();
        long damage = cards.Sum(card => card.DamageDealt);
        uint plays = tab == UiTab.Combat ? view.Plays : (uint)cards.Sum(card => (long)card.Plays);
        return new(view.Turns, plays, view.Combats, damage, view.DamageReceived,
            view.Turns == 0 ? 0 : (int)((Int128)damage * 10 / view.Turns), tab == UiTab.Combat ? TruncateBytes(view.Title, 64) : "");
    }

    internal static string Footer(SummaryView view, UiTab tab)
    {
        if (tab == UiTab.Run)
            return view == null || view.Combats == 0 ? "no completed combats this run yet"
                : $"RUN TOTAL {view.Cards.Sum(card => card.DamageDealt)} dmg | {view.Turns} turns | {view.Combats} combats | {view.Cards.Sum(card => card.BlockGained)} block\n";
        return view == null ? "" : $"TOTAL {view.Cards.Sum(card => card.DamageDealt)} dmg | {view.DamageReceived} taken | {view.BlockTotal} block | pots {view.PotionsUsed} | forge {view.Cards.Sum(card => card.Forge)}\n";
    }

    internal static RowDetail Detail(IReadOnlyList<ChartRow> rows, int index, IReadOnlyList<StatRow> cards)
    {
        if (index < 0 || index >= rows.Count) return RowDetail.Empty;
        var row = rows[index];
        string prefix = UiPalette.Prefix(row.Source.Kind).Text;
        if (row.SelfDamage && !row.SoloSelf)
            return new(prefix + row.Name, new[] { new DetailStat("self dmg", Math.Abs(row.Value).ToString(CultureInfo.InvariantCulture), UiPalette.Self) });
        var card = cards.FirstOrDefault(card => card.Player == row.Source.Player && card.Id == row.Name);
        if (card == null) return RowDetail.Empty;
        var stats = new List<DetailStat>();
        void Add(string label, long value, UiColor color) => stats.Add(new(label, value.ToString(CultureInfo.InvariantCulture), color));
        if (card.DamageDealt > 0)
        {
            Add($"dmg ({card.DamageDealt - card.DamageBlocked} unblk)", card.DamageDealt, UiPalette.Damage);
            Add("direct", card.DmgDirect, UiPalette.Damage);
            Add("indirect", card.DmgAttributed, UiPalette.Attributed);
            Add("mod", card.DmgModifier, UiPalette.Modifier);
        }
        if (card.BlockGained > 0)
        {
            Add($"block ({card.BlockEffective} eff)", card.BlockGained, card.Kind == 4 ? UiPalette.Osty : UiPalette.Block);
            Add("blk mod", card.BlkModifier, UiPalette.Modifier);
        }
        if (card.MitigateDebuff > 0 || card.MitigateBuff > 0 || card.MitigateStr > 0)
        {
            Add("weak", card.MitigateDebuff, UiPalette.Weak);
            Add("buff", card.MitigateBuff, UiPalette.Buff);
            Add("str", card.MitigateStr, UiPalette.Strength);
        }
        if (card.SelfDamage > 0) Add("self dmg", card.SelfDamage, UiPalette.Self);
        if (card.Forge > 0) Add("forge", card.Forge, UiPalette.Cream);
        return new($"{UiPalette.Prefix(card.Kind).Text}{card.Id} x{card.Plays}", stats);
    }

    internal static string TruncateBytes(string text, int maximum)
    {
        int bytes = 0, length = 0;
        foreach (var rune in text.EnumerateRunes())
        {
            if (bytes + rune.Utf8SequenceLength > maximum) break;
            bytes += rune.Utf8SequenceLength;
            length += rune.Utf16SequenceLength;
        }
        return text[..length];
    }

    internal static string TruncateMarked(string text, int maximum)
    {
        var runes = text.EnumerateRunes().ToArray();
        return runes.Length <= maximum ? text : string.Concat(runes.Take(maximum - 1).Select(rune => rune.ToString())) + "…";
    }
}
