using System;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Threading.Tasks;
using Godot;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Screens.RunHistoryScreen;

namespace SpireProfiler;

internal static class ProfilerPanels
{
    private static SceneTree _tree;
    private static PanelTheme _combatTheme, _historyTheme;
    private static ProfilerPanel _combat, _history;
    private static Button _historyButton;
    private static bool _f8Pressed;
    private static ulong _liveGeneration, _historyGeneration;

    internal static async Task AttachPanelAsync(string modDir)
    {
        try
        {
            if (_tree != null && GodotObject.IsInstanceValid(_tree)) return;
            SceneTree tree;
            while ((tree = Engine.GetMainLoop() as SceneTree) == null || tree.Root == null) await Task.Delay(100);
            _tree = tree;
            _combatTheme = new PanelTheme();
            _combat = new ProfilerPanel(_combatTheme, false);
            tree.Root.CallDeferred(Node.MethodName.AddChild, _combat.Backdrop);
            tree.Root.CallDeferred(Node.MethodName.AddChild, _combat.Root);
            tree.ProcessFrame += OnProcessFrame;
            tree.Root.TreeExiting += Detach;
            Log.Info("[SpireProfiler] managed profiler panel attached");
        }
        catch (Exception error) { Log.Error($"[SpireProfiler] panel attach failed: {error}"); }
    }

    private static void Detach()
    {
        if (_tree != null && GodotObject.IsInstanceValid(_tree))
        {
            _tree.ProcessFrame -= OnProcessFrame;
            if (GodotObject.IsInstanceValid(_tree.Root)) _tree.Root.TreeExiting -= Detach;
        }
        _combat = _history = null;
        _historyButton = null;
        _tree = null;
        _combatTheme?.Dispose();
        _historyTheme?.Dispose();
        _combatTheme = _historyTheme = null;
        _f8Pressed = false;
    }

    private static void OnProcessFrame()
    {
        try
        {
            if (_liveGeneration != ProfilerSession.LiveFilterGeneration)
            {
                _liveGeneration = ProfilerSession.LiveFilterGeneration;
                _combat?.ResetFilter();
            }
            if (_historyGeneration != ProfilerSession.HistoryClearGeneration)
            {
                _historyGeneration = ProfilerSession.HistoryClearGeneration;
                _history?.ResetFilter();
                _history?.Hide();
            }
            _combat?.SetAvailable(ProfilerSession.InRun);
            _history?.SetAvailable(ProfilerSession.HistoryOpen);
            if (_combat?.Visible == true || _history?.Visible == true) ProfilerSession.Refresh();
            _combat?.Refresh(ProfilerSession.Revision, ProfilerSession.CurrentCombat, ProfilerSession.CurrentRun);
            _history?.Refresh(ProfilerSession.Revision, null, ProfilerSession.SelectedHistory);
            if (_historyButton != null && GodotObject.IsInstanceValid(_historyButton)) _historyButton.Disabled = _history?.Visible == true;
            bool f8 = Input.IsKeyPressed(Key.F8);
            if (f8 && !_f8Pressed) Toggle();
            _f8Pressed = f8;
        }
        catch (Exception error)
        {
            _combat?.Hide(); _history?.Hide();
            Log.Error($"[SpireProfiler] panel refresh: {error}");
        }
    }

    private static void Toggle()
    {
        var panel = ProfilerSession.HistoryOpen ? _history : ProfilerSession.InRun ? _combat : null;
        if (panel?.IsValid != true) return;
        if (panel.Requested) panel.Hide(); else panel.Show();
    }

    internal static void AttachRunPanelTo(NRunHistory screen)
    {
        try
        {
            if (_history?.IsValid != true)
            {
                _historyTheme?.Dispose();
                _historyTheme = new PanelTheme();
                _history = new ProfilerPanel(_historyTheme, true);
            }
            if (_historyButton == null || !GodotObject.IsInstanceValid(_historyButton))
            {
                _historyButton = new Button { Text = "Profiler [F8]" };
                StyleRunButton(_historyButton);
                _historyButton.Pressed += () =>
                {
                    try { Toggle(); }
                    catch (Exception error) { Log.Error($"[SpireProfiler] run history button: {error}"); }
                };
            }
            PlaceRunButton(screen);
            foreach (var control in new Control[] { _history.Backdrop, _historyButton, _history.Root })
                if (control.GetParent() != screen)
                {
                    control.GetParent()?.RemoveChild(control);
                    screen.CallDeferred(Node.MethodName.AddChild, control);
                }
        }
        catch (Exception error) { Log.Error($"[SpireProfiler] run panel attach: {error}"); }
    }

    private static void PlaceRunButton(NRunHistory screen)
    {
        const float gap = 16.0f;
        var host = screen.GetRect().Size;
        var share = screen.GetNodeOrNull<Control>("ShareButton");
        var shareRect = share?.GetRect() ?? new Rect2();
        var size = shareRect.Size.X > 0.0f ? shareRect.Size : new Vector2(172.0f, 64.0f);
        Vector2 origin;
        if (shareRect.Size.X > 0.0f)
        {
            origin = new Vector2(shareRect.End.X - size.X, shareRect.Position.Y - gap - size.Y);
        }
        else
        {
            Log.Warn("[SpireProfiler] run button: ShareButton node not found; parked above its scene corner");
            origin = new Vector2(host.X - 220.0f, host.Y - 112.0f - gap - size.Y);
        }
        _historyButton.SetAnchorsPreset(Control.LayoutPreset.BottomRight);
        _historyButton.OffsetLeft = origin.X - host.X;
        _historyButton.OffsetTop = origin.Y - host.Y;
        _historyButton.OffsetRight = origin.X + size.X - host.X;
        _historyButton.OffsetBottom = origin.Y + size.Y - host.Y;
    }

    private static void StyleRunButton(Button button)
    {
        try
        {
            var font = GD.Load<FontVariation>("res://themes/kreon_bold_glyph_space_one.tres");
            if (font == null)
            {
                Log.Warn("[SpireProfiler] run button: kreon_bold_glyph_space_one.tres missing; default font kept");
            }
            else
            {
                button.AddThemeFontOverride("font", font);
                button.AddThemeFontSizeOverride("font_size", 22);
            }
            var plate = GD.Load<Texture2D>("res://images/atlases/ui_atlas.sprites/settings_tab_selected.tres");
            if (plate == null)
            {
                Log.Warn("[SpireProfiler] run button: settings_tab_selected.tres missing; chrome-less text button kept");
            }
            else
            {
                var box = new StyleBoxTexture
                {
                    Texture = plate,
                    TextureMarginLeft = 24,
                    TextureMarginTop = 24,
                    TextureMarginRight = 24,
                    TextureMarginBottom = 24,
                    ModulateColor = new Color(0.9f, 0.9f, 0.9f, 1.0f),
                };
                button.AddThemeStyleboxOverride("normal", box);
                button.AddThemeStyleboxOverride("hover", box);
                button.AddThemeStyleboxOverride("pressed", box);
                var disabledBox = new StyleBoxTexture
                {
                    Texture = plate,
                    TextureMarginLeft = 24,
                    TextureMarginTop = 24,
                    TextureMarginRight = 24,
                    TextureMarginBottom = 24,
                    ModulateColor = new Color(0.18f, 0.18f, 0.18f, 1.0f),
                };
                button.AddThemeStyleboxOverride("disabled", disabledBox);
            }
            button.AddThemeColorOverride("font_color", StsColors.cream);
            button.AddThemeColorOverride("font_hover_color", StsColors.gold);
            button.AddThemeColorOverride("font_pressed_color", StsColors.halfTransparentWhite);
            button.AddThemeColorOverride("font_disabled_color", StsColors.halfTransparentWhite);
            button.AddThemeColorOverride("font_focus_color", StsColors.cream);
            button.AddThemeStyleboxOverride("focus", new StyleBoxEmpty());
        }
        catch (Exception ex) { Log.Warn($"[SpireProfiler] run button styling: {ex.Message}"); }
    }

    internal static async Task SelfTestAsync()
    {
        ProfilerPanel panel = null;
        try
        {
            var tree = Engine.GetMainLoop() as SceneTree ?? throw new InvalidOperationException("Panel fixture needs the game scene tree");
            var fixture = new SummaryView
            {
                Title = "CULTIST",
                Character = "IRONCLAD,SILENT",
                GameMode = "Standard",
                Ascension = 10,
                Seed = "PANEL-FIXTURE",
                Outcome = "victory",
                Players = new[] { new PlayerSummary(0, "IRONCLAD"), new PlayerSummary(1, "SILENT") },
                Cards = Enumerable.Range(0, 80).Select(index => new StatRow
                {
                    Id = index == 0 ? "STRIKE_IRONCLAD" : $"FIXTURE_{index}",
                    Player = index % 2,
                    Plays = 2,
                    DmgDirect = index + 1,
                    DamageDealt = index + 1,
                    BlockEffective = 10,
                    SelfDamage = 1,
                    Forge = 3,
                }).ToArray(),
                Coverage = CoverageSummary.Healthy,
                Turns = 3,
                Plays = 160,
            };
            fixture = fixture with { PlayerCards = fixture.Cards.GroupBy(card => card.Player).ToDictionary(group => group.Key, group => (System.Collections.Generic.IReadOnlyList<StatRow>)group.ToArray()) };
            int draws = 0, refreshes = 0;
            long refreshTicks = 0, refreshAllocated = 0;
            for (int cycle = 0; cycle < 2; cycle++)
            {
                panel = new ProfilerPanel(_combatTheme ??= new PanelTheme(), cycle == 1);
                tree.Root.CallDeferred(Node.MethodName.AddChild, panel.Backdrop);
                tree.Root.CallDeferred(Node.MethodName.AddChild, panel.Root);
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                panel.Show();
                if (panel.Visible) throw new InvalidOperationException("Panel toggle changed visibility before refresh");
                panel.Refresh(0, fixture, fixture);
                for (int frame = 0; frame < 3; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                if (!panel.Root.IsInsideTree() || panel.RowCount != 240) throw new InvalidOperationException("Panel fixture failed attachment or chart projection");
                panel.ScrollToEnd();
                if (panel.ScrollPosition <= 0) throw new InvalidOperationException("Panel fixture failed scrolling");
                if (cycle == 0)
                {
                    int scroll = panel.ScrollPosition;
                    var hit = panel.Layout.TabHits.Single(hit => hit.Tab == UiTab.Run);
                    var point = new UiPoint(panel.ControlRect.X + hit.X0 + 8, panel.ControlRect.Y + hit.Y0 + 8);
                    panel.Interact(point, false); panel.Interact(point, true); panel.Interact(point, false);
                    int expectedScroll = Math.Min(scroll, (int)Math.Max(0, panel.Layout.Height - panel.ControlRect.H));
                    if (panel.Tab != UiTab.Run || panel.ScrollPosition != expectedScroll)
                        throw new InvalidOperationException("Tab press did not preserve and clamp the scroll offset");
                    panel.Refresh(1, fixture, fixture with { Players = Array.Empty<PlayerSummary>() });
                    panel.SelectPlayer(1);
                    if (panel.Player != 1 || panel.RowCount != 120)
                        throw new InvalidOperationException("Run-tab filtering did not use the retained combat roster");
                    panel.SelectPlayer(1);
                    panel.Refresh(2, fixture, fixture);
                    hit = panel.Layout.TabHits.Single(hit => hit.Tab == UiTab.Combat);
                    point = new(panel.ControlRect.X + hit.X0 + 8, panel.ControlRect.Y + hit.Y0 + 8);
                    panel.Interact(point, true); panel.Interact(point, false);
                    if (panel.Tab != UiTab.Combat) throw new InvalidOperationException("Combat tab press did not switch back");
                }
                var avatar = panel.Layout.AvatarHits.FirstOrDefault(hit => hit.Slot == 1);
                var avatarPoint = new UiPoint(panel.ControlRect.X + avatar.X0 + 8, panel.ControlRect.Y + avatar.Y0 + 8);
                if (avatar.X1 > avatar.X0) { panel.Interact(avatarPoint, true); panel.Interact(avatarPoint, false); }
                else panel.SelectPlayer(1);
                if (panel.Player != 1 || panel.RowCount != 120) throw new InvalidOperationException("Panel fixture failed player filtering");
                if (panel.ScrollPosition <= 0) throw new InvalidOperationException("Player filtering unexpectedly reset the scroll offset");
                if (avatar.X1 > avatar.X0) { panel.Interact(avatarPoint, true); panel.Interact(avatarPoint, false); }
                else panel.SelectPlayer(1);
                if (panel.Player != null || panel.RowCount != 240) throw new InvalidOperationException("Panel fixture failed clearing the filter");
                var outside = new UiPoint(panel.ControlRect.X - 10, panel.ControlRect.Y + panel.ControlRect.H - 40);
                if (_combatTheme.HasScrollbar)
                {
                    var track = PanelGeometry.Scrollbar(new(panel.ControlRect.W, panel.ControlRect.H), _combatTheme.HasPlate,
                        PanelGeometry.BodyBand(panel.ControlRect.H, _combatTheme.HasPlate, panel.Layout.HeaderBottom), panel.Layout.Height, panel.ScrollPosition).Track;
                    var point = new UiPoint(panel.ControlRect.X + track.X + 10, panel.ControlRect.Y + track.Y + track.H / 2);
                    panel.Interact(point, true);
                    panel.Interact(outside, true);
                    if (!panel.Visible) throw new InvalidOperationException("Scrollbar drag dismissed after leaving the panel");
                    panel.Interact(outside, false);
                }
                panel.Interact(outside, true);
                if (panel.Requested || !panel.Visible) throw new InvalidOperationException("Outside dismissal did not stage visibility until refresh");
                panel.Refresh(3, fixture, fixture);
                if (panel.Visible) throw new InvalidOperationException("Outside press failed to dismiss");
                panel.Show();
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                var nodes = panel.Root.FindChildren("*", "", recursive: true, owned: false).Select(node => node.GetInstanceId()).ToHashSet();
                for (int iteration = 1; iteration <= 10; iteration++)
                {
                    var refreshed = fixture with
                    {
                        Turns = fixture.Turns + (uint)iteration,
                        Cards = fixture.Cards.Select(card => card with
                        {
                            DmgDirect = (iteration % 2 == 0 ? card.DmgDirect : 81 - card.DmgDirect) + iteration,
                            DamageDealt = (iteration % 2 == 0 ? card.DamageDealt : 81 - card.DamageDealt) + iteration,
                        }).ToArray()
                    };
                    long allocated = GC.GetAllocatedBytesForCurrentThread();
                    long started = Stopwatch.GetTimestamp();
                    panel.Refresh((ulong)iteration, refreshed, refreshed);
                    refreshTicks += Stopwatch.GetTimestamp() - started;
                    refreshAllocated += GC.GetAllocatedBytesForCurrentThread() - allocated;
                    refreshes++;
                    for (int frame = 0; frame < 2; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                    var current = panel.Root.FindChildren("*", "", recursive: true, owned: false).Select(node => node.GetInstanceId()).ToHashSet();
                    if (!nodes.SetEquals(current) || panel.RowCount != 240) throw new InvalidOperationException("Immediate chart refresh changed host controls");
                    if (panel.Rows[0].Name != refreshed.Cards.MaxBy(card => card.DamageDealt).Id) throw new InvalidOperationException("Chart rank did not follow updated totals");
                }
                if (panel.DrawCount == 0) throw new InvalidOperationException("Panel fixture never drew");
                draws += panel.DrawCount;
                var root = panel.Root;
                var backdrop = panel.Backdrop;
                panel.Free();
                for (int frame = 0; frame < 2; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                if (GodotObject.IsInstanceValid(root) || GodotObject.IsInstanceValid(backdrop)) throw new InvalidOperationException("Panel fixture retained freed controls");
                panel = null;
            }
            double refreshMsec = refreshTicks * 1000.0 / Stopwatch.Frequency;
            Log.Info(string.Create(CultureInfo.InvariantCulture, $"[SpireProfiler] managed panel refresh stress: refreshes={refreshes} sources=80 elapsed_ms={refreshMsec:F3} allocated_bytes={refreshAllocated}"));
            Log.Info($"[SpireProfiler] managed panel lifecycle: PASS (attach, chart, filter, scroll, hide, free, recreate; draws={draws})");
        }
        catch (Exception error) { Log.Error($"[SpireProfiler] managed panel lifecycle: FAIL {error}"); }
        finally { panel?.Free(); }
    }
}
