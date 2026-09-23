using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Diagnostics.CodeAnalysis;
using System.Linq;
using Godot;
using MegaCrit.Sts2.Core.Logging;

namespace SpireProfiler;

[SuppressMessage("Design", "CA1001", Justification = "Godot owns and frees the control subtree through Root.")]
internal sealed class ProfilerPanel
{
    private readonly PanelTheme _theme;
    private readonly bool _history;
    private readonly Control _canvas;
    private readonly Control _body;
    private readonly Control _overlay;
    private readonly AvatarAnimation _animation = new();
    private Font _fallback;
    private SummaryView _combatView, _runView, _view;
    private IReadOnlyList<StatRow> _cards = Array.Empty<StatRow>();
    private IReadOnlyList<ChartRow> _rows = Array.Empty<ChartRow>();
    private IReadOnlyList<AvatarFact> _avatars = Array.Empty<AvatarFact>();
    private IReadOnlyList<int> _avatarSlots = Array.Empty<int>();
    private IReadOnlyList<float> _scales = Array.Empty<float>();
    private IReadOnlyList<TipLine> _tipLines = Array.Empty<TipLine>();
    private RowDetail _detail = RowDetail.Empty;
    private PanelLayout _layout;
    private UiRect _control, _plate, _frame;
    private UiRect? _legend, _tip;
    private float _originX, _scroll, _queuedScroll, _gutter;
    private int? _hover, _player;
    private UiTab _tab;
    private ulong? _revision;
    private bool _manual, _available = true, _mouseDown, _dragging, _fallbackFont;
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
    internal int ScrollPosition => (int)_scroll;
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
        Root.AddChild(_canvas);
        _canvas.AddChild(_body);
        _canvas.AddChild(_overlay);
        _canvas.GuiInput += input =>
        {
            try
            {
                float delta = input switch
                {
                    InputEventMouseButton button => PanelGeometry.EventScrollDelta((int)button.ButtonIndex, button.Pressed, 0),
                    InputEventPanGesture pan => pan.Delta.Y,
                    _ => 0
                };
                QueueScroll(delta);
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
                if (_layout != null) _theme.Replay(_body, _layout.Body, new(0, -(_layout.HeaderBottom + _scroll)), _fallback, _fallbackFont, _avatars, _player, _scales);
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] chart draw: {error}"); }
        };
        _overlay.Draw += () =>
        {
            try
            {
                if (_layout == null) return;
                _theme.DrawScrollbar(_overlay, Scrollbar(), _originX);
                if (_legend is { } legend) _theme.DrawLegend(_overlay, legend, _fallback, _fallbackFont);
                if (_tip is { } tip) _theme.DrawTooltip(_overlay, tip, _tipLines, _fallback, _fallbackFont);
            }
            catch (Exception error) { Log.Error($"[SpireProfiler] overlay draw: {error}"); }
        };
    }

    internal void Show() { _manual = true; _queuedScroll = 0; _revision = null; }
    internal void Hide() { _manual = false; _queuedScroll = 0; }
    internal void SetAvailable(bool available)
    {
        _available = available;
        if (!IsValid) return;
        Root.Visible = _manual && available;
        Backdrop.Visible = Root.Visible;
        if (!Root.Visible)
        {
            _queuedScroll = 0; _hover = null; _legend = _tip = null;
            _tipLines = Array.Empty<TipLine>(); _detail = RowDetail.Empty;
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
    internal void QueueScroll(float delta)
    {
        if (!Visible) { _queuedScroll = 0; return; }
        if (!float.IsFinite(delta)) { Log.Error("[SpireProfiler] non-finite scroll delta ignored"); return; }
        _queuedScroll = (float)Math.Clamp((double)_queuedScroll + delta, -float.MaxValue, float.MaxValue);
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
        _avatars = (roster ?? Array.Empty<PlayerSummary>()).Select(player => (player.Slot, Path: PanelTheme.PortraitPath(player.Character)))
            .Where(player => player.Path != null).Select(player => new AvatarFact(player.Slot, _theme.Portrait(player.Path) != null, player.Path)).ToArray();
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
            _scroll = Math.Min(_scroll, Math.Max(0, _layout.Height - height));
            float gutter = _layout.Height > height && _theme.HasScrollbar ? 32 : 0;
            if (gutter == _gutter) break;
            _gutter = gutter;
        }
        _detail = _hover is { } index ? ChartProjection.Detail(_rows, index, _cards) : RowDetail.Empty;
        _tipLines = _detail.IsEmpty ? Array.Empty<TipLine>() : TooltipLayout.Shape(_detail, TooltipLayout.MaximumLines(_control.H));
        _fallbackFont = PanelTheme.NeedsFallback(_layout, _detail);
        UpdateFrame();
        _canvas.QueueRedraw(); _body.QueueRedraw(); _overlay.QueueRedraw();
    }

    internal void Interact(UiPoint mouse, bool pressed)
    {
        if (_layout == null || !Visible) return;
        var scrollbar = Scrollbar();
        var local = new UiPoint(mouse.X - _control.X, mouse.Y - _control.Y);
        bool onTrack = scrollbar?.Track.Contains(local.X, local.Y) == true;
        bool edge = pressed && !_mouseDown;
        _dragging = PanelGeometry.DragState(_dragging, pressed, _mouseDown, onTrack);
        float oldScroll = _scroll;
        if (_dragging && scrollbar != null) _scroll = PanelGeometry.TrackScroll(scrollbar.Track, local.Y, Math.Max(0, _layout.Height - _control.H));
        _mouseDown = pressed;
        if (edge && !_control.Contains(mouse.X, mouse.Y)) Hide();
        if (edge && !onTrack && _control.Contains(mouse.X, mouse.Y))
        {
            foreach (var hit in _layout.TabHits)
                if (local.X >= hit.X0 && local.X < hit.X1 && local.Y >= hit.Y0 && local.Y < hit.Y1) { SelectTab(hit.Tab); break; }
            foreach (var hit in _layout.AvatarHits)
                if (local.X >= hit.X0 && local.X < hit.X1 && local.Y >= hit.Y0 && local.Y < hit.Y1) { SelectPlayer(hit.Slot); break; }
        }
        if (_control.Contains(mouse.X, mouse.Y)) _scroll = PanelGeometry.ApplyScroll(_scroll, _queuedScroll, _control.H, _layout.Height);
        _queuedScroll = 0;
        int? hover = PanelGeometry.Hover(_layout.RowHits, _control, mouse, _scroll, PanelGeometry.BodyBand(_control.H, _theme.HasPlate, _layout.HeaderBottom));
        if (hover != _hover) { _hover = hover; Rebuild(); }
        else if (_scroll != oldScroll) { UpdateFrame(); _body.QueueRedraw(); _overlay.QueueRedraw(); }
    }

    private ScrollbarGeometry Scrollbar() => !_theme.HasScrollbar || _layout == null ? null
        : PanelGeometry.Scrollbar(new(_control.W, _control.H), _theme.HasPlate, PanelGeometry.BodyBand(_control.H, _theme.HasPlate, _layout.HeaderBottom), _layout.Height, _scroll);

    private void UpdateFrame()
    {
        UiRect? legend = _layout.HasChart ? PanelGeometry.PlaceLegend(_viewport, _plate, PanelGeometry.LegendPlate(_theme.HasPlate).Size) : null;
        UiRect? tip = null;
        if (_hover is { } index && _tipLines.Count != 0)
        {
            var hit = _layout.RowHits.FirstOrDefault(hit => hit.FlatIndex == index);
            tip = PanelGeometry.PlaceTip(_viewport, _plate, _control.Y + hit.Y0 - _scroll, new(TooltipLayout.Width, TooltipLayout.Height(_tipLines.Count)), legend);
        }
        var (frame, originX) = PanelGeometry.Frame(_plate, legend, tip);
        _frame = frame with { Y = frame.Y - _layout.StripH, H = frame.H + _layout.StripH };
        _originX = originX;
        _canvas.Position = new(_frame.X, _frame.Y);
        _canvas.Size = new(_frame.W, _frame.H);
        var band = PanelGeometry.BodyBand(_control.H, _theme.HasPlate, _layout.HeaderBottom);
        _body.Position = new(_originX, band.Top);
        _body.Size = new(_control.W, band.Bottom - band.Top);
        _overlay.Size = new(_frame.W, _frame.H);
        _legend = legend is { } key ? key with { X = key.X - _frame.X, Y = key.Y - _frame.Y } : null;
        _tip = tip is { } detail ? detail with { X = detail.X - _frame.X, Y = detail.Y - _frame.Y } : null;
    }

    internal void ScrollToEnd()
    {
        if (_layout == null) return;
        _scroll = Math.Max(0, _layout.Height - _control.H);
        UpdateFrame(); _body.QueueRedraw(); _overlay.QueueRedraw();
    }
    internal void Free()
    {
        if (IsValid) Root.QueueFree();
        if (GodotObject.IsInstanceValid(Backdrop) && !Backdrop.IsQueuedForDeletion()) Backdrop.QueueFree();
    }
}
