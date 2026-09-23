using System;
using Godot;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;

namespace SpireProfiler;

internal sealed class PanelTheme
{
    private readonly Font _body = Load<Font>("res://themes/kreon_regular_glyph_space_one.tres");
    private readonly Font _title = Load<Font>("res://themes/kreon_bold_glyph_space_two.tres");
    private readonly Texture2D _plate = Load<Texture2D>("res://images/ui/hover_tip.png");
    private readonly Texture2D _tab = Load<Texture2D>("res://images/atlases/ui_atlas.sprites/settings_tab_selected.tres");

    internal static readonly Color Damage = new("ff6464");
    internal static readonly Color Indirect = new("ffa518");
    internal static readonly Color Modifier = new("ee82ee");
    internal static readonly Color Block = new("67aeeb");
    internal static readonly Color Osty = new("c5dace");
    internal static readonly Color Weak = new("7fff00");
    internal static readonly Color Buff = new("87ceeb");
    internal static readonly Color Strength = new("2aebbe");
    internal static readonly Color SelfDamage = new("bf3030");

    private static T Load<T>(string path) where T : Resource =>
        ResourceLoader.Exists(path) ? GD.Load<T>(path) : null;

    internal void Apply(Control root)
    {
        var theme = new Theme { DefaultFontSize = 22 };
        if (_body != null) theme.DefaultFont = _body;
        theme.SetColor("font_color", "Label", StsColors.cream);
        theme.SetColor("font_color", "TooltipLabel", StsColors.cream);
        theme.SetStylebox("panel", "TooltipPanel", Plate());
        root.Theme = theme;
    }

    internal StyleBox Plate()
    {
        if (_plate != null)
            return new StyleBoxTexture
            {
                Texture = _plate,
                RegionRect = new Rect2(0, 0, 339, 107),
                TextureMarginLeft = 55,
                TextureMarginTop = 43,
                TextureMarginRight = 91,
                TextureMarginBottom = 32,
                ContentMarginLeft = 24,
                ContentMarginTop = 22,
                ContentMarginRight = 32,
                ContentMarginBottom = 22,
            };
        return new StyleBoxFlat
        {
            BgColor = new Color(0.05f, 0.05f, 0.10f, 0.97f),
            BorderColor = new Color(0.3f, 0.4f, 0.7f),
            BorderWidthBottom = 2,
            BorderWidthTop = 2,
            BorderWidthLeft = 2,
            BorderWidthRight = 2,
            ContentMarginLeft = 24,
            ContentMarginTop = 22,
            ContentMarginRight = 24,
            ContentMarginBottom = 22,
        };
    }

    internal Label Label(string text, int size = 22, bool title = false)
    {
        var label = new Label
        {
            Text = text,
            MouseFilter = Control.MouseFilterEnum.Ignore,
            VerticalAlignment = VerticalAlignment.Center,
        };
        label.AddThemeFontSizeOverride("font_size", size);
        if (title && _title != null) label.AddThemeFontOverride("font", _title);
        if (title) label.AddThemeColorOverride("font_color", StsColors.gold);
        return label;
    }

    internal Button Button(string text, Action pressed)
    {
        var button = new Button
        {
            Text = text,
            CustomMinimumSize = new Vector2(0, 48),
            FocusMode = Control.FocusModeEnum.None,
        };
        button.Pressed += () =>
        {
            try { pressed(); }
            catch (Exception ex) { Log.Error($"[SpireProfiler] panel action: {ex}"); }
        };
        if (_title != null) button.AddThemeFontOverride("font", _title);
        button.AddThemeFontSizeOverride("font_size", 22);
        button.AddThemeColorOverride("font_color", StsColors.cream);
        button.AddThemeColorOverride("font_hover_color", StsColors.gold);
        button.AddThemeColorOverride("font_pressed_color", StsColors.halfTransparentWhite);
        button.AddThemeStyleboxOverride("focus", new StyleBoxEmpty());
        if (_tab != null)
        {
            var box = new StyleBoxTexture
            {
                Texture = _tab,
                TextureMarginLeft = 24,
                TextureMarginTop = 24,
                TextureMarginRight = 24,
                TextureMarginBottom = 24,
                ContentMarginLeft = 14,
                ContentMarginRight = 14,
                ModulateColor = new Color(0.9f, 0.9f, 0.9f),
            };
            button.AddThemeStyleboxOverride("normal", box);
            button.AddThemeStyleboxOverride("hover", box);
            button.AddThemeStyleboxOverride("pressed", box);
            button.AddThemeStyleboxOverride("disabled", new StyleBoxEmpty());
        }
        return button;
    }

    internal static Texture2D Portrait(string character)
    {
        if (string.IsNullOrEmpty(character)) return null;
        foreach (char letter in character)
            if (!(char.IsAsciiLetterUpper(letter) || char.IsAsciiDigit(letter) || letter == '_')) return null;
        return Load<Texture2D>($"res://images/ui/top_panel/character_icon_{character.ToLowerInvariant()}.png");
    }

    internal static Color SegmentColor(ChartSegment segment, bool defense, int kind) => segment switch
    {
        ChartSegment.Direct => defense ? (kind == 4 ? Osty : Block) : Damage,
        ChartSegment.Indirect => Indirect,
        ChartSegment.Modifier => Modifier,
        ChartSegment.Weak => Weak,
        ChartSegment.Buff => Buff,
        ChartSegment.Strength => Strength,
        ChartSegment.SelfDamage => SelfDamage,
        _ => StsColors.cream,
    };
}
