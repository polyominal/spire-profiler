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
    private readonly Label _title;
    private readonly Label _subtitle;
    private readonly Label _seed;
    private readonly Label _meta;
    private readonly Label _coverage;
    private readonly Label _footer;
    private readonly Label _empty;
    private readonly HBoxContainer _players;
    private readonly Button _allPlayers;
    private readonly Dictionary<int, Button> _playerButtons = new();
    private IReadOnlyList<PlayerSummary> _roster = Array.Empty<PlayerSummary>();
    private readonly ChartSection _damage;
    private readonly ChartSection _defense;
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
        _title = theme.Label("", 28, true);
        _subtitle = theme.Label("", 20);
        _seed = theme.Label("", 18);
        _seed.ClipText = true;
        _seed.TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis;
        _seed.MouseFilter = Control.MouseFilterEnum.Pass;
        _players = new HBoxContainer();
        _allPlayers = theme.Button("All players", () => SelectPlayer(null));
        _players.AddChild(_allPlayers);
        _meta = theme.Label("", 20);
        _coverage = theme.Label("", 20);
        _coverage.AddThemeColorOverride("font_color", PanelTheme.Indirect);
        foreach (var label in new[] { _title, _subtitle, _meta, _coverage })
            label.AutowrapMode = TextServer.AutowrapMode.WordSmart;
        foreach (var control in new Control[] { _title, _subtitle, _seed, _players, _meta, _coverage })
            _header.AddChild(control);
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
        _empty = theme.Label("Statistics appear after observed combat events.");
        _body.AddChild(_empty);
        _damage = new ChartSection(theme, false);
        _defense = new ChartSection(theme, true);
        _body.AddChild(_damage.Root);
        _body.AddChild(_defense.Root);
        _footer = theme.Label("", 20);
        _footer.AutowrapMode = TextServer.AutowrapMode.WordSmart;
        _body.AddChild(_footer);
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
        RowCount = 0;
        if (_combatTab != null)
        {
            _combatTab.Modulate = _runSelected ? new Color(0.6f, 0.6f, 0.6f) : Colors.White;
            _runTab.Modulate = _runSelected ? Colors.White : new Color(0.6f, 0.6f, 0.6f);
        }
        _empty.Visible = view == null;
        _players.Visible = _meta.Visible = _damage.Root.Visible = _defense.Root.Visible = _footer.Visible = view != null;
        if (view == null)
        {
            _title.Text = _history ? "No profiler record for this run" : "No combat recorded yet";
            _subtitle.Hide();
            _seed.Hide();
            _coverage.Hide();
            _damage.Update(Array.Empty<ChartRow>());
            _defense.Update(Array.Empty<ChartRow>());
            return;
        }
        _title.Text = view.Title;
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
        _subtitle.Text = subtitleText;
        _subtitle.Visible = !string.IsNullOrEmpty(subtitleText);
        _seed.Visible = run && !string.IsNullOrEmpty(view.Seed);
        if (_seed.Visible)
        {
            _seed.Text = $"Seed: {view.Seed}";
            _seed.TooltipText = _seed.Text;
        }
        if (!_roster.SequenceEqual(view.Players))
        {
            foreach (var button in _playerButtons.Values) { _players.RemoveChild(button); button.QueueFree(); }
            _playerButtons.Clear();
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
                _players.AddChild(button);
                _playerButtons.Add(slot, button);
            }
            _roster = view.Players;
        }
        foreach (var (slot, button) in _playerButtons)
            button.Modulate = _player == null || _player == slot ? Colors.White : new Color(0.55f, 0.55f, 0.55f);
        long damage = view.Cards.Sum(card => card.DamageDealt);
        string perTurn = view.Turns == 0 ? "–" : ((double)damage / view.Turns).ToString("0.0", CultureInfo.InvariantCulture);
        string totals = $"{view.Turns} turns · {view.Plays} plays · {damage} damage · {perTurn}/turn";
        totals += run ? $"\n{view.Combats} combats · {view.DamageReceived} damage taken" : $" · {view.DamageReceived} damage taken";
        _meta.Text = totals;
        string coverage = ChartProjection.Coverage(view.Coverage);
        _coverage.Text = coverage;
        _coverage.Visible = coverage.Length != 0;
        var damageRows = ChartProjection.Rows(view.Cards, false, _player);
        var defenseRows = ChartProjection.Rows(view.Cards, true, _player);
        _damage.Update(damageRows);
        _defense.Update(defenseRows);
        RowCount = damageRows.Count + defenseRows.Count;
        _footer.Text = $"{view.Combats} combats · {view.DamageReceived} damage taken · {view.Cards.Sum(card => card.BlockGained)} block\nPotions {view.PotionsUsed} · Forge {view.Cards.Sum(card => card.Forge)}";
    }

    [SuppressMessage("Design", "CA1001", Justification = "Godot owns the chart subtree through the panel root.")]
    private sealed class ChartSection
    {
        internal VBoxContainer Root { get; } = new();
        private readonly PanelTheme _theme;
        private readonly VBoxContainer _rows = new();
        private readonly Label _empty;
        private readonly Dictionary<(int Player, int Kind, string Id, bool SelfDamage), ChartControls> _controls = new();

        internal ChartSection(PanelTheme theme, bool defense)
        {
            _theme = theme;
            Root.AddThemeConstantOverride("separation", 6);
            _rows.AddThemeConstantOverride("separation", 6);
            Root.AddChild(theme.Label(defense ? "Defense" : "Damage", 26, true));
            var legend = new HFlowContainer();
            var entries = defense
                ? new[] { ("block", PanelTheme.Block), ("osty", PanelTheme.Osty), ("modifier", PanelTheme.Modifier), ("weak", PanelTheme.Weak), ("buff", PanelTheme.Buff), ("str down", PanelTheme.Strength), ("self dmg", PanelTheme.SelfDamage) }
                : new[] { ("direct", PanelTheme.Damage), ("indirect", PanelTheme.Indirect), ("modifier", PanelTheme.Modifier) };
            foreach (var (text, color) in entries)
            {
                var key = new HBoxContainer();
                key.AddChild(new ColorRect { Color = color, CustomMinimumSize = new Vector2(12, 12), SizeFlagsVertical = Control.SizeFlags.ShrinkCenter, MouseFilter = Control.MouseFilterEnum.Ignore });
                key.AddChild(theme.Label(text, 17));
                legend.AddChild(key);
            }
            Root.AddChild(legend);
            _empty = theme.Label("None", 20);
            Root.AddChild(_empty);
            Root.AddChild(_rows);
        }

        internal void Update(IReadOnlyList<ChartRow> rows)
        {
            bool changed = rows.Count != _controls.Count || rows.Any(row =>
                !_controls.ContainsKey((row.Source.Player, row.Source.Kind, row.Source.Id, row.SelfDamage)));
            if (changed)
            {
                foreach (var control in _controls.Values) { _rows.RemoveChild(control.Root); control.Root.QueueFree(); }
                _controls.Clear();
            }
            _empty.Visible = rows.Count == 0;
            double maximum = rows.Count == 0 ? 1 : rows.Max(row => Math.Abs((double)row.Value));
            for (int index = 0; index < rows.Count; index++)
            {
                var row = rows[index];
                var key = (row.Source.Player, row.Source.Kind, row.Source.Id, row.SelfDamage);
                if (!_controls.TryGetValue(key, out var control))
                {
                    control = new ChartControls(_theme, row);
                    _controls.Add(key, control);
                    _rows.AddChild(control.Root);
                }
                if (control.Root.GetIndex() != index) _rows.MoveChild(control.Root, index);
                control.Update(row, maximum);
            }
        }
    }

    [SuppressMessage("Design", "CA1001", Justification = "Godot owns the row controls through the chart subtree.")]
    private sealed class ChartControls
    {
        internal HBoxContainer Root { get; }
        private readonly Label _plays;
        private readonly Label _value;
        private readonly Control _bar;
        private readonly string _name;
        private ChartRow _row;
        private double _maximum;

        internal ChartControls(PanelTheme theme, ChartRow row)
        {
            _name = SourceName(row.Source);
            string prefix = row.Source.Kind switch { 1 => "[R] ", 3 => "[P] ", 4 => "[O] ", _ => "" };
            Root = new HBoxContainer
            {
                CustomMinimumSize = new Vector2(0, 34),
                MouseFilter = Control.MouseFilterEnum.Pass,
            };
            var label = theme.Label((row.SelfDamage ? "+ " : prefix) + _name, 20);
            label.CustomMinimumSize = new Vector2(240, 0);
            label.SizeFlagsHorizontal = Control.SizeFlags.ExpandFill;
            label.TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis;
            label.ClipText = true;
            Root.AddChild(label);
            _plays = theme.Label("", 19);
            _plays.CustomMinimumSize = new Vector2(44, 0);
            _plays.ClipText = true;
            Root.AddChild(_plays);
            _bar = new Control
            {
                CustomMinimumSize = new Vector2(100, 28),
                SizeFlagsHorizontal = Control.SizeFlags.ExpandFill,
                MouseFilter = Control.MouseFilterEnum.Ignore,
            };
            _bar.Draw += () =>
            {
                float y = (_bar.Size.Y - 18) / 2;
                _bar.DrawRect(new Rect2(0, y, _bar.Size.X, 18), new Color(1, 1, 1, 0.06f));
                float x = 0;
                for (int index = 0; index < _row.Segments.Count; index++)
                {
                    float width = (float)(_row.Segments[index] / _maximum * _bar.Size.X);
                    if (width <= 0) continue;
                    _bar.DrawRect(new Rect2(x, y, width, 18), PanelTheme.SegmentColor((ChartSegment)index, _row.Defense, _row.Source.Kind));
                    x += width;
                }
            };
            _bar.Resized += _bar.QueueRedraw;
            Root.AddChild(_bar);
            _value = theme.Label("", 20);
            _value.CustomMinimumSize = new Vector2(166, 0);
            _value.ClipText = true;
            _value.HorizontalAlignment = HorizontalAlignment.Right;
            if (row.SelfDamage) _value.AddThemeColorOverride("font_color", PanelTheme.SelfDamage);
            Root.AddChild(_value);
        }

        internal void Update(ChartRow row, double maximum)
        {
            if (_row?.Source != row.Source) Root.TooltipText = ChartProjection.Detail(row.Source, _name);
            if (_row?.Source.Plays != row.Source.Plays) _plays.Text = row.SelfDamage ? "" : $"×{row.Source.Plays}";
            if (_row?.Value != row.Value || _row.Share != row.Share)
                _value.Text = row.SelfDamage ? row.Value.ToString(CultureInfo.InvariantCulture)
                    : string.Create(CultureInfo.InvariantCulture, $"{row.Value} ({row.Share:P1})");
            bool redraw = _row == null || _maximum != maximum || !_row.Segments.SequenceEqual(row.Segments);
            _row = row;
            _maximum = maximum;
            if (redraw) _bar.QueueRedraw();
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
