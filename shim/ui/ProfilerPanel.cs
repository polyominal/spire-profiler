using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Diagnostics.CodeAnalysis;
using System.Linq;
using Godot;
using MegaCrit.Sts2.Core.Logging;

namespace SpireProfiler;

[SuppressMessage("Design", "CA1001", Justification = "Free releases shaped tooltip text and queues the Godot control subtree for deletion.")]
internal sealed class ProfilerPanel
{
    private readonly PanelTheme _theme;
    private readonly bool _history;
    private readonly Control _canvas;
    private readonly Control _body;
    private readonly Control _overlay;
    private readonly VScrollBar _scrollbar;
    private readonly AvatarAnimation _animation = new();
    private Font _fallback;
    private SummaryView _combatView, _runView, _view;
    private IReadOnlyList<StatRow> _cards = Array.Empty<StatRow>();
    private IReadOnlyList<ChartRow> _rows = Array.Empty<ChartRow>();
    private IReadOnlyList<AvatarFact> _avatars = Array.Empty<AvatarFact>();
    private IReadOnlyList<int> _avatarSlots = Array.Empty<int>();
    private IReadOnlyList<float> _scales = Array.Empty<float>();
    private TooltipLayout _tooltip;
    private RowDetail _detail = RowDetail.Empty;
    private PanelLayout _layout;
    private UiRect _control, _plate, _frame;
    private UiRect? _legend, _tip;
    private float _originX, _gutter;
    private float Scroll => (float)_scrollbar.Value;
    private int? _hover, _player;
    private UiTab _tab;
    private ulong? _revision;
    private bool _manual, _available = true, _mouseDown, _fallbackFont;
    private bool _receivedSnapshots;
    private UiPoint _viewport;

    internal Control Root { get; }
    internal ColorRect Backdrop { get; }
    internal bool IsValid => GodotObject.IsInstanceValid(Root) && !Root.IsQueuedForDeletion();
    internal bool Visible => IsValid && Root.Visible;
    internal bool Requested => _manual;
    internal int DrawCount { get; private set; }
    internal int RowCount => _rows.Count;
    internal int? Player => _player;
    internal int ScrollPosition => (int)Scroll;
    internal PanelLayout Layout => _layout;
    internal IReadOnlyList<ChartRow> Rows => _rows;
    internal UiRect ControlRect => _control;
    internal UiTab Tab => _tab;

    internal ProfilerPanel(PanelTheme theme, bool history)
    {
        _theme = theme;
        _history = history;
        Root = new Control { Name = history ? "ProfilerHistory" : "ProfilerCombat", Visible = false, MouseFilter = Control.MouseFilterEnum.Ignore };
        Root.SetAnchorsAndOffsetsPreset(Control.LayoutPreset.FullRect);
        Backdrop = new ColorRect { Color = new Color(0, 0, 0, .8f), MouseFilter = Control.MouseFilterEnum.Stop, Visible = false };
        Backdrop.SetAnchorsAndOffsetsPreset(Control.LayoutPreset.FullRect);
        _canvas = new Control { Name = "Plate", ClipContents = true, MouseFilter = Control.MouseFilterEnum.Stop };
        _body = new Control { Name = "ChartBody", ClipContents = true, MouseFilter = Control.MouseFilterEnum.Ignore };
        _overlay = new Control { Name = "ChartOverlay", MouseFilter = Control.MouseFilterEnum.Ignore };
        _scrollbar = new VScrollBar { Name = "ChartScrollbar", CustomStep = 60, FocusMode = Control.FocusModeEnum.All, MouseFilter = Control.MouseFilterEnum.Pass };
        Root.AddChild(_canvas);
        _canvas.AddChild(_body);
        _canvas.AddChild(_scrollbar);
        _canvas.AddChild(_overlay);
        Root.TreeExiting += () => { _tooltip?.Dispose(); _tooltip = null; _revision = null; };
        // A drag can leave the panel before the frame driver observes its press.
        _scrollbar.GuiInput += input =>
        {
            if (input is InputEventMouseButton { ButtonIndex: MouseButton.Left, Pressed: true }) _mouseDown = true;
        };
        _scrollbar.ValueChanged += _ =>
        {
            try
            {
                if (_layout == null) return;
                UpdateFrame(); _body.QueueRedraw(); _overlay.QueueRedraw();
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] panel scroll: {error}"); }
        };
        _canvas.GuiInput += input =>
        {
            try
            {
                var position = input switch { InputEventMouse pointer => pointer.Position, InputEventGesture gesture => gesture.Position, _ => new Vector2(-1, -1) };
                var mouse = _canvas.GetGlobalTransform() * position;
                if (!Visible || !_control.Contains(mouse.X, mouse.Y)) return;
                float delta = input switch
                {
                    InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelUp } => -_scrollbar.CustomStep,
                    InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.WheelDown } => _scrollbar.CustomStep,
                    InputEventPanGesture pan => pan.Delta.Y,
                    _ => 0
                };
                if (float.IsFinite(delta)) _scrollbar.Value += delta;
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] panel scroll: {error}"); }
        };
        _canvas.Draw += () =>
        {
            try
            {
                if (_layout == null) return;
                _fallback ??= _canvas.GetThemeDefaultFont();
                if (_theme.HasPlate) _theme.DrawPlate(_canvas, new(_originX, _layout.StripH, _control.W, _control.H - _layout.StripH));
                else
                {
                    _canvas.DrawRect(new Rect2(_originX, _layout.StripH, _control.W, _control.H - _layout.StripH), new Color(.05f, .05f, .1f, .78f));
                    if (!_history) _canvas.DrawRect(new Rect2(_originX, 0, _control.W, _layout.HeaderBottom), new Color(.05f, .05f, .1f, .78f));
                }
                _theme.Replay(_canvas, _layout.Header, new(_originX, 0), _fallback, _fallbackFont, _avatars, _player, _scales);
                DrawCount++;
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] panel draw: {error}"); }
        };
        _body.Draw += () =>
        {
            try
            {
                if (_layout != null) _theme.Replay(_body, _layout.Body, new(0, -(_layout.HeaderBottom + Scroll)), _fallback, _fallbackFont, _avatars, _player, _scales);
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] chart draw: {error}"); }
        };
        _overlay.Draw += () =>
        {
            try
            {
                if (_layout == null) return;
                if (_legend is { } legend) _theme.DrawLegend(_overlay, legend, _fallback, _fallbackFont);
                if (_tip is { } tip) { _theme.DrawPlate(_overlay, tip); _tooltip?.Draw(_overlay, tip); }
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] overlay draw: {error}"); }
        };
    }

    internal void Show() { _manual = true; _revision = null; }
    internal void Hide() { _manual = false; }
    internal void SetAvailable(bool available)
    {
        _available = available;
        if (!IsValid) return;
        Root.Visible = _manual && available;
        Backdrop.Visible = Root.Visible;
        if (!Root.Visible)
        {
            _hover = null; _legend = _tip = null;
            _tooltip?.Dispose(); _tooltip = null; _detail = RowDetail.Empty;
            _animation.Clear(); _revision = null;
        }
    }
    internal void ResetFilter() { _player = null; _revision = null; }
    internal void SelectPlayer(int? slot)
    {
        var roster = _history ? _view?.Players : _receivedSnapshots ? _combatView?.Players : _view?.Players;
        if (roster == null || slot != null && !roster.Any(player => player.Slot == slot)) return;
        _player = _player == slot ? null : slot;
        Present(_view);
    }
    internal void SelectTab(UiTab tab)
    {
        if (_history || _tab == tab) return;
        _tab = tab;
        Present(tab == UiTab.Combat ? _combatView : _runView);
    }
    internal void Refresh(ulong revision, SummaryView combat, SummaryView run)
    {
        _combatView = combat; _runView = run;
        _receivedSnapshots = true;
        SetAvailable(_available);
        if (!Visible) return;
        var size = Root.GetViewport().GetVisibleRect().Size;
        var viewport = new UiPoint(size.X, size.Y);
        bool resized = viewport != _viewport;
        _viewport = viewport;
        if (_revision != revision || resized || _layout == null)
        {
            Present(_history || _tab == UiTab.Run ? run : combat);
            _revision = revision;
        }
        var pointer = Root.GetViewport().GetMousePosition();
        Interact(new(pointer.X, pointer.Y), Input.IsMouseButtonPressed(MouseButton.Left));
        if (_animation.AdvanceFrame((double)Stopwatch.GetTimestamp() / Stopwatch.Frequency))
        {
            _scales = _animation.Values;
            _canvas.QueueRedraw();
        }
    }

    internal void Present(SummaryView view)
    {
        _view = view;
        if (_history && _player != null && !(view?.Players.Any(player => player.Slot == _player) ?? false)) _player = null;
        _cards = view?.Cards ?? Array.Empty<StatRow>();
        if (_history && _player is { } slot && view.PlayerCards.TryGetValue(slot, out var playerCards)) _cards = playerCards;
        _rows = ChartProjection.Rows(_cards, _history ? null : _player);
        var roster = _history ? view?.Players : _receivedSnapshots ? _combatView?.Players : view?.Players;
        if (_history && roster?.Count == 0 && !string.IsNullOrEmpty(view.Character))
            roster = view.Character.Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries).Take(4)
                .Select((character, index) => new PlayerSummary(index, character)).ToArray();
        var portraits = (roster ?? Array.Empty<PlayerSummary>()).Select(player => (player.Slot, Path: PanelTheme.PortraitPath(player.Character)))
            .Where(player => player.Path != null).ToArray();
        _theme.RetainPortraits(portraits.Select(player => player.Path));
        _avatars = portraits.Select(player => new AvatarFact(player.Slot, _theme.Portrait(player.Path) != null, player.Path)).ToArray();
        _avatarSlots = _avatars.Select(avatar => avatar.Slot).ToArray();
        _animation.SetTargets(_player, _avatarSlots);
        _scales = _animation.Values;
        Rebuild();
    }

    private void Rebuild()
    {
        var metaView = _view == null ? null : _history ? _view with { Cards = _cards }
            : _tab == UiTab.Run ? _view with { DamageReceived = 0 } : _view;
        var meta = ChartProjection.Meta(metaView, _history ? UiTab.Run : _tab);
        for (int pass = 0; pass < 2; pass++)
        {
            _layout = _history
                ? PanelLayout.History(_view, _rows, meta, _avatars, _hover, flat: !_theme.HasPlate, gutter: _gutter)
                : PanelLayout.Chart(_tab, _rows, meta, ChartProjection.Footer(_view, _tab), _hover, avatars: _avatars, flat: !_theme.HasPlate, tabSprites: _theme.HasTabs, gutter: _gutter);
            float height = Math.Min(_layout.Height, PanelGeometry.HeightCap(_viewport.Y > 0 ? _viewport.Y : null));
            var position = PanelGeometry.Center(_viewport, new(_layout.Width, height));
            _control = new(position.X, position.Y, _layout.Width, height);
            _plate = new(position.X, position.Y + _layout.StripH, _layout.Width, height - _layout.StripH);
            var band = PanelGeometry.BodyBand(height, _theme.HasPlate, _layout.HeaderBottom);
            float page = band.Bottom - band.Top;
            float previousScroll = Scroll;
            _scrollbar.MaxValue = Math.Max(page, _layout.Height - height + page);
            _scrollbar.Page = page;
            _scrollbar.SetValueNoSignal(previousScroll);
            _scrollbar.Visible = _layout.Height > height && page >= _scrollbar.GetCombinedMinimumSize().Y;
            float gutter = _scrollbar.Visible ? Math.Max(20, _scrollbar.GetCombinedMinimumSize().X) + 12 : 0;
            if (gutter == _gutter) break;
            _gutter = gutter;
        }
        _detail = _hover is { } index ? ChartProjection.Detail(_rows, index) : RowDetail.Empty;
        _fallbackFont = _theme.NeedsFallback(_layout, _detail);
        _fallback ??= _canvas.GetThemeDefaultFont();
        _tooltip?.Dispose();
        _tooltip = null;
        if (!_detail.IsEmpty) _tooltip = _theme.ShapeTooltip(_detail, _control.H - 16, _fallback, _fallbackFont);
        UpdateFrame();
        _canvas.QueueRedraw(); _body.QueueRedraw(); _overlay.QueueRedraw();
    }

    internal void Interact(UiPoint mouse, bool pressed)
    {
        if (_layout == null || !Visible) return;
        var local = new UiPoint(mouse.X - _control.X, mouse.Y - _control.Y);
        bool onTrack = _scrollbar.Visible && _scrollbar.GetGlobalRect().HasPoint(new Vector2(mouse.X, mouse.Y));
        bool edge = pressed && !_mouseDown;
        _mouseDown = pressed;
        if (edge && !_control.Contains(mouse.X, mouse.Y)) Hide();
        if (edge && !onTrack && _control.Contains(mouse.X, mouse.Y))
        {
            foreach (var hit in _layout.TabHits)
                if (local.X >= hit.X0 && local.X < hit.X1 && local.Y >= hit.Y0 && local.Y < hit.Y1) { SelectTab(hit.Tab); break; }
            foreach (var hit in _layout.AvatarHits)
                if (local.X >= hit.X0 && local.X < hit.X1 && local.Y >= hit.Y0 && local.Y < hit.Y1) { SelectPlayer(hit.Slot); break; }
        }
        int? hover = onTrack ? null : PanelGeometry.Hover(_layout.RowHits, _control, mouse, Scroll, PanelGeometry.BodyBand(_control.H, _theme.HasPlate, _layout.HeaderBottom));
        if (hover != _hover) { _hover = hover; Rebuild(); }
    }

    private void UpdateFrame()
    {
        UiRect? legend = _layout.HasChart ? PanelGeometry.PlaceLegend(_viewport, _plate, PanelGeometry.LegendPlate(_theme.HasPlate).Size) : null;
        UiRect? tip = null;
        if (_hover is { } index && _tooltip != null)
        {
            var hit = _layout.RowHits.FirstOrDefault(hit => hit.FlatIndex == index);
            tip = PanelGeometry.PlaceTip(_viewport, _plate, _control.Y + hit.Y0 - Scroll, new(TooltipLayout.Width, _tooltip.Height), legend);
        }
        var (frame, originX) = PanelGeometry.Frame(_plate, legend, tip);
        _frame = frame with { Y = frame.Y - _layout.StripH, H = frame.H + _layout.StripH };
        _originX = originX;
        _canvas.Position = new(_frame.X, _frame.Y);
        _canvas.Size = new(_frame.W, _frame.H);
        var band = PanelGeometry.BodyBand(_control.H, _theme.HasPlate, _layout.HeaderBottom);
        _body.Position = new(_originX, band.Top);
        _body.Size = new(_control.W, band.Bottom - band.Top);
        float scrollbarWidth = Math.Max(20, _scrollbar.GetCombinedMinimumSize().X);
        _scrollbar.Position = new(_originX + _layout.Content.Right + 12, band.Top);
        _scrollbar.Size = new(scrollbarWidth, band.Bottom - band.Top);
        _overlay.Size = new(_frame.W, _frame.H);
        _legend = legend is { } key ? key with { X = key.X - _frame.X, Y = key.Y - _frame.Y } : null;
        _tip = tip is { } detail ? detail with { X = detail.X - _frame.X, Y = detail.Y - _frame.Y } : null;
    }

    internal void ScrollToEnd()
    {
        if (_layout == null) return;
        _scrollbar.Value = _scrollbar.MaxValue;
    }
    internal void Free()
    {
        _tooltip?.Dispose(); _tooltip = null;
        if (IsValid) Root.QueueFree();
        if (GodotObject.IsInstanceValid(Backdrop) && !Backdrop.IsQueuedForDeletion()) Backdrop.QueueFree();
    }
}
