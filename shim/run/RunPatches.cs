using System;
using System.Linq;
using HarmonyLib;
using MegaCrit.Sts2.Core.Entities.Multiplayer;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Runs;

namespace SpireProfiler;

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

/// Run identity and roster are captured before combat observations begin.
internal static class RunStartPatches
{
    internal static void NotifyRunStarted(RunState state, bool isResume)
    {
        if (!CaptureRuntime.OnThread) return;
        // InitializeShared restores the original StartTime on resume; history
        // identity uses this value, never the time when the mod observes it.
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
        RunContext.CaptureRunPlayers(state);
        ProfilerSession.StartRun(new RunRecord
        {
            Profile = RunContext.CurrentProfileId(),
            Character = RunContext.CharacterIds(state),
            Ascension = state?.AscensionLevel ?? 0,
            GameMode = state?.GameMode.ToString() ?? "Standard",
            Seed = state?.Rng?.StringSeed ?? "",
            StartedAt = Math.Max(startTime, 0),
            Players = Array.AsReadOnly(state?.Players?.Take(4)
                .Select((player, slot) => new PlayerSummary(slot, player.Character?.Id?.Entry ?? "?"))
                .ToArray() ?? Array.Empty<PlayerSummary>())
        }, isResume);
    }
}

/// <summary>
/// OnEnded reaches every peer for victory, all-dead defeat, and abandonment.
/// IsAbandoned distinguishes the two losing outcomes. Victory's follow-up
/// all-dead kill can notify twice; the session closes the run once.
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
            ProfilerSession.EndRun(outcome);
            Log.Info($"[SpireProfiler] run ended, outcome={outcome}");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunEnded: {ex}"); }
    }
}

/// <summary>
/// Both menu-exit paths reach synchronous CleanUp. Save-and-quit never reaches
/// OnEnded, so it suspends the run and discards its uncompleted combat.
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
            ProfilerSession.Suspend();
            RunContext.CaptureRunPlayers(null);
            Log.Info("[SpireProfiler] run suspended (save & quit)");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunManager.CleanUp: {ex}"); }
    }
}

/// <summary>
/// Disconnect cannot distinguish a resumable host save from an unsaved quit.
/// Suspend preserves the original identity for a later reconnect; OnEnded
/// remains the authority for terminal outcomes.
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
            ProfilerSession.Suspend();
            RunContext.CaptureRunPlayers(null);
            Log.Info("[SpireProfiler] run suspended after multiplayer disconnect");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunDisconnected: {ex}"); }
    }
}
