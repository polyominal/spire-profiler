using System;
using System.Collections.Generic;
using System.Globalization;
using System.Diagnostics.CodeAnalysis;
using System.Linq;
using Godot;
using MegaCrit.Sts2.Core.Localization;

namespace SpireProfiler;

[SuppressMessage("Design", "CA1001", Justification = "Godot owns the node subtree; Free queues its root for deletion.")]
internal sealed class ProfilerPanel
{
    private readonly PanelTheme _theme;
    private readonly PanelContainer _plate;
    private readonly VBoxContainer _header;
    private readonly VBoxContainer _body;
    private readonly ScrollContainer _scroll;
    private readonly Button _combatTab;
    private readonly Button _runTab;
    private readonly bool _history;
    private SummaryView _view;
    private int? _player;
    private bool _runSelected;
    private ulong? _revision;

    internal Control Root { get; }
    internal bool IsValid => GodotObject.IsInstanceValid(Root) && !Root.IsQueuedForDeletion();
    internal bool Visible => IsValid && Root.Visible;
    internal int DrawCount { get; private set; }
    internal int RowCount { get; private set; }
    internal int? Player => _player;
    internal int ScrollPosition => _scroll.ScrollVertical;

    internal ProfilerPanel(PanelTheme theme, bool history)
    {
        _theme = theme;
        _history = history;
        Root = new Control { Name = history ? "ProfilerHistory" : "ProfilerCombat", Visible = false };
        Root.SetAnchorsAndOffsetsPreset(Control.LayoutPreset.FullRect);
        theme.Apply(Root);
        var backdrop = new ColorRect { Color = new Color(0, 0, 0, 0.8f) };
        backdrop.SetAnchorsAndOffsetsPreset(Control.LayoutPreset.FullRect);
        Root.AddChild(backdrop);
        backdrop.GuiInput += input =>
        {
            if (input is InputEventMouseButton { ButtonIndex: MouseButton.Left, Pressed: true }) Hide();
        };
        _plate = new PanelContainer { MouseFilter = Control.MouseFilterEnum.Stop };
        _plate.AddThemeStyleboxOverride("panel", theme.Plate());
        Root.AddChild(_plate);
        var content = new VBoxContainer();
        content.AddThemeConstantOverride("separation", 8);
        _plate.AddChild(content);
        var tabs = new HBoxContainer();
        if (!history)
        {
            _combatTab = theme.Button("This Combat", () => SelectTab(false));
            _runTab = theme.Button("Run Summary", () => SelectTab(true));
            tabs.AddChild(_combatTab);
            tabs.AddChild(_runTab);
        }
        else tabs.AddChild(theme.Label("Run Summary", 30, true));
        var spacer = new Control { SizeFlagsHorizontal = Control.SizeFlags.ExpandFill };
        tabs.AddChild(spacer);
        tabs.AddChild(theme.Button("Close [F8]", Hide));
        content.AddChild(tabs);
        _header = new VBoxContainer();
        content.AddChild(_header);
        _scroll = new ScrollContainer
        {
            SizeFlagsVertical = Control.SizeFlags.ExpandFill,
            HorizontalScrollMode = ScrollContainer.ScrollMode.Disabled,
            VerticalScrollMode = ScrollContainer.ScrollMode.Auto,
        };
        content.AddChild(_scroll);
        _body = new VBoxContainer { SizeFlagsHorizontal = Control.SizeFlags.ExpandFill };
        _body.AddThemeConstantOverride("separation", 6);
        _scroll.AddChild(_body);
        Root.Resized += Layout;
        _plate.Draw += () => DrawCount++;
    }

    internal void Show()
    {
        Root.Show();
        Layout();
        _revision = null;
    }

    internal void Hide()
    {
        if (IsValid) Root.Hide();
    }

    internal void Refresh(ulong revision, SummaryView combat, SummaryView run)
    {
        if (!Visible || _revision == revision) return;
        _revision = revision;
        Present(_history || _runSelected ? run : combat);
    }

    private void SelectTab(bool run)
    {
        _runSelected = run;
        _revision = null;
        _scroll.ScrollVertical = 0;
        Refresh(ProfilerSession.Revision, ProfilerSession.CurrentCombat, ProfilerSession.CurrentRun);
    }

    internal void SelectPlayer(int? slot)
    {
        _player = _player == slot ? null : slot;
        _scroll.ScrollVertical = 0;
        Present(_view);
    }

    internal void Present(SummaryView view)
    {
        _view = view;
        if (_player != null && !(view?.Players.Any(player => player.Slot == _player.Value) ?? false)) _player = null;
        foreach (Node child in _header.GetChildren()) { _header.RemoveChild(child); child.QueueFree(); }
        foreach (Node child in _body.GetChildren()) { _body.RemoveChild(child); child.QueueFree(); }
        RowCount = 0;
        if (_combatTab != null)
        {
            _combatTab.Modulate = _runSelected ? new Color(0.6f, 0.6f, 0.6f) : Colors.White;
            _runTab.Modulate = _runSelected ? Colors.White : new Color(0.6f, 0.6f, 0.6f);
        }
        if (view == null)
        {
            _header.AddChild(_theme.Label(_history ? "No profiler record for this run" : "No combat recorded yet", 26, true));
            _body.AddChild(_theme.Label("Statistics appear after observed combat events."));
            return;
        }
        var title = _theme.Label(view.Title, 28, true);
        title.AutowrapMode = TextServer.AutowrapMode.WordSmart;
        _header.AddChild(title);
        bool run = _history || _runSelected;
        string subtitleText = view.Subtitle;
        if (run)
        {
            string outcome = view.Outcome switch
            {
                "victory" => "Victory",
                "defeat" => "Defeat",
                "abandoned" => "Abandoned",
                "active" => "In progress",
                "suspended" => "Suspended",
                _ => "Unfinished",
            };
            subtitleText = string.IsNullOrEmpty(subtitleText) ? outcome : $"{subtitleText} · {outcome}";
        }
        if (!string.IsNullOrEmpty(subtitleText))
        {
            var subtitle = _theme.Label(subtitleText, 20);
            subtitle.AutowrapMode = TextServer.AutowrapMode.WordSmart;
            _header.AddChild(subtitle);
        }
        if (run && !string.IsNullOrEmpty(view.Seed))
        {
            var seed = _theme.Label($"Seed: {view.Seed}", 18);
            seed.ClipText = true;
            seed.TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis;
            seed.TooltipText = seed.Text;
            seed.MouseFilter = Control.MouseFilterEnum.Pass;
            _header.AddChild(seed);
        }
        var players = new HBoxContainer();
        players.AddChild(_theme.Button("All players", () => SelectPlayer(null)));
        foreach (var player in view.Players)
        {
            int slot = player.Slot;
            var button = _theme.Button($"P{slot + 1}", () => SelectPlayer(slot));
            var portrait = PanelTheme.Portrait(player.Character);
            if (portrait != null)
            {
                button.Icon = portrait;
                button.ExpandIcon = true;
                button.AddThemeConstantOverride("icon_max_width", 48);
            }
            button.TooltipText = $"Player {slot + 1}: {player.Character}";
            button.Modulate = _player == null || _player == slot ? Colors.White : new Color(0.55f, 0.55f, 0.55f);
            players.AddChild(button);
        }
        _header.AddChild(players);
        long damage = view.Cards.Sum(card => card.DamageDealt);
        string perTurn = view.Turns == 0 ? "–" : ((double)damage / view.Turns).ToString("0.0", CultureInfo.InvariantCulture);
        string totals = $"{view.Turns} turns · {view.Plays} plays · {damage} damage · {perTurn}/turn";
        totals += run ? $"\n{view.Combats} combats · {view.DamageReceived} damage taken" : $" · {view.DamageReceived} damage taken";
        var meta = _theme.Label(totals, 20);
        meta.AutowrapMode = TextServer.AutowrapMode.WordSmart;
        _header.AddChild(meta);
        string coverage = ChartProjection.Coverage(view.Coverage);
        if (coverage.Length != 0)
        {
            var warning = _theme.Label(coverage, 20);
            warning.AddThemeColorOverride("font_color", PanelTheme.Indirect);
            warning.AutowrapMode = TextServer.AutowrapMode.WordSmart;
            _header.AddChild(warning);
        }
        AddSection(view.Cards, false);
        AddSection(view.Cards, true);
        var footer = _theme.Label($"{view.Combats} combats · {view.DamageReceived} damage taken · {view.Cards.Sum(card => card.BlockGained)} block\nPotions {view.PotionsUsed} · Forge {view.Cards.Sum(card => card.Forge)}", 20);
        footer.AutowrapMode = TextServer.AutowrapMode.WordSmart;
        _body.AddChild(footer);
        Layout();
    }

    private void AddSection(IReadOnlyList<StatRow> cards, bool defense)
    {
        _body.AddChild(_theme.Label(defense ? "Defense" : "Damage", 26, true));
        var rows = ChartProjection.Rows(cards, defense, _player);
        var legend = new HFlowContainer();
        var entries = defense
            ? new[] { ("block", PanelTheme.Block), ("osty", PanelTheme.Osty), ("modifier", PanelTheme.Modifier), ("weak", PanelTheme.Weak), ("buff", PanelTheme.Buff), ("str down", PanelTheme.Strength), ("self dmg", PanelTheme.SelfDamage) }
            : new[] { ("direct", PanelTheme.Damage), ("indirect", PanelTheme.Indirect), ("modifier", PanelTheme.Modifier) };
        foreach (var (text, color) in entries)
        {
            var key = new HBoxContainer();
            key.AddChild(new ColorRect { Color = color, CustomMinimumSize = new Vector2(12, 12), SizeFlagsVertical = Control.SizeFlags.ShrinkCenter, MouseFilter = Control.MouseFilterEnum.Ignore });
            key.AddChild(_theme.Label(text, 17));
            legend.AddChild(key);
        }
        _body.AddChild(legend);
        if (rows.Count == 0) _body.AddChild(_theme.Label("None", 20));
        double maximum = rows.Count == 0 ? 1 : rows.Max(row => Math.Abs((double)row.Value));
        foreach (var row in rows)
        {
            string name = SourceName(row.Source);
            string prefix = row.Source.Kind switch { 1 => "[R] ", 3 => "[P] ", 4 => "[O] ", _ => "" };
            var line = new HBoxContainer
            {
                CustomMinimumSize = new Vector2(0, 34),
                TooltipText = ChartProjection.Detail(row.Source, name),
                MouseFilter = Control.MouseFilterEnum.Pass,
            };
            var label = _theme.Label((row.SelfDamage ? "+ " : prefix) + name, 20);
            label.CustomMinimumSize = new Vector2(240, 0);
            label.SizeFlagsHorizontal = Control.SizeFlags.ExpandFill;
            label.TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis;
            label.ClipText = true;
            line.AddChild(label);
            var plays = _theme.Label(row.SelfDamage ? "" : $"×{row.Source.Plays}", 19);
            plays.CustomMinimumSize = new Vector2(44, 0);
            plays.ClipText = true;
            line.AddChild(plays);
            var bar = new Control
            {
                CustomMinimumSize = new Vector2(100, 28),
                SizeFlagsHorizontal = Control.SizeFlags.ExpandFill,
                MouseFilter = Control.MouseFilterEnum.Ignore,
            };
            bar.Draw += () =>
            {
                float y = (bar.Size.Y - 18) / 2;
                bar.DrawRect(new Rect2(0, y, bar.Size.X, 18), new Color(1, 1, 1, 0.06f));
                float x = 0;
                for (int index = 0; index < row.Segments.Count; index++)
                {
                    float width = (float)(row.Segments[index] / maximum * bar.Size.X);
                    if (width <= 0) continue;
                    bar.DrawRect(new Rect2(x, y, width, 18), PanelTheme.SegmentColor((ChartSegment)index, defense, row.Source.Kind));
                    x += width;
                }
            };
            bar.Resized += bar.QueueRedraw;
            line.AddChild(bar);
            string valueText = row.SelfDamage ? row.Value.ToString(CultureInfo.InvariantCulture)
                : string.Create(CultureInfo.InvariantCulture, $"{row.Value} ({row.Share:P1})");
            var value = _theme.Label(valueText, 20);
            value.CustomMinimumSize = new Vector2(166, 0);
            value.ClipText = true;
            value.HorizontalAlignment = HorizontalAlignment.Right;
            if (row.SelfDamage) value.AddThemeColorOverride("font_color", PanelTheme.SelfDamage);
            line.AddChild(value);
            _body.AddChild(line);
            RowCount++;
        }
    }

    private static string SourceName(StatRow row)
    {
        string table = row.Kind switch { 0 => "cards", 1 => "relics", 2 => "powers", 3 => "potions", _ => null };
        if (table == null) return row.Id;
        try { return LocString.GetIfExists(table, row.Id + ".title")?.GetFormattedText() ?? row.Id; }
        catch (Exception) { return row.Id; }
    }

    private void Layout()
    {
        if (!IsValid) return;
        Vector2 viewport = Root.Size;
        _plate.Position = new Vector2(Math.Max(16, (viewport.X - 840) / 2), Math.Max(16, viewport.Y * 0.075f));
        _plate.Size = new Vector2(Math.Min(840, Math.Max(320, viewport.X - 32)), Math.Max(240, viewport.Y * 0.85f));
    }

    internal void ScrollToEnd() => _scroll.ScrollVertical = (int)_scroll.GetVScrollBar().MaxValue;

    internal void Free()
    {
        if (IsValid) Root.QueueFree();
    }
}
