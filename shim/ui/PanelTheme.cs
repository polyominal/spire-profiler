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
    private readonly Texture2D _track = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/scrollbar_track_center.tres");
    private readonly Texture2D _edge = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/scrollbar_track_edge2.tres");
    private readonly Texture2D _train = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/scrollbar_train_large.tres");
    private readonly Texture2D _tab = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/settings_tab_selected.tres");
    private readonly Texture2D _stroke = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/settings_tab_stroke.tres");
    private readonly Dictionary<string, Texture2D> _portraits = new(StringComparer.Ordinal);
    private static bool _portraitOverflow;
    private readonly StyleBoxTexture _plate;
    private readonly StyleBoxTexture _shadow;
    internal bool HasPlate => _plate != null;
    internal bool HasScrollbar => _track != null && _edge != null && _train != null;
    internal bool HasTabs => _tab != null && _stroke != null;

    internal PanelTheme()
    {
        if (_plateTexture != null)
        {
            _plate = new StyleBoxTexture
            {
                Texture = _plateTexture, RegionRect = new Rect2(0, 0, 339, 107),
                TextureMarginLeft = 55, TextureMarginTop = 43, TextureMarginRight = 91, TextureMarginBottom = 32,
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

    internal Texture2D Portrait(string path)
    {
        if (_portraits.TryGetValue(path, out var texture)) return texture;
        if (_portraits.Count >= 8)
        {
            if (!_portraitOverflow) Log.Error($"[SpireProfiler] theme per-run asset cache full (8); icon skipped: {path}");
            _portraitOverflow = true;
            return null;
        }
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
        void Pass(float x, float y, UiColor color) => canvas.DrawString(font,
            new Vector2(command.X + offset.X + x, command.Y + offset.Y + y), command.Text, align, width, command.Size, Color(color));
        if (command.Effect == TextEffect.Shadow) Pass(3, 2, UiPalette.Shadow);
        if (command.Effect == TextEffect.Outline)
        {
            Pass(5, 4, UiPalette.HeaderShadow);
            Pass(-1, -1, UiPalette.HeaderOutline); Pass(1, -1, UiPalette.HeaderOutline);
            Pass(-1, 1, UiPalette.HeaderOutline); Pass(1, 1, UiPalette.HeaderOutline);
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

    internal void DrawTooltip(Control canvas, UiRect rect, IReadOnlyList<TipLine> lines, Font fallback, bool useFallback)
    {
        DrawPlate(canvas, rect);
        float y = rect.Y + 38;
        foreach (var line in lines)
        {
            var font = useFallback ? fallback : (line.Title ? _title : _body) ?? fallback;
            if (font != null)
            {
                var position = new Vector2(rect.X + 22, y);
                canvas.DrawString(font, position + new Vector2(3, 2), line.Text, fontSize: 22, modulate: Color(UiPalette.TipShadow));
                canvas.DrawString(font, position, line.Text, fontSize: 22, modulate: Color(line.Color));
                if (line.Value is { } value)
                {
                    position.X += 170;
                    canvas.DrawString(font, position + new Vector2(3, 2), value.Text, HorizontalAlignment.Right, 123, 22, Color(UiPalette.TipShadow));
                    canvas.DrawString(font, position, value.Text, HorizontalAlignment.Right, 123, 22, Color(value.Color));
                }
            }
            y += 26;
        }
    }

    internal void DrawScrollbar(Control canvas, ScrollbarGeometry geometry, float x)
    {
        if (!HasScrollbar || geometry == null) return;
        foreach (var (texture, rectangle) in new[] { (_track, geometry.Body), (_edge, geometry.CapTop), (_edge, geometry.CapBottom) })
            canvas.DrawTextureRect(texture, new Rect2(rectangle.X + x, rectangle.Y, rectangle.W, rectangle.H), false, new Color(.164706f, .290196f, .321569f));
        var grabber = geometry.Grabber;
        canvas.DrawTextureRect(_train, new Rect2(grabber.X + x, grabber.Y, grabber.W, grabber.H), false);
    }

    internal static bool NeedsFallback(PanelLayout layout, RowDetail detail)
        => layout.Header.Concat(layout.Body).OfType<TextCommand>().Any(text => !KreonCovers(text.Text))
            || !KreonCovers(detail.Title) || detail.Stats.Any(stat => !KreonCovers(stat.Label) || !KreonCovers(stat.Value));

    private static readonly (int Low, int High)[] Covered =
    {
        (0x20,0x7e),(0xa0,0x107),(0x10a,0x113),(0x116,0x11b),(0x11e,0x123),(0x126,0x127),(0x12a,0x12b),
        (0x12e,0x131),(0x136,0x137),(0x139,0x13e),(0x141,0x148),(0x14a,0x14d),(0x150,0x15b),(0x15e,0x167),
        (0x16a,0x16b),(0x16e,0x17e),(0x1cd,0x1dc),(0x218,0x21b),(0x2c6,0x2c7),(0x2d8,0x2dd),
        (0x300,0x304),(0x306,0x308),(0x30a,0x30c),(0x312,0x312),(0x323,0x324),(0x326,0x328),(0x3bc,0x3bc),
        (0x1e80,0x1e85),(0x1ef2,0x1ef3),(0x2013,0x2014),(0x2018,0x201a),(0x201c,0x201e),(0x2020,0x2022),
        (0x2026,0x2026),(0x2039,0x203a),(0x2044,0x2044),(0x20ac,0x20ac),(0x2122,0x2122),(0x215b,0x215e),
        (0x2212,0x2212),(0x2260,0x2260)
    };
    internal static bool KreonCovers(string text)
    {
        foreach (var rune in text.EnumerateRunes())
        {
            int code = rune.Value;
            if (code < 0x20 || code == 0x7f) continue;
            if (!Covered.Any(range => code >= range.Low && code <= range.High)) return false;
        }
        return true;
    }
    private static Color Color(UiColor color) => new(color.R, color.G, color.B, color.A);
    private static Rect2 Rect(UiRect rect) => new(rect.X, rect.Y, rect.W, rect.H);
    public void Dispose()
    {
        _shadow?.Dispose();
        _plate?.Dispose();
    }
}
