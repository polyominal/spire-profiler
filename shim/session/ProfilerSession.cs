using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace SpireProfiler;

// The game thread owns one session. Native state covers a combat; this owner
// supplies time and identity, persists completed observations, and publishes views.
internal static class ProfilerSession
{
    private static StatisticsStore store;
    private static RunRecord run;
    private static RunRecord combatRun;
    private static SummaryView completedRun;
    private static CombatStatistics combat;
    private static uint? sequence;
    private static ulong activeEpoch;
    private static ulong nativeRevision;
    private static int thread;
    private static bool recording;
    private static Action<string> report;
    internal static SummaryView CurrentCombat { get; private set; }
    internal static SummaryView CurrentRun { get; private set; }
    internal static SummaryView SelectedHistory { get; private set; }
    internal static bool InRun => run != null;
    internal static bool HistoryOpen { get; private set; }
    internal static ulong Revision { get; private set; }
    internal static ulong LiveFilterGeneration { get; private set; }
    internal static ulong HistoryClearGeneration { get; private set; }

    internal static void Initialize(string dataDirectory, string gameVersion, string modVersion, Action<string> diagnostic)
    {
        thread = Environment.CurrentManagedThreadId;
        report = diagnostic;
        store = new StatisticsStore(dataDirectory, gameVersion, modVersion, diagnostic);
        run = null;
        combatRun = null;
        completedRun = null;
        combat = null;
        CurrentCombat = CurrentRun = SelectedHistory = null;
        HistoryOpen = false;
        LiveFilterGeneration++;
        HistoryClearGeneration++;
        activeEpoch = 0;
        sequence = store.MaxCombatId();
        PoisonAudit.Initialize(dataDirectory, gameVersion, modVersion, diagnostic, Environment.GetEnvironmentVariable("SPIRE_PROFILER_AUDIT") == "1");
        recording = PoisonAudit.Enabled || Environment.GetEnvironmentVariable("SPIRE_PROFILER_RECORD") == "1";
        ProfilerNative.CombatDiscard();
        nativeRevision = ProfilerNative.Revision;
        Revision++;
    }

    internal static void StartRun(RunRecord requested, bool continued)
    {
        if (!OnThread()) return;
        if (run != null)
            store.SaveRun(run with { Outcome = "defeat", EndedAt = DateTimeOffset.UtcNow.ToUnixTimeSeconds() });
        run = store.OpenRun(requested, continued);
        completedRun = run?.EmptySummary();
        if (run == null)
        {
            PoisonAudit.Finish("interrupted", combat?.Coverage);
            ProfilerNative.CombatDiscard();
            activeEpoch = 0;
            combat = null;
            combatRun = null;
            CurrentCombat = null;
        }
        else if (continued) completedRun = store.LoadRun(run).Summary;
        CurrentRun = completedRun;
        LiveFilterGeneration++;
        Revision++;
    }

    internal static ulong StartCombat(string encounter, string type)
    {
        if (!OnThread()) return 0;
        if (activeEpoch != 0)
        {
            Refresh();
            if (combat != null && (combat.Cards.Count != 0 || combat.Plays != 0))
                Finish(combat with { Result = "interrupted", Coverage = combat.Coverage.WithFailure("interrupted-capture") });
        }
        PoisonAudit.Finish("interrupted", combat?.Coverage);
        ProfilerNative.CombatDiscard();
        activeEpoch = 0;
        combat = null;
        combatRun = null;
        CurrentCombat = null;
        if (sequence == null || sequence == uint.MaxValue) { report("combat sequence unavailable or exhausted"); return 0; }
        combatRun = run;
        bool recordingStarted = !recording || ProfilerNative.RecordingBegin();
        sequence++;
        PoisonAudit.Start(combatRun, sequence.Value, encounter);
        activeEpoch = ProfilerNative.CombatStarted(sequence.Value, encounter, type, DateTimeOffset.UtcNow.ToUnixTimeSeconds(), combatRun?.Players.Count ?? 0);
        if (!recordingStarted) ReportFailure("recording-start-failed");
        combat = null;
        CurrentCombat = null;
        nativeRevision = ulong.MaxValue;
        Refresh();
        return activeEpoch;
    }

    internal static int EndCombat(ulong epoch)
    {
        if (!OnThread() || epoch == 0 || epoch != activeEpoch) return 0;
        PoisonAudit.EndCheckpoint();
        bool ended = ProfilerNative.CombatEnded(epoch) == 1;
        if (!ended) ReportFailure("combat-end");
        bool refreshed = Refresh();
        if (combat == null) { PoisonAudit.Finish("interrupted", null); activeEpoch = 0; return 0; }
        if (!ended || !refreshed) combat = combat with { Result = "interrupted" };
        Finish(combat);
        return 1;
    }

    private static void Finish(CombatStatistics finished)
    {
        var record = new CombatRecord
        {
            GameVersion = store.GameVersion,
            ModVersion = store.ModVersion,
            RunId = combatRun?.RunId ?? "0",
            Ordinal = finished.CombatId,
            Run = combatRun,
            Combat = finished
        };
        combat = finished;
        CurrentCombat = finished.View(combatRun?.Players ?? Array.Empty<PlayerSummary>(), combatRun);
        if (run != null && combatRun != null && run.RunId == combatRun.RunId
            && run.Seed == combatRun.Seed && run.Profile == combatRun.Profile && run.StartedAt == combatRun.StartedAt)
        {
            completedRun = completedRun.Add(CurrentCombat);
            CurrentRun = completedRun;
        }
        if (!store.SaveCombat(record))
        {
            combat = finished with { Coverage = finished.Coverage.WithFailure("statistics-write-failed") };
            CurrentCombat = combat.View(combatRun?.Players ?? Array.Empty<PlayerSummary>(), combatRun);
        }
        if (recording) store.SaveTrace(record.RunId, record.Ordinal, ProfilerNative.Recording());
        PoisonAudit.Finish(finished.Result, combat.Coverage);
        activeEpoch = 0;
        Revision++;
    }

    internal static bool Refresh()
    {
        if (!OnThread() || activeEpoch == 0) return false;
        ulong next = ProfilerNative.Revision;
        if (next == nativeRevision) return combat != null;
        try
        {
            var snapshot = StatisticsJson.ParseNative(ProfilerNative.Snapshot());
            if (snapshot == null || snapshot.CombatId != activeEpoch) throw new InvalidDataException("Native snapshot differs from active combat");
            combat = snapshot;
            CurrentCombat = snapshot.View(combatRun?.Players ?? Array.Empty<PlayerSummary>(), combatRun);
            nativeRevision = next;
            Revision++;
            return true;
        }
        catch (Exception ex) when (ex is InvalidDataException or IOException or System.Text.Json.JsonException or OverflowException or InvalidOperationException or ArgumentException)
        {
            report($"cannot read attribution snapshot: {ex.Message}");
            PoisonAudit.Diagnostic("snapshot-read-failed: " + ex.Message);
            combat = combat == null ? null : combat with { Coverage = combat.Coverage.WithFailure("snapshot-read-failed") };
            CurrentCombat = combat?.View(combatRun?.Players ?? Array.Empty<PlayerSummary>(), combatRun);
            Revision++;
            return false;
        }
    }

    internal static void EndRun(int outcome)
    {
        if (!OnThread() || run == null) return;
        var ended = run with
        {
            Outcome = outcome switch { 0 => "victory", 1 => "defeat", 2 => "abandoned", _ => "defeat" },
            EndedAt = DateTimeOffset.UtcNow.ToUnixTimeSeconds()
        };
        store.SaveRun(ended);
        CurrentRun = completedRun with { Outcome = ended.Outcome, EndedAt = ended.EndedAt };
        run = null;
        Revision++;
    }

    internal static void Suspend()
    {
        if (!OnThread()) return;
        bool suspended = run != null;
        PoisonAudit.Finish("interrupted", combat?.Coverage);
        ProfilerNative.CombatDiscard();
        activeEpoch = 0;
        combat = null;
        combatRun = null;
        run = null;
        completedRun = null;
        CurrentCombat = CurrentRun = null;
        LiveFilterGeneration++;
        if (suspended) ClearHistory();
        Revision++;
    }

    internal static void SelectHistory(string seed, long startedAt, int profile)
    {
        if (!OnThread()) return;
        var identity = RunIdentity.Parse(profile, seed, startedAt);
        SelectedHistory = identity == null ? null : store.Select(identity);
        HistoryOpen = true;
        Revision++;
    }

    internal static void ClearHistory()
    {
        if (!OnThread()) return;
        SelectedHistory = null;
        HistoryOpen = false;
        HistoryClearGeneration++;
        Revision++;
    }

    internal static void ReportFailure(string reason)
    {
        if (!OnThread() || activeEpoch == 0) return;
        PoisonAudit.Diagnostic(reason);
        ProfilerNative.CaptureFailed(reason);
    }

    private static bool OnThread() => store != null && Environment.CurrentManagedThreadId == thread;

    internal static void SelfTest(Action<string> message)
    {
        StartRun(new RunRecord
        {
            Profile = 97,
            Seed = "PROFILER-SELF-TEST",
            StartedAt = 1_786_579_200,
            Character = "IRONCLAD",
            GameMode = "Standard",
            Players = Array.AsReadOnly(new[] { new PlayerSummary(0, "IRONCLAD") })
        }, continued: false);
        ProfilerNative.RecordingBegin();
        ulong epoch = StartCombat("CULTIST", "Normal");
        ulong source = ProfilerNative.SourceCapture(epoch, 1, 1, "STRIKE_IRONCLAD", 0, 0, 0);
        ulong hit = ProfilerNative.DamageCalculationBegin(epoch, source, 1, 0, 99);
        if (epoch == 0 || source == 0 || hit == 0
            || ProfilerNative.TurnStarted(epoch) != 1
            || ProfilerNative.DamageResultAppend(hit, 6, 4, 2, 0, 4, 0) != 1
            || ProfilerNative.DamageCalculationCommit(hit) != 1
            || ProfilerNative.BlockGained(epoch, 5, source, 0, Array.Empty<BlockModifier>(), false) != 1
            || ProfilerNative.DamageUnattributed(epoch, 3, 0, 3, 1, 0, 0) != 1
            || ProfilerNative.Forge(epoch, source, 2) != 1
            || EndCombat(epoch) != 1)
            throw new InvalidOperationException("Native observation self-test failed");
        string expected = ProfilerNative.Snapshot();
        string replayed = ProfilerNative.Replay(ProfilerNative.Recording());
        if (replayed != expected) throw new InvalidOperationException("Observation replay changed the result");
        var row = CurrentCombat.Cards.Single(value => value.Id == "STRIKE_IRONCLAD");
        if (row.DamageDealt != 6 || row.DamageBlocked != 2 || row.BlockEffective != 3 || row.Forge != 2)
            throw new InvalidOperationException("Native accounting self-test failed");
        message("[SpireProfiler] managed session self-test: PASS");
        EndRun(0);
        SelectHistory("PROFILER-SELF-TEST", 1_786_579_200, 97);
        if (SelectedHistory == null || SelectedHistory.Combats != 1 || SelectedHistory.Cards.Single().DamageDealt != 6
            || SelectedHistory.Outcome != "victory") throw new InvalidOperationException("Stored summary self-test failed");
        message("[SpireProfiler] managed records: PASS");
        ClearHistory();
    }
}
