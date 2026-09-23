using System;
using HarmonyLib;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Nodes.Screens.MainMenu;
using MegaCrit.Sts2.Core.Nodes.Screens.RunHistoryScreen;
using MegaCrit.Sts2.Core.Runs;

namespace SpireProfiler;

[HarmonyPatch(typeof(NRunHistory), "DisplayRun")]
internal static class PatchRunHistoryDisplay
{
    [HarmonyPostfix]
    private static void Postfix(NRunHistory __instance, RunHistory history)
    {
        try
        {
            ProfilerSession.SelectHistory(history?.Seed ?? "", history?.StartTime ?? 0, RunContext.CurrentProfileId());
            ProfilerPanels.AttachRunPanelTo(__instance);
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunHistoryDisplay: {ex}"); }
    }
}

[HarmonyPatch(typeof(NRunHistory), nameof(NRunHistory.OnSubmenuOpened))]
internal static class PatchRunHistoryOpened
{
    // DisplayRun executes inside OnSubmenuOpened, so stale selection must clear first.
    [HarmonyPrefix]
    private static void Prefix()
    {
        try { ProfilerSession.ClearHistory(); }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunHistoryOpened: {ex}"); }
    }
}

[HarmonyPatch(typeof(NSubmenu), nameof(NSubmenu.OnSubmenuClosed))]
internal static class PatchRunHistoryClosed
{
    [HarmonyPostfix]
    private static void Postfix(NSubmenu __instance)
    {
        try
        {
            if (__instance is NRunHistory) ProfilerSession.ClearHistory();
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunHistoryClosed: {ex}"); }
    }
}
