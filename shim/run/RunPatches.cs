using System;
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
        RunContext.CaptureRunPlayers(state);
        ProfilerNative.OnSetRunMeta(RunContext.CurrentProfileId());
        ProfilerNative.OnRunStarted(
            RunContext.CharacterIds(state),
            state?.AscensionLevel ?? 0,
            state?.GameMode.ToString() ?? "Standard",
            state?.Rng?.StringSeed ?? "",
            isResume,
            RunContext.NetIds(state),
            startTime);
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
            RunContext.CaptureRunPlayers(null);
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
            RunContext.CaptureRunPlayers(null);
            Log.Info("[SpireProfiler] run suspended after multiplayer disconnect; no record written");
        }
        catch (Exception ex) { Log.Error($"[SpireProfiler] RunDisconnected: {ex}"); }
    }
}
