using System;
using HarmonyLib;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Screens.MainMenu;
using MegaCrit.Sts2.Core.Nodes.Screens.RunHistoryScreen;
using MegaCrit.Sts2.Core.Runs;

namespace SpireProfiler;

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
                RunContext.CurrentProfileId());
            ProfilerPanels.AttachRunPanelTo(__instance);
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
