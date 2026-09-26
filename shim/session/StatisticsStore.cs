using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace SpireProfiler;

// SQLite owns persisted identities, immutable combat payloads, and history.
// Schema-2 JSON is the managed/native record contract. The database schema and
// migrations belong to the Rust store; old JSON archives are imported once.
internal sealed class StatisticsStore : IDisposable
{
    internal const int SchemaVersion = 2;
    private NativeStatisticsStore native;
    private readonly Action<string> report;
    private readonly string traceDirectory;
    internal string GameVersion { get; }
    internal string ModVersion { get; }

    private sealed record StoredRun
    {
        public RunRecord Run { get; init; }
        public IReadOnlyList<CombatRecord> Combats { get; init; } = Array.Empty<CombatRecord>();
        public IReadOnlyList<string> Reasons { get; init; } = Array.Empty<string>();
        public uint LastOrdinal { get; init; }
    }

    internal StatisticsStore(string dataDirectory, string gameVersion, string modVersion, Action<string> report)
    {
        GameVersion = gameVersion;
        ModVersion = modVersion;
        this.report = report;
        traceDirectory = Path.Combine(dataDirectory, "statistics-v3", "traces");
        try
        {
            string directory = Path.Combine(dataDirectory, "statistics-v3");
            Directory.CreateDirectory(directory);
            native = new NativeStatisticsStore(Path.Combine(directory, "statistics.sqlite3"));
            var (statusRead, imported) = RequestResult(new { op = "import_status" }, false);
            if (!statusRead)
            {
                Dispose();
                return;
            }
            if (!imported && !Request(new LegacyStatisticsImport(dataDirectory, report).ReadArchive(), false)) Dispose();
        }
        catch (Exception error) when (RecordFailure(error))
        {
            native?.Dispose();
            native = null;
            report($"cannot open statistics: {error.Message}");
        }
    }

    public void Dispose()
    {
        native?.Dispose();
        native = null;
    }

    internal RunRecord OpenRun(RunRecord requested, bool continued)
    {
        var run = requested with
        {
            GameVersion = GameVersion,
            ModVersion = ModVersion,
            Outcome = "active",
            EndedAt = 0,
            StartedAt = Math.Max(0, requested.StartedAt),
            Profile = Math.Max(-1, requested.Profile)
        };
        var (success, opened) = RequestResult<RunRecord>(new { op = "open_run", run, continued }, null);
        if (!success)
        {
            Dispose();
            return run with { RunId = "0", PreservedRunIds = Array.Empty<string>() };
        }
        if (opened == null) return null;
        return opened with
        {
            Players = Array.AsReadOnly(opened.Players.ToArray()),
            PreservedRunIds = Array.AsReadOnly(opened.PreservedRunIds.ToArray())
        };
    }

    internal uint? MaxCombatId()
    {
        var maximum = Request<uint?>(new { op = "max_combat_id" }, null);
        if (maximum != null) return maximum;
        Dispose();
        return 0;
    }
    internal bool SaveRun(RunRecord run) => Request(new { op = "save_run", run }, false);
    internal bool SaveCombat(CombatRecord record) => Request(new { op = "save_combat", record }, false);
    internal void SaveTrace(string runId, uint ordinal, string recording)
    {
        string temporary = null;
        try
        {
            if (!uint.TryParse(runId, out uint id) || id.ToString(System.Globalization.CultureInfo.InvariantCulture) != runId || ordinal == 0)
                throw new InvalidDataException("Invalid recording identity");
            using var document = JsonDocument.Parse(recording);
            string directory = Path.Combine(traceDirectory, runId);
            Directory.CreateDirectory(directory);
            string path = Path.Combine(directory, ordinal.ToString(System.Globalization.CultureInfo.InvariantCulture) + ".trace.json");
            temporary = path + ".tmp";
            File.WriteAllText(temporary, document.RootElement.GetRawText(), new UTF8Encoding(false));
            File.Move(temporary, path, overwrite: false);
        }
        catch (Exception error) when (RecordFailure(error))
        {
            if (temporary != null)
                try { File.Delete(temporary); } catch (Exception cleanup) when (RecordFailure(cleanup)) { }
            report($"cannot persist observation recording: {error.Message}");
        }
    }

    internal (SummaryView Summary, uint LastOrdinal) LoadRun(RunRecord run, bool historyView = false)
    {
        var stored = Request<StoredRun>(new { op = "load_run", run_id = run.RunId }, null);
        var summary = run.EmptySummary();
        if (stored == null) return (summary with { Coverage = summary.Coverage.WithFailure("statistics-read-failed") }, 0);
        foreach (var record in stored.Combats)
            summary = historyView ? summary.AddHistory(record.Combat) : summary.Add(record.Combat.View(run.Players));
        foreach (string reason in stored.Reasons) summary = summary with { Coverage = summary.Coverage.WithFailure(reason) };
        return (summary, stored.LastOrdinal);
    }

    internal SummaryView Select(RunIdentity identity)
    {
        if (identity == null) return null;
        var stored = Request<StoredRun>(new { op = "select", identity.Profile, identity.Seed, identity.StartedAt }, null);
        if (stored?.Run == null) return null;
        var run = stored.Run with { Players = Array.AsReadOnly(stored.Run.Players.ToArray()) };
        var summary = run.EmptySummary();
        foreach (var record in stored.Combats) summary = summary.AddHistory(record.Combat);
        foreach (string reason in stored.Reasons) summary = summary with { Coverage = summary.Coverage.WithFailure(reason) };
        return summary;
    }

    private T Request<T>(object command, T fallback)
        => RequestResult(command, fallback).Value;

    private (bool Success, T Value) RequestResult<T>(object command, T fallback)
    {
        if (native == null) return (false, fallback);
        try
        {
            using var response = native.Execute(command);
            var root = response.RootElement;
            if (!root.GetProperty("ok").GetBoolean())
            {
                report($"statistics operation failed: {root.GetProperty("error").GetString()}");
                return (false, fallback);
            }
            var value = root.GetProperty("value");
            return (true, value.ValueKind == JsonValueKind.Null ? fallback : value.Deserialize<T>(StatisticsJson.Options));
        }
        catch (Exception error) when (RecordFailure(error))
        {
            report($"cannot access statistics: {error.Message}");
            return (false, fallback);
        }
    }

    private static bool RecordFailure(Exception error)
        => error is InvalidDataException or IOException or UnauthorizedAccessException or JsonException or OverflowException
            or ArgumentException or InvalidOperationException or KeyNotFoundException;
}
