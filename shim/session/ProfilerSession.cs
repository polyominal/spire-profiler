using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace SpireProfiler;

// The game thread owns one session. Native state covers a combat; this owner
// supplies time and identity, persists completed observations, and publishes views.
internal static class ProfilerSession
{
    private const int PendingWriteLimit = 64;
    private static StatisticsStore store;
    private static RunRecord run;
    private static RunRecord combatRun;
    private static SummaryView completedRun;
    private static CombatStatistics combat;
    private static readonly Queue<CombatRecord> pendingCombats = new();
    private static readonly Dictionary<string, RunRecord> pendingHeaders = new(StringComparer.Ordinal);
    private static uint sequence;
    private static uint ordinal;
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
        activeEpoch = 0;
        ordinal = 0;
        pendingCombats.Clear();
        pendingHeaders.Clear();
        recording = Environment.GetEnvironmentVariable("SPIRE_PROFILER_RECORD") == "1";
        ProfilerNative.CombatDiscard();
        nativeRevision = ProfilerNative.Revision;
        Revision++;
    }

    internal static void StartRun(RunRecord requested, bool continued)
    {
        if (!OnThread()) return;
        Suspend();
        run = store.OpenRun(requested, continued);
        if (continued && requested.Identity != null)
        {
            var pending = pendingHeaders.Values.Where(header => header.Identity == requested.Identity).Take(2).ToArray();
            if (pending.Length == 1) run = run with { RunId = pending[0].RunId };
        }
        WriteHeader(run);
        var loaded = store.LoadRun(run);
        completedRun = loaded.Summary;
        ordinal = loaded.LastOrdinal;
        foreach (var pending in pendingCombats.Where(record => record.RunId == run.RunId).OrderBy(record => record.Ordinal))
        {
            ordinal = Math.Max(ordinal, pending.Ordinal);
            completedRun = completedRun.Add(pending.Combat.View(run.Players));
        }
        if (pendingCombats.Any(record => record.RunId == run.RunId))
            completedRun = completedRun with { Coverage = completedRun.Coverage.WithFailure("statistics-write-pending") };
        CurrentRun = completedRun;
        Revision++;
    }

    internal static ulong StartCombat(string encounter, string type)
    {
        if (!OnThread()) return 0;
        if (activeEpoch != 0)
        {
            Refresh();
            if (combat != null && (combat.Cards.Count != 0 || combat.Turns != 0))
                Finish(combat with { Result = "interrupted", Coverage = combat.Coverage.WithFailure("interrupted-capture") });
        }
        FlushPending();
        ProfilerNative.CombatDiscard();
        if (sequence == uint.MaxValue) { report("combat sequence exhausted"); return 0; }
        combatRun = run ?? store.OpenRun(new RunRecord(), continued: false);
        if (run == null) { ordinal = 0; WriteHeader(combatRun); }
        bool recordingStarted = !recording || ProfilerNative.RecordingBegin();
        activeEpoch = ProfilerNative.CombatStarted(++sequence, encounter, type, DateTimeOffset.UtcNow.ToUnixTimeSeconds(), combatRun.Players.Count);
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
        bool ended = ProfilerNative.CombatEnded(epoch) == 1;
        if (!ended) ReportFailure("combat-end");
        bool refreshed = Refresh();
        if (combat == null) { activeEpoch = 0; return 0; }
        if (!ended || !refreshed) combat = combat with { Result = "interrupted" };
        Finish(combat);
        return 1;
    }

    private static void Finish(CombatStatistics finished)
    {
        if (ordinal == uint.MaxValue) { ReportFailure("combat-store-sequence-exhausted"); return; }
        var record = new CombatRecord
        {
            GameVersion = store.GameVersion,
            ModVersion = store.ModVersion,
            RunId = combatRun.RunId,
            Ordinal = ++ordinal,
            Combat = finished
        };
        if (!store.SaveCombat(record))
        {
            if (pendingCombats.Count < PendingWriteLimit) pendingCombats.Enqueue(record);
            finished = finished with { Coverage = finished.Coverage.WithFailure("statistics-write-failed") };
        }
        if (recording) store.SaveTrace(record.RunId, record.Ordinal, ProfilerNative.Recording());
        combat = finished;
        CurrentCombat = finished.View(combatRun.Players);
        if (run != null && run.RunId == combatRun.RunId)
        {
            completedRun = completedRun.Add(CurrentCombat);
            CurrentRun = completedRun;
        }
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
            CurrentCombat = snapshot.View(combatRun.Players);
            if (run != null) CurrentRun = completedRun.Add(CurrentCombat);
            nativeRevision = next;
            Revision++;
            return true;
        }
        catch (Exception ex) when (ex is InvalidDataException or IOException or System.Text.Json.JsonException or OverflowException or InvalidOperationException or ArgumentException)
        {
            report($"cannot read attribution snapshot: {ex.Message}");
            combat = combat == null ? null : combat with { Coverage = combat.Coverage.WithFailure("snapshot-read-failed") };
            CurrentCombat = combat?.View(combatRun.Players);
            if (run != null) CurrentRun = CurrentCombat == null
                ? completedRun with { Coverage = completedRun.Coverage.WithFailure("snapshot-read-failed") }
                : completedRun.Add(CurrentCombat);
            Revision++;
            return false;
        }
    }

    internal static void EndRun(int outcome)
    {
        if (!OnThread() || run == null) return;
        if (activeEpoch != 0) EndCombat(activeEpoch);
        var ended = run with
        {
            Outcome = outcome switch { 0 => "victory", 1 => "defeat", 2 => "abandoned", _ => "defeat" },
            EndedAt = DateTimeOffset.UtcNow.ToUnixTimeSeconds()
        };
        WriteHeader(ended);
        FlushPending();
        CurrentRun = completedRun with { Outcome = ended.Outcome, EndedAt = ended.EndedAt };
        run = null;
        Revision++;
    }

    internal static void Suspend()
    {
        if (!OnThread()) return;
        if (run != null) WriteHeader(run with { Outcome = "suspended" });
        FlushPending();
        ProfilerNative.CombatDiscard();
        activeEpoch = 0;
        combat = null;
        combatRun = null;
        run = null;
        completedRun = null;
        CurrentCombat = CurrentRun = null;
        ClearHistory();
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
        Revision++;
    }

    internal static void ReportFailure(string reason)
    {
        if (!OnThread() || activeEpoch == 0) return;
        ProfilerNative.CaptureFailed(reason);
    }

    private static bool OnThread() => store != null && Environment.CurrentManagedThreadId == thread;

    private static void WriteHeader(RunRecord header)
    {
        if (store.SaveRun(header)) pendingHeaders.Remove(header.RunId);
        else if (pendingHeaders.Count < PendingWriteLimit || pendingHeaders.ContainsKey(header.RunId)) pendingHeaders[header.RunId] = header;
    }

    private static void FlushPending()
    {
        foreach (var header in pendingHeaders.Values.ToArray())
            if (store.SaveRun(header)) pendingHeaders.Remove(header.RunId);
        int count = pendingCombats.Count;
        for (int i = 0; i < count; i++)
        {
            var record = pendingCombats.Dequeue();
            if (!store.SaveCombat(record)) pendingCombats.Enqueue(record);
        }
    }

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
            || ProfilerNative.BlockGained(epoch, 5, source, 0) != 1
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
