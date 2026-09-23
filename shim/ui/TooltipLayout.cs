using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;

namespace SpireProfiler;

internal sealed record TipValue(string Text, UiColor Color);
internal sealed record TipLine(string Text, bool Title, UiColor Color, TipValue Value = null);

internal static class TooltipLayout
{
    internal const float Width = 360;
    internal static float Height(int lines) => lines * 26 + 44;
    internal static int MaximumLines(float boxHeight) => (int)Math.Max(1, MathF.Floor((boxHeight - 60) / 26));
    internal static IReadOnlyList<TipLine> Shape(RowDetail detail, int maximum)
    {
        var lines = new List<TipLine>();
        Wrap(detail.Title.Trim(), 18, true, UiPalette.Gold, lines);
        foreach (var stat in detail.Stats)
        {
            if (stat.Label.EnumerateRunes().Count() <= 18 && stat.Value.EnumerateRunes().Count() <= 10)
                lines.Add(new(stat.Label, false, UiPalette.Cream, new(stat.Value, stat.Color)));
            else Wrap(stat.Label + " " + stat.Value, 24, false, stat.Color, lines);
        }
        if (lines.Count > maximum)
        {
            lines.RemoveRange(maximum, lines.Count - maximum);
            if (lines.Count != 0) lines[^1] = new("…", false, UiPalette.Dim);
        }
        return lines;
    }
    private static void Wrap(string text, int budget, bool title, UiColor color, List<TipLine> output)
    {
        string line = "";
        void Flush()
        {
            if (line.Length == 0) return;
            output.Add(new(line, title, color));
            line = "";
        }
        foreach (string word in text.Split(' ', StringSplitOptions.RemoveEmptyEntries))
        {
            string rest = word;
            while (rest.Length != 0)
            {
                int count = line.EnumerateRunes().Count(), separator = line.Length == 0 ? 0 : 1;
                if (count + separator + rest.EnumerateRunes().Count() <= budget)
                {
                    line += (separator == 0 ? "" : " ") + rest;
                    break;
                }
                if (line.Length != 0 && rest.EnumerateRunes().Count() <= budget) { Flush(); continue; }
                int room = Math.Max(0, budget - count - separator);
                if (line.Length != 0)
                {
                    if (room > 0)
                    {
                        string piece = ChartProjection.TruncateBytes(rest, room);
                        if (piece.Length != 0) { line += " " + piece; rest = rest[piece.Length..]; }
                    }
                    Flush();
                    continue;
                }
                string chunk = ChartProjection.TruncateBytes(rest, budget);
                line += chunk;
                rest = rest[chunk.Length..];
                Flush();
            }
        }
        Flush();
    }
}
