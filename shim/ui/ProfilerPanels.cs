using System;
using System.IO;
using System.Threading.Tasks;
using Godot;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Screens.RunHistoryScreen;

namespace SpireProfiler;

internal static class ProfilerPanels
{
    private static Control _panel;
    private static Control _runPanel;
    // The modal backdrop dimmers, one per panel. A panel cannot dim the
    // surround itself: its Control is plate-box-sized with ClipContents on
    // (its scroll needs it), so a fullscreen ColorRect added immediately
    // BEFORE the panel (same parent, drawn behind it) carries the dimmer —
    // the game's screen backdrop (StsColors.screenBackdrop, 80% black).
    // MouseFilter.Stop makes the dimmer
    // swallow clicks so they never reach the game while the panel is up
    // (modal). Each backdrop's Visible mirrors its panel's every frame in
    // OnProcessFrame; being plain ColorRects, they need no fallback
    // discipline of their own.
    private static ColorRect _panelBackdrop;
    private static ColorRect _runPanelBackdrop;
    private static Button _runButton;
    private static bool _f8Pressed;

    /// <summary>
    /// Creates one panel's modal backdrop (see the field declarations):
    /// a fullscreen 80%-black ColorRect that swallows clicks, hidden until
    /// the first OnProcessFrame mirrors the panel's visibility onto it.
    /// </summary>
    private static ColorRect NewBackdrop()
    {
        var backdrop = new ColorRect
        {
            Color = new Color(0.0f, 0.0f, 0.0f, 0.8f),
            MouseFilter = Control.MouseFilterEnum.Stop,
            Visible = false,
        };
        backdrop.SetAnchorsPreset(Control.LayoutPreset.FullRect);
        return backdrop;
    }

    internal static async Task AttachPanelAsync(string modDir)
    {
        try
        {
            var extensionResult = GDExtensionManager.LoadExtension(Path.Combine(modDir, "spire_profiler.gdextension"));
            Log.Info($"[SpireProfiler] GDExtension load result: {extensionResult}");
            if (extensionResult != GDExtensionManager.LoadStatus.Ok) return;

            SceneTree sceneTree = null;
            while ((sceneTree = Engine.GetMainLoop() as SceneTree) == null || sceneTree.Root == null)
            {
                await Task.Delay(100);
            }
            _panel = ClassDB.Instantiate("SpireProfilerPanel").As<Control>();
            if (_panel == null)
            {
                Log.Error("[SpireProfiler] panel instantiation failed");
                return;
            }
            // Scroll input is forwarded from C# (see ForwardScrollInput):
            // the native core never inspects the engine's input events.
            _panel.GuiInput += OnCombatPanelGuiInput;
            // The backdrop goes in immediately before the panel so the
            // panel draws over the dimmer (deferred adds run in order).
            _panelBackdrop = NewBackdrop();
            sceneTree.Root.CallDeferred(Node.MethodName.AddChild, _panelBackdrop);
            sceneTree.Root.CallDeferred(Node.MethodName.AddChild, _panel);
            sceneTree.ProcessFrame += OnProcessFrame;
            Log.Info("[SpireProfiler] profiler panel attached");
        }
        catch (Exception ex)
        {
            Log.Error($"[SpireProfiler] panel attach failed: {ex}");
        }
    }

    /// <summary>
    /// Per-frame driver for both panels: refreshes them, mirrors each
    /// panel's visibility onto its backdrop dimmer, and forwards
    /// edge-detected panel hotkeys to the native core, which owns all
    /// routing — F8 toggles the panel (the run panel on the run-history
    /// screen, the combat panel elsewhere). The per-player filter no
    /// longer has a key: the character avatars in the panels' headers
    /// are the toggles (pressed core-side, like the tabs). While the run
    /// panel is up, the toggle button is disabled and dimmed into the
    /// background so it cannot fight the click-away dismissal: a press on
    /// it closes the panel like any other background press.
    /// </summary>
    private static void OnProcessFrame()
    {
        try
        {
            if (_panel != null && GodotObject.IsInstanceValid(_panel)) _panel.Call("refresh");
            // The core owns visibility (it hides the panel outside a run);
            // the shim only reflects it onto the dimmer.
            if (_panel != null && GodotObject.IsInstanceValid(_panel)
                && _panelBackdrop != null && GodotObject.IsInstanceValid(_panelBackdrop))
                _panelBackdrop.Visible = _panel.Visible;
            // The run panel refreshes whenever it exists (lazily created on
            // the first run-history display); before its first attach it
            // stays hidden — the core's screen-open flag is false.
            if (_runPanel != null && GodotObject.IsInstanceValid(_runPanel)) _runPanel.Call("refresh");
            if (_runPanel != null && GodotObject.IsInstanceValid(_runPanel)
                && _runPanelBackdrop != null && GodotObject.IsInstanceValid(_runPanelBackdrop))
                _runPanelBackdrop.Visible = _runPanel.Visible;
            // A live toggle button would fight the click-away dismissal: the
            // press closes the panel, the release's Pressed re-opens it. The
            // button is inert while the panel is up (the core's outside-press
            // poll still sees the click and dismisses); the disabled stylebox
            // dims it into the background. F8 keeps working — it is not a
            // background press.
            if (_runButton != null && GodotObject.IsInstanceValid(_runButton)
                && _runPanel != null && GodotObject.IsInstanceValid(_runPanel))
                _runButton.Disabled = _runPanel.Visible;
            // C# only forwards edge-detected key presses; the core owns the
            // per-frame visibility and toggle routing.
            bool f8 = Input.IsKeyPressed(Key.F8);
            if (f8 && !_f8Pressed) ProfilerNative.OnPanelToggle();
            _f8Pressed = f8;
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] ProcessFrame: {ex}"); }
    }

    /// <summary>
    /// Lazily attaches the run-summary panel and its toggle button under
    /// the run-history screen node, on the first DisplayRun (never in a
    /// headless boot, which only reaches the main menu). Both hosts
    /// (main-menu stack, mid-run stack) cache their own NRunHistory node,
    /// so a second open reparents via AddChild; a freed screen instance
    /// frees its children, and the validity checks re-instantiate.
    /// </summary>
    internal static void AttachRunPanelTo(NRunHistory screen)
    {
        try
        {
            if (_runPanel == null || !GodotObject.IsInstanceValid(_runPanel))
            {
                _runPanel = ClassDB.Instantiate("SpireProfilerRunPanel").As<Control>();
                if (_runPanel == null)
                {
                    Log.Error("[SpireProfiler] run panel instantiation failed");
                    return;
                }
                // Same scroll forward as the combat panel (the connection
                // dies with the panel instance, so a re-instantiation
                // re-connects exactly once). The backdrop shares the
                // panel's lifecycle: a freed screen frees both, and this
                // validity check re-creates both.
                _runPanel.GuiInput += OnRunPanelGuiInput;
                _runPanelBackdrop = NewBackdrop();
            }
            if (_runButton == null || !GodotObject.IsInstanceValid(_runButton))
            {
                _runButton = new Button { Text = "Profiler [F8]" };
                // Styling is best-effort: a failed game-resource load leaves
                // the plain engine look for that piece, never a dead button.
                StyleRunButton(_runButton);
                _runButton.Pressed += OnRunHistoryToggleButton;
            }
            // Placement is derived, not ad-hoc: the button joins the
            // screen's own bottom-right button cluster, directly above the
            // run-history screen's ShareButton (run_history.tscn: a direct
            // child of the full-rect screen root, anchored bottom-right at
            // 172×64 with 48px margins), right edges flush, a 16px gap
            // between. The bottom-right cluster is the free corner BECAUSE
            // the top-right is the date block's (the RightAlignedStuff
            // labels). The size follows the game button's, not our own
            // literal; the bottom-right anchors match the ShareButton's, so
            // both track a viewport resize together. The panel centers
            // itself (the core owns all placement), so the shim never
            // positions it.
            PlaceRunButton(screen);
            // Draw order among the three siblings: backdrop, button, panel.
            // The button is inert background while the panel is up, so it
            // must sit UNDER the panel — added after it, the button would
            // cover the panel's hover tooltip, which can extend into the
            // button's bottom-right corner. AddChild reparents on a second
            // open, so each guarded call runs at most once per screen.
            if (_runPanel.GetParent() != screen)
                screen.CallDeferred(Node.MethodName.AddChild, _runPanelBackdrop);
            if (_runButton.GetParent() != screen)
                screen.CallDeferred(Node.MethodName.AddChild, _runButton);
            if (_runPanel.GetParent() != screen)
                screen.CallDeferred(Node.MethodName.AddChild, _runPanel);
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] run panel attach: {ex}"); }
    }

    /// <summary>
    /// The button forwards the same toggle the F8 key does; the core
    /// routes it to the run panel while the run-history screen is open.
    /// It can only ever OPEN the panel: while the panel is up the shim
    /// disables the button (see OnProcessFrame), so the Pressed handler
    /// never fires against an open panel.
    /// </summary>
    private static void OnRunHistoryToggleButton()
    {
        try { ProfilerNative.OnPanelToggle(); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] run history button: {ex}"); }
    }

    /// <summary>
    /// Positions the run-history toggle button relative to the screen's
    /// ShareButton LIVE rect (see the attach site for why the bottom-right
    /// cluster): same size as the game button, directly above it with a
    /// 16px gap, right edges flush. Node missing or not yet laid out →
    /// the documented fallback: the ShareButton's own scene corner
    /// (run_history.tscn offsets -220/-112 left/top, 48px right/bottom
    /// margins — the button lands where the ShareButton would be, shifted
    /// up by the gap and its height), with a WARNING, never an ERROR.
    /// </summary>
    private static void PlaceRunButton(NRunHistory screen)
    {
        const float gap = 16.0f;
        var host = screen.GetRect().Size;
        var share = screen.GetNodeOrNull<Control>("ShareButton");
        var shareRect = share?.GetRect() ?? new Rect2();
        // The fallback size is the ShareButton's scene size (172×64), not
        // an invention; the label measures 130px at the shipped TTF's
        // advances, leaving the plate's side margins their air.
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
        _runButton.SetAnchorsPreset(Control.LayoutPreset.BottomRight);
        _runButton.OffsetLeft = origin.X - host.X;
        _runButton.OffsetTop = origin.Y - host.Y;
        _runButton.OffsetRight = origin.X + size.X - host.X;
        _runButton.OffsetBottom = origin.Y + size.Y - host.Y;
    }

    /// <summary>
    /// Restyles the run-history toggle button with the game's own chrome
    /// and text behavior. The plate is the game's settings-tab plate
    /// (`settings_tab_selected.tres`, the same art the core's tab strip
    /// draws — the shim loads its own copy via GD.Load, the safe managed
    /// path) as a StyleBoxTexture: a dark rounded plate that still reads
    /// as a button at 64px tall, where the confirm/proceed button art
    /// does not (confirm_button.tres bakes a 70px bottom margin of shadow
    /// into a 132px region — its nine-patchable center is 37px, over half
    /// the button's height; proceed_button.tres is a 2.0-aspect banner
    /// against the button's 2.7). The AtlasTexture's draw frame is
    /// region + margin = 515×181 (`theme.rs` TAB_ART_SIZE pins the same);
    /// the 24px patch margins cover the rounded corners (~20px in the
    /// art) plus the frame's transparent pad, and the flat center
    /// stretches invisibly. The 0.9 modulate is the tab's rest state
    /// (NSettingsTab drives the art through the game's HSV recolor shader
    /// at v=0.9, and scaling HSV value by 0.9 is exactly an 0.9 RGB
    /// multiply). The text behavior is NMainMenuTextButton's exact
    /// treatment: cream going gold on hover and half-transparent white
    /// while pressed (StsColors.cream/gold/halfTransparentWhite
    /// verbatim) — one plate for every state, the label carries the
    /// state, and the focus ring stays off (the engine's default ring is
    /// not the game's look). The disabled state (the panel is up) dims
    /// the plate to the backdrop's 20% brightness and the label to
    /// half-transparent white, so the button reads as part of the dimmed
    /// background instead of a live control. The game ships no project theme, so a bare
    /// Button renders in Godot's default look — that was the defect. The face is Kreon Bold
    /// glyph-space-one (the game's menu-button face, submenu_button.tscn's
    /// Label) at 22px. The game's hover/press scale tweens need an
    /// animation clock the shim does not keep — deliberately omitted.
    ///
    /// Every piece is independent best-effort: a failed game-resource
    /// load keeps the engine default for that piece with one WARNING —
    /// never an ERROR, and never a dead button (the Pressed wiring
    /// happens regardless).
    /// </summary>
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
                    // The 80% backdrop leaves 20% of the scene visible
                    // (1 - 0.8); the plate's rest state (0.9) scaled by
                    // that 0.2 is 0.18 — exactly how the button would
                    // render under the dimmer, so it reads as part of the
                    // background.
                    ModulateColor = new Color(0.18f, 0.18f, 0.18f, 1.0f),
                };
                button.AddThemeStyleboxOverride("disabled", disabledBox);
            }
            button.AddThemeColorOverride("font_color", StsColors.cream);
            button.AddThemeColorOverride("font_hover_color", StsColors.gold);
            button.AddThemeColorOverride("font_pressed_color", StsColors.halfTransparentWhite);
            button.AddThemeColorOverride("font_disabled_color", StsColors.halfTransparentWhite);
            button.AddThemeColorOverride("font_focus_color", StsColors.cream);
            // The focus stylebox stays empty — the engine's default focus
            // ring is not the game's look.
            button.AddThemeStyleboxOverride("focus", new StyleBoxEmpty());
        }
        catch (Exception ex) { Log.Warn($"[SpireProfiler] run button styling: {ex.Message}"); }
    }

    // ── scroll input forward ─────────────────────────────────────────────
    // The panels' scroll input crosses the ABI as raw event fields because
    // the NATIVE side must never inspect an engine InputEvent: every
    // GDExtension-originated call about one (method dispatch AND the pure C
    // object_get_class_name alike) hangs the game's proprietary engine fork.
    // The managed side is the game's own input path (its NScrollableContainer
    // reads the same event types in C#), so the GuiInput signal — same
    // targeting as the _gui_input virtual, fired for the control under the
    // cursor — is the safe place to read them. The shim stays plumbing:
    // kind discrimination and field extraction only; all translation and
    // routing lives in the core (ui::scroll).

    private static void OnCombatPanelGuiInput(InputEvent inputEvent) =>
        ForwardScrollInput(ProfilerNative.PanelCombat, inputEvent);

    private static void OnRunPanelGuiInput(InputEvent inputEvent) =>
        ForwardScrollInput(ProfilerNative.PanelRun, inputEvent);

    /// <summary>
    /// Forwards one gui_input event's raw scroll fields to the native core.
    /// Non-scroll kinds (mouse motion, keys) never cross the ABI — they
    /// dominate the event stream and carry no scroll content.
    /// </summary>
    private static void ForwardScrollInput(int panel, InputEvent inputEvent)
    {
        try
        {
            switch (inputEvent)
            {
                case InputEventMouseButton button:
                    ProfilerNative.OnScrollInput(panel, (int)button.ButtonIndex, button.Pressed, 0.0);
                    break;
                case InputEventPanGesture pan:
                    ProfilerNative.OnScrollInput(panel, 0, false, (double)pan.Delta.Y);
                    break;
            }
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] scroll input: {ex.Message}"); }
    }
}
