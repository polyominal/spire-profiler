using System;
using System.Collections.Generic;
using Godot;

namespace SpireProfiler;

internal sealed class TooltipLayout : IDisposable
{
    internal const float Width = 360;
    private const int FontSize = 22;
    private const float TextWidth = 293, ValueX = 170, ValueWidth = 123;
    private sealed record Block(TextParagraph Text, Font Font, UiColor Color, float Y, DetailStat Value);
    private readonly List<Block> _blocks = new();
    internal float Height { get; private set; } = 44;

    internal TooltipLayout(RowDetail detail, float maximumHeight, Font title, Font body)
    {
        try
        {
            if (!Add(detail.Title.Trim(), title, UiPalette.Gold, maximumHeight)) return;
            if (body == null) return;
            foreach (var stat in detail.Stats)
            {
                bool columns = body.GetStringSize(stat.Label, fontSize: FontSize).X <= ValueX - 8
                    && body.GetStringSize(stat.Value, fontSize: FontSize).X <= ValueWidth;
                if (!Add(columns ? stat.Label : stat.Label + " " + stat.Value, body,
                    columns ? UiPalette.Cream : stat.Color, maximumHeight, columns ? stat : null)) break;
            }
        }
        catch { Dispose(); throw; }
    }

    private bool Add(string text, Font font, UiColor color, float maximumHeight, DetailStat value = null)
    {
        if (font == null) return true;
        var paragraph = new TextParagraph
        {
            Width = value == null ? TextWidth : ValueX - 8,
            BreakFlags = TextServer.LineBreakFlag.Mandatory | TextServer.LineBreakFlag.WordBound | TextServer.LineBreakFlag.Adaptive,
            TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
        };
        try
        {
            paragraph.AddString(text, font, FontSize);
            int count = paragraph.GetLineCount(), visible = 0;
            float height = 0;
            while (visible < count && Height + height + paragraph.GetLineSize(visible).Y <= maximumHeight)
                height += paragraph.GetLineSize(visible++).Y;
            if (visible == 0)
            {
                paragraph.Dispose();
                if (_blocks.Count != 0)
                {
                    var last = _blocks[^1];
                    last.Text.AddString("\n…", last.Font, FontSize);
                }
                return false;
            }
            paragraph.MaxLinesVisible = visible;
            _blocks.Add(new(paragraph, font, color, Height - 44, value));
            Height += height;
            return visible == count;
        }
        catch { paragraph.Dispose(); throw; }
    }

    internal void Draw(Control canvas, UiRect rect)
    {
        foreach (var block in _blocks)
        {
            var position = new Vector2(rect.X + 22, rect.Y + 16 + block.Y);
            block.Text.Draw(canvas.GetCanvasItem(), position + new Vector2(3, 2), new Color(0, 0, 0, .25098f));
            block.Text.Draw(canvas.GetCanvasItem(), position, new Color(block.Color.R, block.Color.G, block.Color.B, block.Color.A));
            if (block.Value is { } value)
            {
                position += new Vector2(ValueX, block.Text.GetLineAscent(0));
                canvas.DrawString(block.Font, position + new Vector2(3, 2), value.Value, HorizontalAlignment.Right, ValueWidth, FontSize, new Color(0, 0, 0, .25098f));
                canvas.DrawString(block.Font, position, value.Value, HorizontalAlignment.Right, ValueWidth, FontSize,
                    new Color(value.Color.R, value.Color.G, value.Color.B, value.Color.A));
            }
        }
    }

    public void Dispose()
    {
        foreach (var block in _blocks) block.Text.Dispose();
        _blocks.Clear();
    }
}
