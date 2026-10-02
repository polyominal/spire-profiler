using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using Godot;
using MegaCrit.Sts2.Core.Logging;

namespace SpireProfiler;

internal sealed class PanelTheme : IDisposable
{
    private readonly Font _title = Load<Font>("res://themes/kreon_bold_glyph_space_two.tres");
    private readonly Font _body = Load<Font>("res://themes/kreon_regular_glyph_space_one.tres");
    private readonly Texture2D _plateTexture = Load<Texture2D>("res://images/ui/hover_tip.png");
    private readonly Texture2D _tab = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/settings_tab_selected.tres");
    private readonly Texture2D _stroke = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/settings_tab_stroke.tres");
    private readonly Dictionary<string, Texture2D> _portraits = new(StringComparer.Ordinal);
    private readonly StyleBoxTexture _plate;
    private readonly StyleBoxTexture _shadow;
    internal bool HasPlate => _plate != null;
    internal bool HasTabs => _tab != null && _stroke != null;
    internal int RetainedPortraitCount => _portraits.Count;

    internal PanelTheme()
    {
        if (_plateTexture != null)
        {
            _plate = new StyleBoxTexture
            {
                Texture = _plateTexture,
                RegionRect = new Rect2(0, 0, 339, 107),
                TextureMarginLeft = 55,
                TextureMarginTop = 43,
                TextureMarginRight = 91,
                TextureMarginBottom = 32,
                AxisStretchHorizontal = StyleBoxTexture.AxisStretchMode.Tile,
                AxisStretchVertical = StyleBoxTexture.AxisStretchMode.Tile,
            };
            _shadow = (StyleBoxTexture)_plate.Duplicate();
            _shadow.ModulateColor = new Color(0, 0, 0, .25098f);
        }
    }

    private static T Load<T>(string path) where T : Resource
    {
        try
        {
            if (ResourceLoader.Exists(path))
            {
                var resource = GD.Load<T>(path);
                if (resource != null) return resource;
            }
            Log.Warn($"[SpireProfiler] theme asset unavailable: {path}");
        }
        catch (Exception error) { Log.Warn($"[SpireProfiler] theme asset unavailable: {path}: {error.Message}"); }
        return null;
    }

    internal static string PortraitPath(string character)
    {
        if (string.IsNullOrEmpty(character) || character.Any(letter => !(char.IsAsciiLetterUpper(letter) || char.IsAsciiDigit(letter) || letter == '_'))) return null;
        return $"res://images/ui/top_panel/character_icon_{character.ToLowerInvariant()}.png";
    }

    internal void RetainPortraits(IEnumerable<string> paths)
    {
        var retained = paths.ToHashSet(StringComparer.Ordinal);
        foreach (string path in _portraits.Keys.Where(path => !retained.Contains(path)).ToArray()) _portraits.Remove(path);
    }

    internal Texture2D Portrait(string path)
    {
        if (_portraits.TryGetValue(path, out var texture)) return texture;
        texture = Load<Texture2D>(path);
        _portraits.Add(path, texture);
        return texture;
    }

    internal void DrawPlate(Control canvas, UiRect rect)
    {
        if (_plate != null)
        {
            canvas.DrawStyleBox(_shadow, new Rect2(rect.X + 8, rect.Y + 8, rect.W - 8, rect.H - 8));
            canvas.DrawStyleBox(_plate, new Rect2(rect.X, rect.Y, rect.W - 8, rect.H - 8));
        }
        else
        {
            canvas.DrawRect(Rect(rect), Color(UiPalette.Panel));
            foreach (var command in PanelLayout.Borders(rect.W, rect.H).Cast<RectCommand>())
                canvas.DrawRect(new Rect2(rect.X + command.X, rect.Y + command.Y, command.W, command.H), Color(command.Color));
        }
    }

    internal void Replay(Control canvas, IReadOnlyList<DrawCommand> commands, UiPoint offset, Font fallback, bool useFallback,
        IReadOnlyList<AvatarFact> avatars, int? selected, IReadOnlyList<float> scales)
    {
        foreach (var command in commands)
        {
            switch (command)
            {
                case RectCommand rectangle:
                    canvas.DrawRect(new Rect2(rectangle.X + offset.X, rectangle.Y + offset.Y, rectangle.W, rectangle.H), Color(rectangle.Color));
                    break;
                case TextCommand text:
                    DrawText(canvas, text, offset, fallback, useFallback);
                    break;
                case TextureCommand image:
                    Texture2D texture = image.Icon switch { IconId.TabPlate => _tab, IconId.TabStroke => _stroke, _ => Portrait(avatars[image.Slot].Path) };
                    if (texture == null) break;
                    float scale = image.Icon == IconId.Character && image.Slot < scales.Count ? scales[image.Slot] : 1;
                    var rect = new Rect2(image.X + offset.X - image.W * (scale - 1) / 2, image.Y + offset.Y - image.H * (scale - 1) / 2, image.W * scale, image.H * scale);
                    var modulate = image.Icon switch
                    {
                        IconId.TabPlate => new Color(.9f, .9f, .9f),
                        IconId.TabStroke => new Color(.3648f, .9104f, .96f, .752941f),
                        _ => selected != null && selected != avatars[image.Slot].Slot ? new Color(.55f, .55f, .55f) : Colors.White
                    };
                    canvas.DrawTextureRect(texture, rect, false, modulate);
                    break;
            }
        }
    }

    private void DrawText(Control canvas, TextCommand command, UiPoint offset, Font fallback, bool useFallback)
    {
        var font = useFallback ? fallback : (command.Role == TextRole.Title ? _title : _body) ?? fallback;
        if (font == null) return;
        var align = command.Align switch { TextAlign.Center => HorizontalAlignment.Center, TextAlign.Right => HorizontalAlignment.Right, _ => HorizontalAlignment.Left };
        float width = command.Align == TextAlign.Left ? -1 : command.Width;
        using var line = new TextLine { Alignment = align, Width = width, TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis };
        line.AddString(command.Text, font, command.Size);
        var position = new Vector2(command.X + offset.X, command.Y + offset.Y - line.GetLineAscent());
        void Pass(float x, float y, UiColor color) => line.Draw(canvas.GetCanvasItem(), position + new Vector2(x, y), Color(color));
        if (command.Effect == TextEffect.Shadow) Pass(3, 2, UiPalette.Shadow);
        if (command.Effect == TextEffect.Outline)
        {
            Pass(5, 4, UiPalette.HeaderShadow);
            line.DrawOutline(canvas.GetCanvasItem(), position, 1, Color(UiPalette.HeaderOutline));
        }
        Pass(0, 0, command.Color);
    }

    internal void DrawLegend(Control canvas, UiRect rect, Font fallback, bool useFallback)
    {
        DrawPlate(canvas, rect);
        var (_, origin) = PanelGeometry.LegendPlate(HasPlate);
        Replay(canvas, PanelLayout.Legend(new(rect.X + origin.X, rect.Y + origin.Y)), default, fallback, useFallback,
            Array.Empty<AvatarFact>(), null, Array.Empty<float>());
    }

    internal TooltipLayout ShapeTooltip(RowDetail detail, float maximumHeight, Font fallback, bool useFallback)
        => new(detail, maximumHeight, useFallback ? fallback : _title ?? fallback, useFallback ? fallback : _body ?? fallback);

    internal bool NeedsFallback(PanelLayout layout, RowDetail detail)
        => layout.Header.Concat(layout.Body).OfType<TextCommand>().Any(text => !Covers(text.Role == TextRole.Title ? _title : _body, text.Text))
            || !Covers(_title, detail.Title) || detail.Stats.Any(stat => !Covers(_body, stat.Label) || !Covers(_body, stat.Value));

    private static bool Covers(Font font, string text)
    {
        if (font == null) return true;
        foreach (var rune in text.EnumerateRunes())
        {
            int code = rune.Value;
            if (code < 0x20 || code == 0x7f) continue;
            if (!font.HasChar(code)) return false;
        }
        return true;
    }
    private static Color Color(UiColor color) => new(color.R, color.G, color.B, color.A);
    private static Rect2 Rect(UiRect rect) => new(rect.X, rect.Y, rect.W, rect.H);
    public void Dispose()
    {
        _portraits.Clear();
        _shadow?.Dispose();
        _plate?.Dispose();
    }
}
