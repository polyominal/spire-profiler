using System;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Threading.Tasks;
using Godot;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Screens.RunHistoryScreen;

namespace SpireProfiler;

internal static class ProfilerPanels
{
    private static SceneTree _tree;
    private static PanelTheme _theme;
    private static ProfilerPanel _combat;
    private static ProfilerPanel _history;
    private static Button _historyButton;
    private static NRunHistory _historyHost;
    private static bool _f8Pressed;
    private static ulong _nextRefreshAtMsec;

    internal static async Task AttachPanelAsync(string modDir)
    {
        try
        {
            if (_tree != null && GodotObject.IsInstanceValid(_tree)) return;
            SceneTree tree;
            while ((tree = Engine.GetMainLoop() as SceneTree) == null || tree.Root == null) await Task.Delay(100);
            _tree = tree;
            _theme = new PanelTheme();
            _combat = new ProfilerPanel(_theme, false);
            tree.Root.CallDeferred(Node.MethodName.AddChild, _combat.Root);
            tree.ProcessFrame += OnProcessFrame;
            tree.Root.TreeExiting += Detach;
            Log.Info("[SpireProfiler] managed profiler panel attached");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] panel attach failed: {ex}"); }
    }

    private static void Detach()
    {
        if (_tree != null && GodotObject.IsInstanceValid(_tree))
        {
            _tree.ProcessFrame -= OnProcessFrame;
            if (GodotObject.IsInstanceValid(_tree.Root)) _tree.Root.TreeExiting -= Detach;
        }
        _combat = null;
        _history = null;
        _historyButton = null;
        _historyHost = null;
        _theme = null;
        _tree = null;
        _f8Pressed = false;
    }

    private static void OnProcessFrame()
    {
        try
        {
            bool f8 = Input.IsKeyPressed(Key.F8);
            if (f8 && !_f8Pressed) Toggle();
            _f8Pressed = f8;
            if (!ProfilerSession.InRun || ProfilerSession.HistoryOpen) _combat?.Hide();
            if (!ProfilerSession.HistoryOpen) _history?.Hide();
            if ((_combat?.Visible == true || _history?.Visible == true) && Time.GetTicksMsec() >= _nextRefreshAtMsec)
            {
                _nextRefreshAtMsec = Time.GetTicksMsec() + 100;
                ProfilerSession.Refresh();
                _combat?.Refresh(ProfilerSession.Revision, ProfilerSession.CurrentCombat, ProfilerSession.CurrentRun);
                _history?.Refresh(ProfilerSession.Revision, null, ProfilerSession.SelectedHistory);
            }
            if (_historyButton != null && GodotObject.IsInstanceValid(_historyButton))
                _historyButton.Disabled = _history?.Visible == true;
        }
        catch (Exception ex)
        {
            _combat?.Hide();
            _history?.Hide();
            Log.Error($"[SpireProfiler] panel refresh: {ex}");
        }
    }

    private static void Toggle()
    {
        ProfilerPanel panel = ProfilerSession.HistoryOpen ? _history : ProfilerSession.InRun ? _combat : null;
        if (panel == null || !panel.IsValid) return;
        if (panel.Visible) panel.Hide();
        else
        {
            panel.Show();
            _nextRefreshAtMsec = 0;
        }
    }

    internal static void AttachRunPanelTo(NRunHistory screen)
    {
        try
        {
            _theme ??= new PanelTheme();
            if (_history?.IsValid == true && _historyHost == screen) return;
            _history?.Free();
            if (_historyButton != null && GodotObject.IsInstanceValid(_historyButton)) _historyButton.QueueFree();
            _historyHost = screen;
            _history = new ProfilerPanel(_theme, true);
            _historyButton = _theme.Button("Profiler [F8]", Toggle);
            var share = screen.GetNodeOrNull<Control>("ShareButton");
            var size = share?.Size ?? new Vector2(172, 64);
            if (size.X <= 0 || size.Y <= 0) size = new Vector2(172, 64);
            _historyButton.SetAnchorsPreset(Control.LayoutPreset.BottomRight);
            _historyButton.OffsetRight = -48;
            _historyButton.OffsetBottom = -48 - size.Y - 16;
            _historyButton.OffsetLeft = _historyButton.OffsetRight - size.X;
            _historyButton.OffsetTop = _historyButton.OffsetBottom - size.Y;
            screen.CallDeferred(Node.MethodName.AddChild, _historyButton);
            screen.CallDeferred(Node.MethodName.AddChild, _history.Root);
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] run panel attach: {ex}"); }
    }

    internal static async Task SelfTestAsync()
    {
        ProfilerPanel panel = null;
        try
        {
            var tree = Engine.GetMainLoop() as SceneTree ?? throw new InvalidOperationException("Panel fixture needs the game scene tree");
            var fixture = new SummaryView
            {
                Title = "Managed panel lifecycle fixture",
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
            int draws = 0;
            int refreshes = 0;
            long refreshTicks = 0;
            long refreshAllocated = 0;
            for (int cycle = 0; cycle < 2; cycle++)
            {
                panel = new ProfilerPanel(new PanelTheme(), cycle == 1);
                tree.Root.CallDeferred(Node.MethodName.AddChild, panel.Root);
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                panel.Show();
                panel.Present(fixture);
                for (int frame = 0; frame < 3; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                if (!panel.Root.IsInsideTree() || panel.RowCount != 240)
                    throw new InvalidOperationException("Panel fixture did not attach and build both chart sections");
                panel.ScrollToEnd();
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                if (panel.ScrollPosition <= 0) throw new InvalidOperationException("Panel fixture did not scroll overflowing content");
                panel.SelectPlayer(1);
                if (panel.Player != 1 || panel.RowCount != 120)
                    throw new InvalidOperationException("Panel fixture did not filter by source owner");
                panel.SelectPlayer(1);
                if (panel.Player != null || panel.RowCount != 240)
                    throw new InvalidOperationException("Panel fixture did not clear the selected player");
                panel.Hide();
                if (panel.Visible) throw new InvalidOperationException("Panel fixture did not hide");
                panel.Show();
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                if (panel.DrawCount == 0) throw new InvalidOperationException("Panel fixture never received a managed draw signal");
                int nodes = panel.Root.FindChildren("*", "", recursive: true, owned: false).Count;
                for (int iteration = 1; iteration <= 10; iteration++)
                {
                    var previous = panel.Root.FindChildren("*", "", recursive: true, owned: false)
                        .Select(node => node.GetInstanceId()).ToHashSet();
                    var refreshed = fixture with
                    {
                        Turns = fixture.Turns + (uint)iteration,
                        Cards = fixture.Cards.Select(card => card with
                        {
                            DmgDirect = (iteration % 2 == 0 ? card.DmgDirect : 81 - card.DmgDirect) + iteration,
                            DamageDealt = (iteration % 2 == 0 ? card.DamageDealt : 81 - card.DamageDealt) + iteration,
                        }).ToArray(),
                    };
                    long allocated = GC.GetAllocatedBytesForCurrentThread();
                    long started = Stopwatch.GetTimestamp();
                    panel.Refresh((ulong)iteration, refreshed, refreshed);
                    refreshTicks += Stopwatch.GetTimestamp() - started;
                    refreshAllocated += GC.GetAllocatedBytesForCurrentThread() - allocated;
                    refreshes++;
                    for (int frame = 0; frame < 2; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                    var current = panel.Root.FindChildren("*", "", recursive: true, owned: false);
                    var currentIds = current.Select(node => node.GetInstanceId()).ToHashSet();
                    if (current.Count != nodes || panel.RowCount != 240 || !previous.SetEquals(currentIds))
                        throw new InvalidOperationException("Panel refresh rebuilt controls despite unchanged source identities");
                    string damage = $"{refreshed.Cards.Sum(card => card.DamageDealt)} damage";
                    if (!current.OfType<Label>().Any(label => label.Text.Contains(damage, StringComparison.Ordinal)))
                        throw new InvalidOperationException("Panel refresh did not display changed totals");
                    string firstId = current.OfType<HBoxContainer>().First(line => line.TooltipText.Length != 0).TooltipText.Split('\n')[1].Trim();
                    if (firstId != refreshed.Cards.MaxBy(card => card.DamageDealt).Id)
                        throw new InvalidOperationException("Retained chart rows did not follow their changing damage rank");
                }
                var replacedNodes = panel.Root.FindChildren("*", "", recursive: true, owned: false)
                    .Select(node => (Node: node, Id: node.GetInstanceId())).ToArray();
                var replaced = fixture with { Cards = fixture.Cards.Select((card, index) => index == 0 ? card with { Id = "REPLACED_SOURCE" } : card).ToArray() };
                panel.Present(replaced);
                for (int frame = 0; frame < 2; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                var remaining = panel.Root.FindChildren("*", "", recursive: true, owned: false);
                var remainingIds = remaining.Select(node => node.GetInstanceId()).ToHashSet();
                if (remaining.Count != nodes || panel.RowCount != 240 || replacedNodes.All(node => remainingIds.Contains(node.Id))
                    || replacedNodes.Any(node => !remainingIds.Contains(node.Id) && GodotObject.IsInstanceValid(node.Node))
                    || !remaining.OfType<HBoxContainer>().Any(line => line.TooltipText.Contains(System.Environment.NewLine + "REPLACED_SOURCE" + System.Environment.NewLine, StringComparison.Ordinal)))
                    throw new InvalidOperationException("A changed source set must replace and free its old chart rows");
                draws += panel.DrawCount;
                bool exited = false;
                panel.Root.TreeExited += () => exited = true;
                panel.Free();
                for (int frame = 0; frame < 2; frame++) await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                if (!exited || panel.IsValid) throw new InvalidOperationException("Panel fixture retained its freed root");
                panel = null;
            }
            double refreshMsec = refreshTicks * 1000.0 / Stopwatch.Frequency;
            Log.Info(string.Create(CultureInfo.InvariantCulture,
                $"[SpireProfiler] managed panel refresh stress: refreshes={refreshes} sources=80 elapsed_ms={refreshMsec:F3} allocated_bytes={refreshAllocated}"));
            Log.Info($"[SpireProfiler] managed panel lifecycle: PASS (attach, chart, filter, scroll, hide, free, recreate; draws={draws})");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] managed panel lifecycle: FAIL {ex}"); }
        finally { panel?.Free(); }
    }
}
