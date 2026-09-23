using System;
using System.Collections.Generic;
using System.Linq;

namespace SpireProfiler;

internal readonly record struct UiColor(float R, float G, float B, float A = 1);
internal enum TextRole { Title, Body }
internal enum TextEffect { Plain, Shadow, Outline }
internal enum TextAlign { Left, Right, Center, LeftClipped }
internal enum IconId { Character, TabPlate, TabStroke }
internal abstract record DrawCommand(float X, float Y);
internal sealed record RectCommand(float X, float Y, float W, float H, UiColor Color) : DrawCommand(X, Y);
internal sealed record TextCommand(float X, float Y, int Size, UiColor Color, TextRole Role, TextEffect Effect, TextAlign Align, float Width, string Text) : DrawCommand(X, Y);
internal sealed record TextureCommand(float X, float Y, float W, float H, IconId Icon, int Slot = 0) : DrawCommand(X, Y);
internal readonly record struct ContentBox(float X, float Top, float W, float OuterBottomPad)
{
    internal float Right => X + W;
}
internal readonly record struct RowHit(float Y0, float Y1, int FlatIndex);
internal readonly record struct TabHit(float X0, float Y0, float X1, float Y1, UiTab Tab);
internal readonly record struct AvatarHit(float X0, float Y0, float X1, float Y1, int Slot);
internal sealed record AvatarFact(int Slot, bool Loaded, string Path);

internal static class UiPalette
{
    internal static readonly UiColor Damage = new(1, .392157f, .392157f);
    internal static readonly UiColor Attributed = new(1, .647059f, .094118f);
    internal static readonly UiColor Modifier = new(.933333f, .509804f, .933333f);
    internal static readonly UiColor Self = new(.749020f, .188235f, .188235f);
    internal static readonly UiColor Block = new(.403922f, .682353f, .921569f);
    internal static readonly UiColor Weak = new(.498039f, 1, 0);
    internal static readonly UiColor Buff = new(.529412f, .807843f, .921569f);
    internal static readonly UiColor Strength = new(.164706f, .921569f, .745098f);
    internal static readonly UiColor Osty = new(.772549f, .854902f, .807843f);
    internal static readonly UiColor Gold = new(.937f, .784f, .318f);
    internal static readonly UiColor Cream = new(1, .964706f, .886275f);
    internal static readonly UiColor Dim = new(1, .964706f, .886275f, .5f);
    internal static readonly UiColor RowAlt = new(1, 1, 1, .025f);
    internal static readonly UiColor Track = new(1, 1, 1, .06f);
    internal static readonly UiColor Hover = new(1, 1, 1, .08f);
    internal static readonly UiColor Header = new(0, 0, 0, .25f);
    internal static readonly UiColor Shadow = new(0, 0, 0, .5f);
    internal static readonly UiColor HeaderShadow = new(0, 0, 0, .12549f);
    internal static readonly UiColor HeaderOutline = new(.33f, .2475f, 0);
    internal static readonly UiColor TipShadow = new(0, 0, 0, .25098f);
    internal static readonly UiColor Panel = new(.05f, .05f, .1f, .78f);
    internal static readonly UiColor Border = new(.3f, .4f, .7f, .6f);
    internal static (string Text, UiColor Color) Prefix(int kind) => kind switch
    {
        1 => ("[R] ", Gold), 3 => ("[P] ", Strength), 4 => ("[O] ", Osty), _ => ("", Cream)
    };
    internal static UiColor Segment(ChartSegment segment, ChartSection section, int kind) => segment switch
    {
        ChartSegment.Direct => section == ChartSection.Damage ? Damage : kind == 4 ? Osty : Block,
        ChartSegment.Attributed => Attributed, ChartSegment.Modifier => Modifier,
        ChartSegment.MitigateDebuff => Weak, ChartSegment.MitigateBuff => Buff,
        ChartSegment.MitigateStr => Strength, _ => Self
    };
    internal static readonly (string Label, UiColor Color)[] Legend =
    {
        ("direct", Damage), ("indirect", Attributed), ("modifier", Modifier), ("block", Block), ("osty", Osty),
        ("weak", Weak), ("buff", Buff), ("str down", Strength), ("self dmg", Self)
    };
}

internal sealed class PanelLayout
{
    internal const float PanelWidth = 780;
    internal readonly List<DrawCommand> Header = new();
    internal readonly List<DrawCommand> Body = new();
    internal readonly List<RowHit> RowHits = new();
    internal readonly List<TabHit> TabHits = new();
    internal readonly List<AvatarHit> AvatarHits = new();
    internal readonly List<string> PortraitPaths = new();
    internal ContentBox Content;
    internal float Width;
    internal float Height;
    internal float HeaderBottom;
    internal float StripH;
    internal bool HasChart;

    internal static ContentBox ContentArea(float width, bool plate, float gutter) => plate
        ? new(22, 16, width - 8 - 22 - 37 - gutter, 28) : new(12, 12, width - 24 - gutter, 12);

    internal static PanelLayout Chart(UiTab tab, IReadOnlyList<ChartRow> rows, ChartMeta meta, string footer,
        int? hover = null, bool skipChrome = false, IReadOnlyList<AvatarFact> avatars = null,
        bool flat = true, bool tabSprites = false, float width = PanelWidth, float gutter = 0)
    {
        var content = ContentArea(width, !flat, gutter);
        float strip = skipChrome ? 0 : 102;
        content = content with { Top = (skipChrome ? 0 : content.Top) + strip };
        var layout = new PanelLayout { Width = width, Content = content, StripH = strip, HasChart = true };
        var sink = new CommandSink(layout.Header);
        float y = content.Top;
        if (!skipChrome)
        {
            float x0 = content.X + Math.Max(0, (content.W - 520) / 2);
            for (int index = 0; index < 2; index++)
            {
                var current = (UiTab)index;
                float x = x0 + index * 264;
                if (tabSprites)
                {
                    sink.Texture(x, 0, 256, 90, IconId.TabPlate);
                    if (current == tab) sink.Texture(x, 0, 256, 90, IconId.TabStroke);
                }
                sink.Text(x, 56, 32, current == tab ? UiPalette.Cream : UiPalette.Dim,
                    current == UiTab.Combat ? "This Combat" : "Run Summary", TextRole.Title, TextEffect.Shadow, TextAlign.Center, 256);
                if (current == tab && !tabSprites) sink.Rect(x + 24, 82, 208, 2, UiPalette.Gold);
                layout.TabHits.Add(new(x, 0, x + 256, 90, current));
            }
            sink.Title(content.X, y + 30, "Contribution");
            y += 40;
            float avatarX = content.X;
            bool drew = false;
            if (avatars != null)
                for (int index = 0; index < avatars.Count; index++)
                {
                    var avatar = avatars[index];
                    layout.PortraitPaths.Add(avatar.Path);
                    if (!avatar.Loaded) continue;
                    sink.Texture(avatarX, y, 64, 64, IconId.Character, index);
                    layout.AvatarHits.Add(new(avatarX, y, avatarX + 64, y + 64, avatar.Slot));
                    avatarX += 70;
                    drew = true;
                }
            if (drew) y += 64;
            if (tab == UiTab.Combat && meta.Encounter.Length != 0)
            {
                sink.Text(content.X, y + 26, 24, UiPalette.Cream, "Vs. " + meta.Encounter, align: TextAlign.LeftClipped, width: content.W);
                y += 34;
            }
            sink.Text(content.X, y + 26, 24, UiPalette.Cream, MetaLine(tab, meta), TextRole.Title, align: TextAlign.LeftClipped, width: content.W);
            y += 34;
        }
        layout.HeaderBottom = y;
        sink = new CommandSink(layout.Body);
        float barX = content.X + 308;
        float barW = Math.Max(40, content.W - 516);
        float valueX = barX + barW + 8;
        foreach (var section in new[] { ChartSection.Damage, ChartSection.Defense })
        {
            sink.Rect(content.X, y, content.W, 38, UiPalette.Header);
            sink.Rect(content.X, y + 34, content.W, 2, UiPalette.Gold);
            sink.Text(content.X + 8, y + 26, 24, UiPalette.Gold, section.ToString(), TextRole.Title, TextEffect.Shadow);
            y += 38;
            bool any = false;
            for (int index = 0; index < rows.Count; index++)
            {
                var row = rows[index];
                if (row.Section != section) continue;
                any = true;
                bool hanging = row.SelfDamage && !row.SoloSelf;
                float baseline = y + 25;
                if (index % 2 == 0) sink.Rect(content.X, y, content.W, 32, UiPalette.RowAlt);
                if (hover == index) sink.Rect(content.X, y, content.W, 32, UiPalette.Hover);
                float nameX = content.X + 4 + (hanging ? 18 : 0);
                var nameColor = row.SelfDamage ? UiPalette.Self : UiPalette.Cream;
                if (hanging) sink.Text(nameX, baseline, 24, nameColor, "+ self damage");
                else
                {
                    var prefix = UiPalette.Prefix(row.Source.Kind);
                    if (prefix.Text.Length != 0)
                    {
                        sink.Text(nameX, baseline, 24, prefix.Color, prefix.Text);
                        nameX += 41;
                    }
                    sink.Text(nameX, baseline, 24, nameColor, ChartProjection.TruncateMarked(row.Name, 16));
                }
                if (row.Source.Plays > 0 && !hanging) sink.Text(content.X + 264, baseline, 24, UiPalette.Dim, $"x{row.Source.Plays}");
                sink.Rect(barX, y + 6, barW, 20, UiPalette.Track);
                float offset = 0;
                for (int segment = 0; segment < row.SegMilli.Count; segment++)
                {
                    float segmentWidth = (uint)row.SegMilli[segment] * (uint)barW / 1000;
                    if (segmentWidth > 0) sink.Rect(barX + offset, y + 6, segmentWidth, 20, UiPalette.Segment((ChartSegment)segment, section, row.Source.Kind));
                    offset += segmentWidth;
                }
                string value = row.SelfDamage ? row.Value.ToString(System.Globalization.CultureInfo.InvariantCulture)
                    : $"{row.Value}  ({row.ShareX10 / 10}.{row.ShareX10 % 10}%)";
                sink.Text(valueX, baseline, 24, nameColor, value, align: TextAlign.Right, width: content.Right - valueX);
                layout.RowHits.Add(new(y, y + 32, index));
                y += 32;
            }
            if (!any) { sink.Text(content.X + 8, y + 25, 24, UiPalette.Dim, "(none)"); y += 30; }
            y += 12;
        }
        y += 4;
        foreach (string line in footer.Split('\n').Take(64).Select(line => line.TrimEnd('\r')).Where(line => line.Length != 0))
        {
            sink.Text(content.X, y + 25, 24, UiPalette.Dim, line);
            y += 32;
        }
        layout.Height = y + content.OuterBottomPad;
        if (!skipChrome && flat) layout.Header.InsertRange(0, Borders(width, layout.Height));
        return layout;
    }

    internal static PanelLayout History(SummaryView view, IReadOnlyList<ChartRow> rows, ChartMeta meta,
        IReadOnlyList<AvatarFact> portraits, int? hover = null, float width = PanelWidth, bool flat = true, float gutter = 0)
    {
        var content = ContentArea(width, !flat, gutter);
        var layout = new PanelLayout { Width = width, Content = content, HasChart = view != null };
        var header = new CommandSink(layout.Header);
        var body = new CommandSink(layout.Body);
        float y = content.Top;
        header.Title(content.X, y + 30, "Run Summary");
        y += 40;
        if (view == null)
        {
            y += 6;
            layout.HeaderBottom = y;
            body.Text(content.X + 8, y + 25, 24, UiPalette.Dim, "no profiling history for this run");
            y += 30;
            body.Text(content.X + 8, y + 25, 24, UiPalette.Dim, "runs recorded by Spire Profiler appear here");
            layout.Height = y + 32 + content.OuterBottomPad;
        }
        else
        {
            float x = content.X;
            bool drew = false;
            for (int index = 0; index < portraits.Count; index++)
            {
                var portrait = portraits[index];
                layout.PortraitPaths.Add(portrait.Path);
                if (!portrait.Loaded) continue;
                header.Texture(x, y, 64, 64, IconId.Character, index);
                layout.AvatarHits.Add(new(x, y, x + 64, y + 64, portrait.Slot));
                x += 70;
                drew = true;
            }
            string identity = IdentityLine(view);
            string seed = "seed " + ChartProjection.TruncateBytes(view.Seed, 72);
            if (drew)
            {
                float left = x + 8;
                float remaining = Math.Max(0, content.Right - left);
                header.Text(left, y + 26, 24, UiPalette.Cream, identity, align: TextAlign.Right, width: remaining);
                header.Text(left, y + 54, 24, UiPalette.Dim, seed, align: TextAlign.Right, width: remaining);
                y += 64;
            }
            else
            {
                float left = content.X + 8;
                float remaining = Math.Max(0, content.Right - left);
                header.Text(left, y + 25, 24, UiPalette.Cream, identity, align: TextAlign.LeftClipped, width: remaining);
                y += 28;
                header.Text(left, y + 25, 24, UiPalette.Dim, seed, align: TextAlign.LeftClipped, width: remaining);
                y += 28;
            }
            header.Text(content.X, y + 26, 24, UiPalette.Cream, MetaLine(UiTab.Run, meta), TextRole.Title, align: TextAlign.LeftClipped, width: content.W);
            y += 40;
            layout.HeaderBottom = y;
            var chart = Chart(UiTab.Run, rows, meta, "", hover, skipChrome: true, flat: flat, width: width, gutter: gutter);
            foreach (var command in chart.Body)
                layout.Body.Add(command switch
                {
                    RectCommand rect => rect with { Y = rect.Y + y },
                    TextCommand text => text with { Y = text.Y + y },
                    TextureCommand texture => texture with { Y = texture.Y + y },
                    _ => throw new InvalidOperationException("Unknown chart command")
                });
            layout.RowHits.AddRange(chart.RowHits.Select(hit => hit with { Y0 = hit.Y0 + y, Y1 = hit.Y1 + y }));
            layout.Height = y + chart.Height;
        }
        if (flat) layout.Header.InsertRange(0, Borders(width, layout.Height));
        return layout;
    }

    internal static string IdentityLine(SummaryView view)
    {
        var parts = new List<string>();
        if (view.Ascension >= 0) parts.Add($"A{view.Ascension}");
        if (view.GameMode.Length != 0) parts.Add(view.GameMode);
        parts.Add(view.Outcome switch { "victory" => "Victory", "defeat" => "Defeat", "abandoned" => "Abandoned", _ => "Unfinished" });
        return string.Join(" · ", parts);
    }

    internal static string MetaLine(UiTab tab, ChartMeta meta)
    {
        if (tab == UiTab.Run) return meta.Turns == 0 ? $"DPS — · {meta.Combats} combats"
            : $"DPS {meta.DpsX10 / 10}.{meta.DpsX10 % 10} · {meta.Turns} turns · {meta.Combats} combats";
        return meta.Turns == 0 ? $"DPS — · {meta.Plays} plays"
            : $"DPS {meta.DpsX10 / 10}.{meta.DpsX10 % 10} · {meta.Turns} turns · {meta.Plays} plays · took {meta.DamageTaken}";
    }

    internal static IReadOnlyList<DrawCommand> Legend(UiPoint origin)
    {
        var commands = new List<DrawCommand>(18);
        var sink = new CommandSink(commands);
        float y = origin.Y;
        foreach (var entry in UiPalette.Legend)
        {
            sink.Rect(origin.X, y + 5, 20, 12, entry.Color);
            sink.Text(origin.X + 26, y + 18, 22, UiPalette.Dim, entry.Label);
            y += 24;
        }
        return commands;
    }

    internal static IReadOnlyList<DrawCommand> Borders(float width, float height) => new DrawCommand[]
    {
        new RectCommand(0, 0, width, 1, UiPalette.Border), new RectCommand(0, height - 1, width, 1, UiPalette.Border),
        new RectCommand(0, 0, 1, height, UiPalette.Border), new RectCommand(width - 1, 0, 1, height, UiPalette.Border)
    };
}

internal sealed class CommandSink
{
    private readonly List<DrawCommand> _commands;
    internal CommandSink(List<DrawCommand> commands) => _commands = commands;
    internal void Rect(float x, float y, float w, float h, UiColor color) => _commands.Add(new RectCommand(x, y, w, h, color));
    internal void Texture(float x, float y, float w, float h, IconId icon, int slot = 0) => _commands.Add(new TextureCommand(x, y, w, h, icon, slot));
    internal void Text(float x, float y, int size, UiColor color, string text, TextRole role = TextRole.Body,
        TextEffect effect = TextEffect.Plain, TextAlign align = TextAlign.Left, float width = -1)
    {
        if (text.Length != 0) _commands.Add(new TextCommand(x, y, size, color, role, effect, align, width, text));
    }
    internal void Title(float x, float y, string text) => Text(x, y, 32, UiPalette.Gold, text, TextRole.Title, TextEffect.Outline);
}
