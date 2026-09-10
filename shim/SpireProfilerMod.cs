using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Godot;
using HarmonyLib;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Combat.History;
using MegaCrit.Sts2.Core.Context;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Multiplayer;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Hooks;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Modding;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Orbs;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Models.Monsters;
using MegaCrit.Sts2.Core.Models.Relics;
using MegaCrit.Sts2.Core.Nodes.CommonUi;
using MegaCrit.Sts2.Core.Nodes.Screens.MainMenu;
using MegaCrit.Sts2.Core.Nodes.Screens.RunHistoryScreen;
using MegaCrit.Sts2.Core.Rooms;
using MegaCrit.Sts2.Core.Runs;
using MegaCrit.Sts2.Core.Saves;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

/// <summary>
/// Required by the StS2 mod loader (types with [ModInitializer] are found
/// and their static initializer invoked). The profiler's real work happens
/// in the native core; this shim loads it and forwards game events
/// through the C ABI.
/// </summary>
[ModInitializer("Initialize")]
public static class SpireProfilerMod
{
    private static bool _initialized;

    /// <summary>
    /// The run's players in lobby order (host first) — the game's own slot
    /// numbering (RunState.GetPlayerSlotIndex is Players.IndexOf, identical
    /// on every peer). Captured at run start and immutable for the session;
    /// single player is a one-element list, so every slot resolves to 0.
    /// </summary>
    private static readonly List<Player> _runPlayers = new();
    private static RunState _runState;
    internal static object CurrentCombat => (_runState?.CurrentRoom as CombatRoom)?.CombatState;

    internal static int PlayerSlot(Player player)
    {
        if (player == null || _runPlayers.Count == 0) return 0;
        int index = _runPlayers.IndexOf(player);
        return index < 0 ? 0 : Math.Min(index, 3);
    }

    internal static uint PaperKraneSlots()
    {
        uint slots = 0;
        for (int index = 0; index < Math.Min(_runPlayers.Count, 4); index++)
            if (_runPlayers[index].GetRelic<PaperKrane>() != null) slots |= 1u << index;
        return slots;
    }

    internal static int CreditorSlot(Player player)
    {
        if (player == null) return ProfilerNative.TeamSlot;
        int index = _runPlayers.FindIndex(candidate => ReferenceEquals(candidate, player));
        return index >= 0 && index < 4 ? index : ProfilerNative.TeamSlot;
    }

    /// <summary>
    /// Refills the run-player registry from a run state. Every run start and
    /// resume re-fires NotifyRunStarted; run state is immutable during a
    /// run, so clear + refill is exact.
    /// </summary>
    internal static void CaptureRunPlayers(RunState state)
    {
        if (!CaptureRuntime.OnThread) return;
        _runState = state;
        _runPlayers.Clear();
        if (state?.Players != null) _runPlayers.AddRange(state.Players);
    }

    /// <summary>
    /// The only locality check left: the enemy→player hit capture in the
    /// ModifyDamage prefix, which feeds the LOCAL player's mitigation math
    /// in the core (that capture is intentionally local — see the patch).
    /// All other event forwarding is locality-free: every player's events
    /// are counted and slot-tagged instead.
    /// </summary>
    internal static bool IsLocalPlayer(Player player)
    {
        if (player == null) return false;
        return LocalContext.NetId.HasValue ? LocalContext.IsMe(player) : true;
    }

    /// Comma-joined character ids of the players in a run state (one id in
    /// single player, several in multiplayer).
    internal static string CharacterIds(RunState state)
    {
        if (state?.Players == null) return "";
        return string.Join(",", state.Players.Select(p => p.Character?.Id?.Entry ?? "?"));
    }

    /// Comma-joined player NetIds (ulong) of the players in a run state,
    /// positional with CharacterIds. The NetIds are the roster's
    /// cross-session identity — slots are session-immutable, NetIds survive
    /// — so the persistence side can match players across runs.
    internal static string NetIds(RunState state)
    {
        if (state?.Players == null) return "";
        return string.Join(",", state.Players.Select(p => p.NetId));
    }

    /// The active game profile id (SaveManager.CurrentProfileId), or -1
    /// when the save manager is not initialized yet (safe at run start).
    internal static int CurrentProfileId()
    {
        try { return SaveManager.Instance.CurrentProfileId; }
        catch (Exception) { return -1; }
    }

    public static void Initialize()
    {
        if (_initialized) return;
        _initialized = true;
        try
        {
            var assemblyDir = Path.GetDirectoryName(typeof(SpireProfilerMod).Assembly.Location);
            var modDir = !string.IsNullOrEmpty(assemblyDir)
                ? assemblyDir
                : Path.Combine(AppContext.BaseDirectory, "mods", "spire-profiler");
            // The mod bundle lives at <game dir>/mods/spire-profiler, so one
            // parent hop lands inside mods/. Climb two hops to the game dir:
            // the data files must sit at <game dir>/mod_data/spire-profiler,
            // the sibling of mods/ — the mod scanner sweeps every *.json
            // under mods/ as a manifest (see docs/game.md). SPIRE_PROFILER_DATA_DIR overrides the location:
            // headless-test points it at a scratch dir so its self-test
            // records never mix into real play data.
            var modsDir = Path.GetDirectoryName(modDir) ?? ".";
            // Qualified: the bare name is ambiguous with Godot.Environment.
            var dataDirOverride = System.Environment.GetEnvironmentVariable("SPIRE_PROFILER_DATA_DIR");
            var dataDir = !string.IsNullOrEmpty(dataDirOverride)
                ? dataDirOverride
                : Path.Combine(Path.GetDirectoryName(modsDir) ?? ".", "mod_data", "spire-profiler");

            ProfilerNative.Load(Path.Combine(modDir, NativeLibrarySelector.FileName()));
            ProfilerNative.Init(dataDir);
            CaptureRuntime.Initialize(new NativeAttributionBackend());
            Log.Info($"[SpireProfiler] native core loaded; mod dir: {modDir}; data dir: {dataDir}");

            var harmony = new Harmony("dev.spireprofiler");
            foreach (var type in typeof(SpireProfilerMod).Assembly.GetTypes())
            {
                if (!type.GetCustomAttributes(typeof(HarmonyPatch), false).Any()) continue;
                try
                {
                    harmony.CreateClassProcessor(type).Patch();
                }
                catch (Exception ex)
                {
                    Log.Error($"[SpireProfiler] Harmony patch failed for {type.Name}: {ex.Message}");
                }
            }
            FlowCapture.Install(harmony, message => Log.Info($"[SpireProfiler] {message}"));
            var ownedMethods = Harmony.GetAllPatchedMethods().Where(method => Harmony.GetPatchInfo(method)?.Owners.Contains(harmony.Id) == true).ToArray();
            Log.Info($"[SpireProfiler] OWN PATCHES owner={harmony.Id} methods={ownedMethods.Length}");
            // Version-drift tripwire: these five gate the run lifecycle;
            // if a game update renames them, runs would silently go unrecorded.
            foreach (var name in new[] { "SetUpNewSingleplayer", "SetUpNewMultiplayer", "SetUpSavedSingleplayer", "SetUpSavedMultiplayer", "CleanUp" })
            {
                if (!ownedMethods.Any(m => m.DeclaringType == typeof(RunManager) && m.Name == name))
                    Log.Error($"[SpireProfiler] expected patch missing: RunManager.{name}");
            }

            // The self-test only runs on request (see the headless-test build
            // step) so normal play never pollutes the data files. Use the
            // game's own arg parser (Environment.GetCommandLineArgs is
            // unreliable under Godot's Mono bootstrap).
            if (CommandLineHelper.HasArg("spire-profiler-self-test"))
            {
                ProfilerNative.SelfTest();
                Log.Info("[SpireProfiler] self-test sequence sent to native core");
            }

            // In-game panel: load the GDExtension and attach the panel to
            // the scene tree once it exists (async, so a slow tree doesn't
            // block mod initialization).
            _ = AttachPanelAsync(modDir);
        }
        catch (Exception ex)
        {
            Log.Error($"[SpireProfiler] initialization failed: {ex}");
        }
    }

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

    private static async Task AttachPanelAsync(string modDir)
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

/// <summary>
/// C ABI bindings for the native core. Function pointers are resolved
/// explicitly so no DllImport probing rules apply; the native library is
/// loaded by absolute path from the mod directory.
/// </summary>
internal static class ProfilerNative
{
    internal const int TeamSlot = 4;
    internal const int PanelCombat = 0;
    internal const int PanelRun = 1;

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeInit([MarshalAs(UnmanagedType.LPUTF8Str)] string dataDir);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeSelfTest();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeSetRunMeta(int profileId);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunStarted([MarshalAs(UnmanagedType.LPUTF8Str)] string characterIds, int ascension, [MarshalAs(UnmanagedType.LPUTF8Str)] string gameMode, [MarshalAs(UnmanagedType.LPUTF8Str)] string seed, int continued, [MarshalAs(UnmanagedType.LPUTF8Str)] string netIds, long startTime);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunEnded(int outcome);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunSuspended();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunHistorySelect([MarshalAs(UnmanagedType.LPUTF8Str)] string seed, long startTime, int profile);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeRunHistoryClear();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativePanelToggle();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeScrollInput(int panel, int buttonIndex, int pressed, double panY);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeCombatStarted([MarshalAs(UnmanagedType.LPUTF8Str)] string encounterId, [MarshalAs(UnmanagedType.LPUTF8Str)] string encounterType);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCombatEnded(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeTurnStarted(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockPoolClear(ulong combatSeq, int playerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePlayerDied(ulong combatSeq, int playerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePotionUsed(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceCapture(ulong combatSeq, int captureKind, ulong instance, [MarshalAs(UnmanagedType.LPUTF8Str)] string sourceId, int sourceKind, int sourceSlot, int generationState);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceCount(ulong transfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceDestination(ulong transfer, int index);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceWeight(ulong transfer, int index);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeSourceTransferBegin(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceTransferAdd(ulong transfer, ulong destination, ulong weight);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceTransferSeal(ulong transfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeSourceTransferRelease(ulong transfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerAttached(ulong combatSeq, ulong powerInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int amount, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerAmountChanged(ulong combatSeq, ulong powerInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int oldAmount, int newAmount, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerRemoved(ulong combatSeq, ulong powerInstance);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativePowerProvenanceInvalidate(ulong combatSeq, ulong powerInstance);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardGenerated(ulong combatSeq, ulong cardInstance, ulong sourceTransfer, int producerRole);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeCardPlayStarted(ulong combatSeq, ulong executionId, ulong cardInstance, [MarshalAs(UnmanagedType.LPUTF8Str)] string cardId, int playerSlot, int playIndex, int playCount, int generationState, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardPlayFinished(ulong play);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeCardExecutionEnded(ulong combatSeq, ulong executionId);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOrbChanneled(ulong combatSeq, ulong orbInstance, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOrbContextBegin(ulong combatSeq, ulong orbInstance, ulong play, int ownerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeDamageCalculationBegin(ulong combatSeq, ulong sourceTransfer, int producerRole, int segment, ulong originalTarget);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageModifierContribution(ulong calculation, ulong sourceTransfer, int amount);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationEnemyHit(ulong calculation, ulong dealerCreature, int baseDamage, int dealerStrength);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationWeakSource(ulong calculation, ulong sourceTransfer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageResultAppend(ulong calculation, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationCommit(ulong calculation);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageCalculationAbort(ulong calculation);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDamageUnattributed(ulong combatSeq, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBuffMitigation(ulong combatSeq, ulong sourceTransfer, int prevented);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockModifierContribution(ulong combatSeq, ulong sourceTransfer, int amount, int receiverSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeBlockGained(ulong combatSeq, int amount, ulong sourceTransfer, int receiverSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeForge(ulong combatSeq, ulong sourceTransfer, int amount);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOstySummoned(ulong combatSeq, ulong sourceTransfer, int hpAmount, int ownerSlot);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeOstyKilled(ulong combatSeq, int ownerSlot, ulong play);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate ulong NativeDoomBatchBegin(ulong combatSeq);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomTargetCapture(ulong batch, ulong creatureInstance, ulong doomPowerInstance, int currentHp);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomKillsCompleted(ulong batch);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int NativeDoomBatchAbort(ulong batch);

    private static NativeInit _init;
    private static NativeSelfTest _self_test;
    private static NativeSetRunMeta _set_run_meta;
    private static NativeRunStarted _run_started;
    private static NativeRunEnded _run_ended;
    private static NativeRunSuspended _run_suspended;
    private static NativeRunHistorySelect _run_history_select;
    private static NativeRunHistoryClear _run_history_clear;
    private static NativePanelToggle _panel_toggle;
    private static NativeScrollInput _scroll_input;
    private static NativeCombatStarted _combat_started;
    private static NativeCombatEnded _combat_ended;
    private static NativeTurnStarted _turn_started;
    private static NativeBlockPoolClear _block_pool_clear;
    private static NativePlayerDied _player_died;
    private static NativePotionUsed _potion_used;
    private static NativeSourceCapture _source_capture;
    private static NativeSourceCount _source_count;
    private static NativeSourceDestination _source_destination;
    private static NativeSourceWeight _source_weight;
    private static NativeSourceTransferBegin _source_transfer_begin;
    private static NativeSourceTransferAdd _source_transfer_add;
    private static NativeSourceTransferSeal _source_transfer_seal;
    private static NativeSourceTransferRelease _source_transfer_release;
    private static NativePowerAttached _power_attached;
    private static NativePowerAmountChanged _power_amount_changed;
    private static NativePowerRemoved _power_removed;
    private static NativePowerProvenanceInvalidate _power_provenance_invalidate;
    private static NativeCardGenerated _card_generated;
    private static NativeCardPlayStarted _card_play_started;
    private static NativeCardPlayFinished _card_play_finished;
    private static NativeCardExecutionEnded _card_execution_ended;
    private static NativeOrbChanneled _orb_channeled;
    private static NativeOrbContextBegin _orb_context_begin;
    private static NativeDamageCalculationBegin _damage_calculation_begin;
    private static NativeDamageModifierContribution _damage_modifier_contribution;
    private static NativeDamageCalculationEnemyHit _damage_calculation_enemy_hit;
    private static NativeDamageCalculationWeakSource _damage_calculation_weak_source;
    private static NativeDamageResultAppend _damage_result_append;
    private static NativeDamageCalculationCommit _damage_calculation_commit;
    private static NativeDamageCalculationAbort _damage_calculation_abort;
    private static NativeDamageUnattributed _damage_unattributed;
    private static NativeBuffMitigation _buff_mitigation;
    private static NativeBlockModifierContribution _block_modifier_contribution;
    private static NativeBlockGained _block_gained;
    private static NativeForge _forge;
    private static NativeOstySummoned _osty_summoned;
    private static NativeOstyKilled _osty_killed;
    private static NativeDoomBatchBegin _doom_batch_begin;
    private static NativeDoomTargetCapture _doom_target_capture;
    private static NativeDoomKillsCompleted _doom_kills_completed;
    private static NativeDoomBatchAbort _doom_batch_abort;

    private static T GetExport<T>(IntPtr lib, string name) where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(NativeLibrary.GetExport(lib, name));

    internal static void Load(string path)
    {
        var lib = NativeLibrary.Load(path);
        _init = GetExport<NativeInit>(lib, "spire_profiler_init");
        _self_test = GetExport<NativeSelfTest>(lib, "spire_profiler_self_test");
        _set_run_meta = GetExport<NativeSetRunMeta>(lib, "spire_profiler_set_run_meta");
        _run_started = GetExport<NativeRunStarted>(lib, "spire_profiler_run_started");
        _run_ended = GetExport<NativeRunEnded>(lib, "spire_profiler_run_ended");
        _run_suspended = GetExport<NativeRunSuspended>(lib, "spire_profiler_run_suspended");
        _run_history_select = GetExport<NativeRunHistorySelect>(lib, "spire_profiler_run_history_select");
        _run_history_clear = GetExport<NativeRunHistoryClear>(lib, "spire_profiler_run_history_clear");
        _panel_toggle = GetExport<NativePanelToggle>(lib, "spire_profiler_panel_toggle");
        _scroll_input = GetExport<NativeScrollInput>(lib, "spire_profiler_scroll_input");
        _combat_started = GetExport<NativeCombatStarted>(lib, "spire_profiler_combat_started");
        _combat_ended = GetExport<NativeCombatEnded>(lib, "spire_profiler_combat_ended");
        _turn_started = GetExport<NativeTurnStarted>(lib, "spire_profiler_turn_started");
        _block_pool_clear = GetExport<NativeBlockPoolClear>(lib, "spire_profiler_block_pool_clear");
        _player_died = GetExport<NativePlayerDied>(lib, "spire_profiler_player_died");
        _potion_used = GetExport<NativePotionUsed>(lib, "spire_profiler_potion_used");
        _source_capture = GetExport<NativeSourceCapture>(lib, "spire_profiler_source_capture");
        _source_count = GetExport<NativeSourceCount>(lib, "spire_profiler_source_count");
        _source_destination = GetExport<NativeSourceDestination>(lib, "spire_profiler_source_destination");
        _source_weight = GetExport<NativeSourceWeight>(lib, "spire_profiler_source_weight");
        _source_transfer_begin = GetExport<NativeSourceTransferBegin>(lib, "spire_profiler_source_transfer_begin");
        _source_transfer_add = GetExport<NativeSourceTransferAdd>(lib, "spire_profiler_source_transfer_add");
        _source_transfer_seal = GetExport<NativeSourceTransferSeal>(lib, "spire_profiler_source_transfer_seal");
        _source_transfer_release = GetExport<NativeSourceTransferRelease>(lib, "spire_profiler_source_transfer_release");
        _power_attached = GetExport<NativePowerAttached>(lib, "spire_profiler_power_attached");
        _power_amount_changed = GetExport<NativePowerAmountChanged>(lib, "spire_profiler_power_amount_changed");
        _power_removed = GetExport<NativePowerRemoved>(lib, "spire_profiler_power_removed");
        _power_provenance_invalidate = GetExport<NativePowerProvenanceInvalidate>(lib, "spire_profiler_power_provenance_invalidate");
        _card_generated = GetExport<NativeCardGenerated>(lib, "spire_profiler_card_generated");
        _card_play_started = GetExport<NativeCardPlayStarted>(lib, "spire_profiler_card_play_started");
        _card_play_finished = GetExport<NativeCardPlayFinished>(lib, "spire_profiler_card_play_finished");
        _card_execution_ended = GetExport<NativeCardExecutionEnded>(lib, "spire_profiler_card_execution_ended");
        _orb_channeled = GetExport<NativeOrbChanneled>(lib, "spire_profiler_orb_channeled");
        _orb_context_begin = GetExport<NativeOrbContextBegin>(lib, "spire_profiler_orb_context_begin");
        _damage_calculation_begin = GetExport<NativeDamageCalculationBegin>(lib, "spire_profiler_damage_calculation_begin");
        _damage_modifier_contribution = GetExport<NativeDamageModifierContribution>(lib, "spire_profiler_damage_modifier_contribution");
        _damage_calculation_enemy_hit = GetExport<NativeDamageCalculationEnemyHit>(lib, "spire_profiler_damage_calculation_enemy_hit");
        _damage_calculation_weak_source = GetExport<NativeDamageCalculationWeakSource>(lib, "spire_profiler_damage_calculation_weak_source");
        _damage_result_append = GetExport<NativeDamageResultAppend>(lib, "spire_profiler_damage_result_append");
        _damage_calculation_commit = GetExport<NativeDamageCalculationCommit>(lib, "spire_profiler_damage_calculation_commit");
        _damage_calculation_abort = GetExport<NativeDamageCalculationAbort>(lib, "spire_profiler_damage_calculation_abort");
        _damage_unattributed = GetExport<NativeDamageUnattributed>(lib, "spire_profiler_damage_unattributed");
        _buff_mitigation = GetExport<NativeBuffMitigation>(lib, "spire_profiler_buff_mitigation");
        _block_modifier_contribution = GetExport<NativeBlockModifierContribution>(lib, "spire_profiler_block_modifier_contribution");
        _block_gained = GetExport<NativeBlockGained>(lib, "spire_profiler_block_gained");
        _forge = GetExport<NativeForge>(lib, "spire_profiler_forge");
        _osty_summoned = GetExport<NativeOstySummoned>(lib, "spire_profiler_osty_summoned");
        _osty_killed = GetExport<NativeOstyKilled>(lib, "spire_profiler_osty_killed");
        _doom_batch_begin = GetExport<NativeDoomBatchBegin>(lib, "spire_profiler_doom_batch_begin");
        _doom_target_capture = GetExport<NativeDoomTargetCapture>(lib, "spire_profiler_doom_target_capture");
        _doom_kills_completed = GetExport<NativeDoomKillsCompleted>(lib, "spire_profiler_doom_kills_completed");
        _doom_batch_abort = GetExport<NativeDoomBatchAbort>(lib, "spire_profiler_doom_batch_abort");
    }

    internal static void Init(string dataDir) => _init(dataDir);
    internal static void SelfTest() => _self_test();
    internal static void SetRunMeta(int profileId) => _set_run_meta(profileId);
    internal static void RunStarted(string characterIds, int ascension, string gameMode, string seed, int continued, string netIds, long startTime) => _run_started(characterIds, ascension, gameMode, seed, continued, netIds, startTime);
    internal static void RunEnded(int outcome) => _run_ended(outcome);
    internal static void RunSuspended() => _run_suspended();
    internal static void RunHistorySelect(string seed, long startTime, int profile) => _run_history_select(seed, startTime, profile);
    internal static void RunHistoryClear() => _run_history_clear();
    internal static void PanelToggle() => _panel_toggle();
    internal static void ScrollInput(int panel, int buttonIndex, int pressed, double panY) => _scroll_input(panel, buttonIndex, pressed, panY);
    internal static ulong CombatStarted(string encounterId, string encounterType) => _combat_started(encounterId, encounterType);
    internal static int CombatEnded(ulong combatSeq) => _combat_ended(combatSeq);
    internal static int TurnStarted(ulong combatSeq) => _turn_started(combatSeq);
    internal static int BlockPoolClear(ulong combatSeq, int playerSlot) => _block_pool_clear(combatSeq, playerSlot);
    internal static int PlayerDied(ulong combatSeq, int playerSlot) => _player_died(combatSeq, playerSlot);
    internal static int PotionUsed(ulong combatSeq) => _potion_used(combatSeq);
    internal static ulong SourceCapture(ulong combatSeq, int captureKind, ulong instance, string sourceId, int sourceKind, int sourceSlot, int generationState) => _source_capture(combatSeq, captureKind, instance, sourceId, sourceKind, sourceSlot, generationState);
    internal static int SourceCount(ulong transfer) => _source_count(transfer);
    internal static ulong SourceDestination(ulong transfer, int index) => _source_destination(transfer, index);
    internal static ulong SourceWeight(ulong transfer, int index) => _source_weight(transfer, index);
    internal static ulong SourceTransferBegin(ulong combatSeq) => _source_transfer_begin(combatSeq);
    internal static int SourceTransferAdd(ulong transfer, ulong destination, ulong weight) => _source_transfer_add(transfer, destination, weight);
    internal static int SourceTransferSeal(ulong transfer) => _source_transfer_seal(transfer);
    internal static int SourceTransferRelease(ulong transfer) => _source_transfer_release(transfer);
    internal static int PowerAttached(ulong combatSeq, ulong powerInstance, string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int amount, ulong sourceTransfer) => _power_attached(combatSeq, powerInstance, powerId, ownerCreature, ownerKind, ownerSlot, amount, sourceTransfer);
    internal static int PowerAmountChanged(ulong combatSeq, ulong powerInstance, string powerId, ulong ownerCreature, int ownerKind, int ownerSlot, int oldAmount, int newAmount, ulong sourceTransfer) => _power_amount_changed(combatSeq, powerInstance, powerId, ownerCreature, ownerKind, ownerSlot, oldAmount, newAmount, sourceTransfer);
    internal static int PowerRemoved(ulong combatSeq, ulong powerInstance) => _power_removed(combatSeq, powerInstance);
    internal static int PowerProvenanceInvalidate(ulong combatSeq, ulong powerInstance) => _power_provenance_invalidate(combatSeq, powerInstance);
    internal static int CardGenerated(ulong combatSeq, ulong cardInstance, ulong sourceTransfer, int producerRole) => _card_generated(combatSeq, cardInstance, sourceTransfer, producerRole);
    internal static ulong CardPlayStarted(ulong combatSeq, ulong executionId, ulong cardInstance, string cardId, int playerSlot, int playIndex, int playCount, int generationState, ulong sourceTransfer) => _card_play_started(combatSeq, executionId, cardInstance, cardId, playerSlot, playIndex, playCount, generationState, sourceTransfer);
    internal static int CardPlayFinished(ulong play) => _card_play_finished(play);
    internal static int CardExecutionEnded(ulong combatSeq, ulong executionId) => _card_execution_ended(combatSeq, executionId);
    internal static int OrbChanneled(ulong combatSeq, ulong orbInstance, ulong sourceTransfer) => _orb_channeled(combatSeq, orbInstance, sourceTransfer);
    internal static int OrbContextBegin(ulong combatSeq, ulong orbInstance, ulong play, int ownerSlot) => _orb_context_begin(combatSeq, orbInstance, play, ownerSlot);
    internal static ulong DamageCalculationBegin(ulong combatSeq, ulong sourceTransfer, int producerRole, int segment, ulong originalTarget) => _damage_calculation_begin(combatSeq, sourceTransfer, producerRole, segment, originalTarget);
    internal static int DamageModifierContribution(ulong calculation, ulong sourceTransfer, int amount) => _damage_modifier_contribution(calculation, sourceTransfer, amount);
    internal static int DamageCalculationEnemyHit(ulong calculation, ulong dealerCreature, int baseDamage, int dealerStrength) => _damage_calculation_enemy_hit(calculation, dealerCreature, baseDamage, dealerStrength);
    internal static int DamageCalculationWeakSource(ulong calculation, ulong sourceTransfer) => _damage_calculation_weak_source(calculation, sourceTransfer);
    internal static int DamageResultAppend(ulong calculation, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented) => _damage_result_append(calculation, total, unblocked, blocked, resultKind, receiverSlot, weakPrevented);
    internal static int DamageCalculationCommit(ulong calculation) => _damage_calculation_commit(calculation);
    internal static int DamageCalculationAbort(ulong calculation) => _damage_calculation_abort(calculation);
    internal static int DamageUnattributed(ulong combatSeq, int total, int unblocked, int blocked, int resultKind, int receiverSlot, int weakPrevented) => _damage_unattributed(combatSeq, total, unblocked, blocked, resultKind, receiverSlot, weakPrevented);
    internal static int BuffMitigation(ulong combatSeq, ulong sourceTransfer, int prevented) => _buff_mitigation(combatSeq, sourceTransfer, prevented);
    internal static int BlockModifierContribution(ulong combatSeq, ulong sourceTransfer, int amount, int receiverSlot) => _block_modifier_contribution(combatSeq, sourceTransfer, amount, receiverSlot);
    internal static int BlockGained(ulong combatSeq, int amount, ulong sourceTransfer, int receiverSlot) => _block_gained(combatSeq, amount, sourceTransfer, receiverSlot);
    internal static int Forge(ulong combatSeq, ulong sourceTransfer, int amount) => _forge(combatSeq, sourceTransfer, amount);
    internal static int OstySummoned(ulong combatSeq, ulong sourceTransfer, int hpAmount, int ownerSlot) => _osty_summoned(combatSeq, sourceTransfer, hpAmount, ownerSlot);
    internal static int OstyKilled(ulong combatSeq, int ownerSlot, ulong play) => _osty_killed(combatSeq, ownerSlot, play);
    internal static ulong DoomBatchBegin(ulong combatSeq) => _doom_batch_begin(combatSeq);
    internal static int DoomTargetCapture(ulong batch, ulong creatureInstance, ulong doomPowerInstance, int currentHp) => _doom_target_capture(batch, creatureInstance, doomPowerInstance, currentHp);
    internal static int DoomKillsCompleted(ulong batch) => _doom_kills_completed(batch);
    internal static int DoomBatchAbort(ulong batch) => _doom_batch_abort(batch);

    internal static void OnSetRunMeta(int profile) { if (CaptureRuntime.OnThread) SetRunMeta(profile); }
    internal static void OnRunStarted(string ids, int ascension, string mode, string seed, bool continued, string netIds, long startedAt)
    { if (CaptureRuntime.OnThread) RunStarted(ids, ascension, mode, seed, continued ? 1 : 0, netIds, startedAt); }
    internal static void OnRunEnded(int outcome) { if (CaptureRuntime.OnThread) RunEnded(outcome); }
    internal static void OnRunSuspended() { if (CaptureRuntime.OnThread) RunSuspended(); }
    internal static void OnRunHistorySelect(string seed, long startedAt, int profile) { if (CaptureRuntime.OnThread) RunHistorySelect(seed, startedAt, profile); }
    internal static void OnRunHistoryClear() { if (CaptureRuntime.OnThread) RunHistoryClear(); }
    internal static void OnPanelToggle() { if (CaptureRuntime.OnThread) PanelToggle(); }
    internal static void OnScrollInput(int panel, int button, bool pressed, double pan) { if (CaptureRuntime.OnThread) ScrollInput(panel, button, pressed ? 1 : 0, pan); }
}

// Run and UI lifecycle patches are installed alongside the capture layer.

[HarmonyPatch(typeof(RunManager), nameof(RunManager.SetUpNewSingleplayer))]
internal static class PatchRunStartSingleplayer
{
    [HarmonyPostfix]
    private static void Postfix(RunState state)
    {
        try { RunStartPatches.NotifyRunStarted(state, isResume: false); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] SetUpNewSingleplayer: {ex}"); }
    }
}

[HarmonyPatch(typeof(RunManager), nameof(RunManager.SetUpNewMultiplayer))]
internal static class PatchRunStartMultiplayer
{
    [HarmonyPostfix]
    private static void Postfix(RunState state)
    {
        try { RunStartPatches.NotifyRunStarted(state, isResume: false); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] SetUpNewMultiplayer: {ex}"); }
    }
}

// Resumed runs (save+quit continue): without these, combats in a continued
// run would be recorded run-less.
[HarmonyPatch(typeof(RunManager), nameof(RunManager.SetUpSavedSingleplayer))]
internal static class PatchRunResumeSingleplayer
{
    [HarmonyPostfix]
    private static void Postfix(RunState state)
    {
        try { RunStartPatches.NotifyRunStarted(state, isResume: true); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] SetUpSavedSingleplayer: {ex}"); }
    }
}

[HarmonyPatch(typeof(RunManager), nameof(RunManager.SetUpSavedMultiplayer))]
internal static class PatchRunResumeMultiplayer
{
    [HarmonyPostfix]
    private static void Postfix(RunState state)
    {
        try { RunStartPatches.NotifyRunStarted(state, isResume: true); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] SetUpSavedMultiplayer: {ex}"); }
    }
}

/// Shared run-start postfix body for the four RunManager.SetUp* patches:
/// the run meta (profile id + build commit) must reach the core before the
/// run record it will stamp is opened, then the record is opened with the
/// roster (character ids + NetIds, positional), the run identity, and the
/// resumed flag (true only for the SetUpSaved* variants). Plain helper
/// class — no class-level [HarmonyPatch], so the auto-patch loop skips it.
internal static class RunStartPatches
{
    internal static void NotifyRunStarted(RunState state, bool isResume)
    {
        if (!CaptureRuntime.OnThread) return;
        // The game's run id IS its own StartTime: assigned in
        // RunManager.InitializeShared (UtcNow for a fresh run, the
        // original run's start for a resume), already set before these
        // SetUp* postfixes run. Forwarding it makes run-history matching
        // exact — both sides carry the same integer by provenance. The
        // read degrades to 0 on failure, leaving the identity unknown.
        long startTime = 0;
        try
        {
            startTime = Traverse.Create(RunManager.Instance)?.Field("_startTime")?.GetValue<long>() ?? 0;
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] NotifyRunStarted.StartTime: {ex}"); }

        // The slot registry must exist before the first combat event; run
        // start is the earliest event (and re-fires on resume, refilling
        // the same session-immutable player list).
        CaptureRuntime.InvalidateEpoch();
        SpireProfilerMod.CaptureRunPlayers(state);
        ProfilerNative.OnSetRunMeta(SpireProfilerMod.CurrentProfileId());
        ProfilerNative.OnRunStarted(
            SpireProfilerMod.CharacterIds(state),
            state?.AscensionLevel ?? 0,
            state?.GameMode.ToString() ?? "Standard",
            state?.Rng?.StringSeed ?? "",
            isResume,
            SpireProfilerMod.NetIds(state),
            startTime);
    }
}

// ── Run-history screen ──────────────────────────────────────────────────────
// The screen displays one RunHistory per open (left/right arrows switch via
// RefreshAndSelectRun); these postfixes forward the displayed run's identity
// to the core, which matches it against the profiler's own run records, and
// keep the selection scoped to the screen's lifecycle. The run panel +
// toggle button attach lazily on the first display.

/// <summary>
/// DisplayRun(RunHistory) fires for every displayed run — initial open
/// and each left/right switch. The postfix reads the
/// displayed run straight from the argument (Seed + StartTime are the join
/// keys; identical seed provenance on both sides via RunRngSet.StringSeed)
/// and forwards them with the active profile id.
/// </summary>
[HarmonyPatch(typeof(NRunHistory), "DisplayRun")]
internal static class PatchRunHistoryDisplay
{
    [HarmonyPostfix]
    private static void Postfix(NRunHistory __instance, RunHistory history)
    {
        try
        {
            ProfilerNative.OnRunHistorySelect(
                history?.Seed ?? "",
                history?.StartTime ?? 0,
                SpireProfilerMod.CurrentProfileId());
            SpireProfilerMod.AttachRunPanelTo(__instance);
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunHistoryDisplay: {ex}"); }
    }
}

/// <summary>
/// OnSubmenuOpened fires only on a fresh push. The clear must be a
/// PREFIX, not a postfix: despite the `TaskHelper.RunSafely` wrapper,
/// `RefreshAndSelectRun` is synchronous (no await before `DisplayRun`),
/// so the initial `DisplayRun` — and
/// our select postfix — runs INSIDE the original method. A postfix clear
/// would run after that select and blank the screen-open flag, breaking F8
/// on the first displayed run until an arrow flip re-selects. Clearing
/// first drops any stale selection before the body loads the first run,
/// whose `DisplayRun` postfix then re-selects (a failed load keeps the
/// screen marked closed, so no stale numbers render).
/// </summary>
[HarmonyPatch(typeof(NRunHistory), nameof(NRunHistory.OnSubmenuOpened))]
internal static class PatchRunHistoryOpened
{
    [HarmonyPrefix]
    private static void Prefix()
    {
        try { ProfilerNative.OnRunHistoryClear(); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunHistoryOpened: {ex}"); }
    }
}

/// <summary>
/// The screen's close path is the NSubmenu.OnSubmenuClosed virtual
/// (popped from its stack); NRunHistory does not override it, so the
/// patch targets the base virtual and filters on the
/// instance. Only the run-history screen's own pop clears — a submenu
/// closing on top of it must not blank a still-displayed run (which only
/// re-selects on the next arrow flip or re-push).
/// </summary>
[HarmonyPatch(typeof(NSubmenu), nameof(NSubmenu.OnSubmenuClosed))]
internal static class PatchRunHistoryClosed
{
    [HarmonyPostfix]
    private static void Postfix(NSubmenu __instance)
    {
        try
        {
            if (!(__instance is NRunHistory)) return;
            ProfilerNative.OnRunHistoryClear();
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunHistoryClosed: {ex}"); }
    }
}

/// <summary>
/// The run-end chokepoint: victory (WinRun), all-dead defeat, and abandon
/// (the force-kill that ends an abandoned run) all funnel through
/// RunManager.OnEnded on every peer. OnEnded's bool cannot distinguish
/// abandon from defeat, but RunManager.IsAbandoned can — the force-kill
/// leaves the flag set — so the postfix maps the three run terminals to
/// 0 = victory, 1 = defeat, 2 = abandoned. OnEnded can fire twice
/// (victory's follow-up all-dead kill); the second run_ended is a
/// core-side no-op because the record is already closed. Save&quit and
/// multiplayer-disconnect do not reach OnEnded and stay handled by the
/// suspend patches below.
/// </summary>
[HarmonyPatch(typeof(RunManager), nameof(RunManager.OnEnded))]
internal static class PatchRunEnded
{
    [HarmonyPostfix]
    private static void Postfix(RunManager __instance, bool isVictory)
    {
        try
        {
            int outcome = isVictory
                ? 0
                : (__instance != null && __instance.IsAbandoned ? 2 : 1);
            CaptureRuntime.InvalidateEpoch();
            ProfilerNative.OnRunEnded(outcome);
            Log.Info($"[SpireProfiler] run ended, outcome={outcome}");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunEnded: {ex}"); }
    }
}

/// <summary>
/// Save & exit forwards run_suspended, not run_ended: the save-and-quit
/// path (NPauseMenu → CloseToMenu → NGame.ReturnToMainMenu) never
/// reaches RunManager.OnEnded, so without this postfix the core
/// would keep the run active — the combat panel stays visible on the main
/// menu, and the next continue's run_started closes the stale run as a
/// spurious defeat. The postfix targets RunManager.CleanUp, the
/// synchronous teardown both menu-exit funnels (ReturnToMainMenu and
/// GoToTimeline) pass through, rather than the async ReturnToMainMenu. A
/// run that truly ended (victory/defeat/abandon) already cleared `active`,
/// so the forwarded suspend is a core-side no-op.
/// </summary>
[HarmonyPatch(typeof(RunManager), nameof(RunManager.CleanUp))]
internal static class PatchRunSuspend
{
    [HarmonyPostfix]
    private static void Postfix()
    {
        try
        {
            CaptureRuntime.InvalidateEpoch();
            ProfilerNative.OnRunSuspended();
            SpireProfilerMod.CaptureRunPlayers(null);
            Log.Info("[SpireProfiler] run suspended (save & quit); no record written");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunManager.CleanUp: {ex}"); }
    }
}

/// <summary>
/// A host quit or a dropped connection sends the client back to the main
/// menu with no end-of-run signal (LocalPlayerDisconnected →
/// ReturnToMainMenuWithError) — no OnEnded. The
/// disconnect reason cannot distinguish "host saved and quit" (the run is
/// resumable) from "host quit without saving", so the run must be
/// SUSPENDED, not closed: run_suspended deactivates without writing a
/// record, and a later resume rejoins by profile, seed, and original
/// StartTime (closing as defeat here would split one run across two records). The
/// game-over case never reaches this forward: QuitGameOver disconnects are
/// skipped below, and an already-abandoned run is closed by the OnEnded
/// postfix. StateDivergence disconnects suspend as well.
/// </summary>
[HarmonyPatch(typeof(RunManager), nameof(RunManager.LocalPlayerDisconnected))]
internal static class PatchRunDisconnected
{
    [HarmonyPostfix]
    private static void Postfix(NetErrorInfo info)
    {
        try
        {
            var runManager = RunManager.Instance;
            if (info.GetReason() == NetError.QuitGameOver) return;
            if (runManager == null || runManager.IsAbandoned) return;
            CaptureRuntime.InvalidateEpoch();
            ProfilerNative.OnRunSuspended();
            SpireProfilerMod.CaptureRunPlayers(null);
            Log.Info("[SpireProfiler] run suspended after multiplayer disconnect; no record written");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunDisconnected: {ex}"); }
    }
}

internal sealed class NativeAttributionBackend : GameAttributionBackend
{
    internal override object CurrentCombat => SpireProfilerMod.CurrentCombat;
    internal override ulong Capture(ulong epoch, CaptureKind kind, ulong instance, string id, int sourceKind, int slot, GenerationState generation)
        => ProfilerNative.SourceCapture(epoch, (int)kind, instance, id, sourceKind, slot, (int)generation);
    internal override int SourceCount(ulong transfer) => ProfilerNative.SourceCount(transfer);
    internal override ulong SourceDestination(ulong transfer, int index) => ProfilerNative.SourceDestination(transfer, index);
    internal override ulong SourceWeight(ulong transfer, int index) => ProfilerNative.SourceWeight(transfer, index);
    internal override ulong TransferBegin(ulong epoch) => ProfilerNative.SourceTransferBegin(epoch);
    internal override int TransferAdd(ulong transfer, ulong destination, ulong weight) => ProfilerNative.SourceTransferAdd(transfer, destination, weight);
    internal override int TransferSeal(ulong transfer) => ProfilerNative.SourceTransferSeal(transfer);
    internal override int TransferRelease(ulong transfer) => ProfilerNative.SourceTransferRelease(transfer);
    internal override int PowerAttached(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, ulong source)
        => ProfilerNative.PowerAttached(epoch.Sequence, identity, observed.Id, owner, observed.OwnerKind, observed.OwnerSlot, observed.Amount, source);
    internal override int PowerChanged(CaptureEpoch epoch, ulong identity, ulong owner, PowerObservation observed, int before, ulong source)
        => ProfilerNative.PowerAmountChanged(epoch.Sequence, identity, observed.Id, owner, observed.OwnerKind, observed.OwnerSlot, before, observed.Amount, source);
    internal override int PowerRemoved(ulong epoch, ulong identity) => ProfilerNative.PowerRemoved(epoch, identity);
    internal override int PowerInvalidate(ulong epoch, ulong identity) => ProfilerNative.PowerProvenanceInvalidate(epoch, identity);
    internal override int CardGenerated(ulong epoch, ulong identity, ulong source, ProducerRole role) => ProfilerNative.CardGenerated(epoch, identity, source, (int)role);
    internal override ulong PlayStarted(ulong epoch, ulong execution, ulong card, string id, int slot, int index, int count, GenerationState generation, ulong source)
        => ProfilerNative.CardPlayStarted(epoch, execution, card, id, slot, index, count, (int)generation, source);
    internal override int PlayFinished(ulong play) => ProfilerNative.CardPlayFinished(play);
    internal override int ExecutionEnded(ulong epoch, ulong execution) => ProfilerNative.CardExecutionEnded(epoch, execution);
    internal override int OrbBegin(ulong epoch, ulong orb, ulong play, int ownerSlot) => ProfilerNative.OrbContextBegin(epoch, orb, play, ownerSlot);
    internal override int OrbChanneled(ulong epoch, ulong orb, ulong source) => ProfilerNative.OrbChanneled(epoch, orb, source);
    internal override ulong DamageBegin(ulong epoch, ulong source, ProducerRole role, DamageSegment segment, ulong target)
        => ProfilerNative.DamageCalculationBegin(epoch, source, (int)role, (int)segment, target);
    internal override int DamageModifier(ulong calculation, ulong source, int amount) => ProfilerNative.DamageModifierContribution(calculation, source, amount);
    internal override int DamageEnemyHit(ulong calculation, ulong dealer, int baseDamage, int strength) => ProfilerNative.DamageCalculationEnemyHit(calculation, dealer, baseDamage, strength);
    internal override int DamageWeak(ulong calculation, ulong source) => ProfilerNative.DamageCalculationWeakSource(calculation, source);
    internal override int DamageAppend(ulong calculation, ResultPacket packet)
        => ProfilerNative.DamageResultAppend(calculation, packet.Total, packet.Unblocked, packet.Blocked, (int)packet.Kind, packet.ReceiverSlot, packet.WeakPrevented);
    internal override int DamageCommit(ulong calculation) => ProfilerNative.DamageCalculationCommit(calculation);
    internal override int DamageAbort(ulong calculation) => ProfilerNative.DamageCalculationAbort(calculation);
    internal override int DamageFallback(ulong epoch, ResultPacket packet)
        => ProfilerNative.DamageUnattributed(epoch, packet.Total, packet.Unblocked, packet.Blocked, (int)packet.Kind, packet.ReceiverSlot, packet.WeakPrevented);
    internal override int BlockGained(ulong epoch, int amount, ulong source, int slot) => ProfilerNative.BlockGained(epoch, amount, source, slot);
    internal override int BlockModifier(ulong epoch, ulong source, int amount, int slot) => ProfilerNative.BlockModifierContribution(epoch, source, amount, slot);
    internal override int Forge(ulong epoch, ulong source, int amount) => ProfilerNative.Forge(epoch, source, amount);
    internal override int OstySummoned(ulong epoch, ulong source, int hp, int slot) => ProfilerNative.OstySummoned(epoch, source, hp, slot);
    internal override int OstyKilled(ulong epoch, int slot, ulong play) => ProfilerNative.OstyKilled(epoch, slot, play);
    internal override int BuffMitigation(ulong epoch, ulong source, int prevented) => ProfilerNative.BuffMitigation(epoch, source, prevented);
    internal override ulong DoomBegin(ulong epoch) => ProfilerNative.DoomBatchBegin(epoch);
    internal override int DoomTarget(ulong batch, ulong creature, ulong power, int hp) => ProfilerNative.DoomTargetCapture(batch, creature, power, hp);
    internal override int DoomComplete(ulong batch) => ProfilerNative.DoomKillsCompleted(batch);
    internal override int DoomAbort(ulong batch) => ProfilerNative.DoomBatchAbort(batch);
    internal override int TurnStarted(ulong epoch) => ProfilerNative.TurnStarted(epoch);
    internal override int BlockCleared(ulong epoch, int slot) => ProfilerNative.BlockPoolClear(epoch, slot);
    internal override int PlayerDied(ulong epoch, int slot) => ProfilerNative.PlayerDied(epoch, slot);
    internal override int PotionUsed(ulong epoch) => ProfilerNative.PotionUsed(epoch);
    internal override ulong CombatStarted(string encounter, string type) => ProfilerNative.CombatStarted(encounter, type);
    internal override int CombatEnded(ulong epoch) => ProfilerNative.CombatEnded(epoch);
    internal override void Diagnostic(string category, Exception error) => Log.Error($"[SpireProfiler] attribution {category}: {error?.Message}");
}
