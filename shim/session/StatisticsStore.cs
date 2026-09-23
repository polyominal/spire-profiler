using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace SpireProfiler;

// Schema 1 lives at statistics-v1/runs/<GUID>/: run.json is replaceable metadata,
// <ordinal:D8>.json is an immutable CombatRecord, and optional *.trace.json files
// record native observations. JSON names use snake_case, zeros stay explicit,
// and unknown fields are ignored. Both envelopes require schema_version=1.
// Combat policy_version describes attribution, independently of storage shape.
// Identity is (profile, seed, original started_at in epoch seconds); a missing
// component or ambiguous match cannot join runs. Coverage defaults to unknown.
// History reads headers first, then opens only the selected combat directory.
internal sealed class StatisticsStore
{
    internal const int SchemaVersion = 1;
    private const int MaxDocumentBytes = 64 * 1024 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private readonly string dataDirectory;
    private readonly string runsDirectory;
    private readonly Action<string> report;
    private List<RunRecord> headers;
    internal string GameVersion { get; }
    internal string ModVersion { get; }

    internal StatisticsStore(string dataDirectory, string gameVersion, string modVersion, Action<string> report)
    {
        this.dataDirectory = dataDirectory;
        runsDirectory = Path.Combine(dataDirectory, "statistics-v1", "runs");
        GameVersion = gameVersion;
        ModVersion = modVersion;
        this.report = report;
    }

    internal RunRecord OpenRun(RunRecord requested, bool continued)
    {
        var matches = continued && requested.Identity != null ? FindRuns(requested.Identity) : Array.Empty<RunRecord>();
        var prior = matches.Length == 1 ? matches[0] : null;
        var run = requested with
        {
            RunId = prior?.RunId ?? Guid.NewGuid().ToString("N", CultureInfo.InvariantCulture),
            GameVersion = GameVersion,
            ModVersion = ModVersion,
            Outcome = "active",
            EndedAt = 0
        };
        return run;
    }

    internal bool SaveRun(RunRecord run)
    {
        bool written = Write(Path.Combine(RunDirectory(run.RunId), "run.json"), run, overwrite: true);
        if (written) headers = null;
        return written;
    }

    internal bool SaveCombat(CombatRecord record)
        => Write(Path.Combine(RunDirectory(record.RunId), $"{record.Ordinal:D8}.json"), record, overwrite: false);

    internal void SaveTrace(string runId, uint ordinal, string recording)
    {
        try
        {
            using var document = JsonDocument.Parse(recording);
            Write(Path.Combine(RunDirectory(runId), $"{ordinal:D8}.trace.json"), document.RootElement, overwrite: false);
        }
        catch (JsonException ex) { report($"cannot parse observation recording: {ex.Message}"); }
    }

    internal (SummaryView Summary, uint LastOrdinal) LoadRun(RunRecord run)
    {
        var summary = run.EmptySummary();
        uint highest = 0;
        try
        {
            var directory = RunDirectory(run.RunId);
            if (!Directory.Exists(directory)) return (summary, highest);
            foreach (var path in Directory.EnumerateFiles(directory, "*.json").OrderBy(path => path, StringComparer.Ordinal))
            {
                if (!uint.TryParse(Path.GetFileNameWithoutExtension(path), NumberStyles.None, CultureInfo.InvariantCulture, out uint ordinal)
                    || ordinal == 0) continue;
                highest = Math.Max(highest, ordinal);
                try
                {
                    var record = StatisticsJson.ParseCombat(Read(path), run.RunId, ordinal);
                    var combat = record.Combat;
                    summary = summary.Add(combat.View(run.Players));
                }
                catch (Exception ex) when (RecordFailure(ex))
                {
                    report($"cannot read combat '{path}': {ex.Message}");
                    summary = summary with { Coverage = summary.Coverage.WithFailure("unreadable-combat") };
                }
            }
        }
        catch (Exception ex) when (RecordFailure(ex))
        {
            report($"cannot list run '{run.RunId}': {ex.Message}");
            summary = summary with { Coverage = summary.Coverage.WithFailure("unreadable-run") };
        }
        return (summary, highest);
    }

    internal SummaryView Select(RunIdentity identity)
    {
        var matches = FindRuns(identity);
        return matches.Length switch { 0 => LoadLegacy(identity), 1 => LoadRun(matches[0]).Summary, _ => null };
    }

    private RunRecord[] FindRuns(RunIdentity identity)
    {
        if (identity == null) return Array.Empty<RunRecord>();
        if (headers == null)
        {
            var loaded = new List<RunRecord>();
            try
            {
                if (Directory.Exists(runsDirectory))
                    foreach (var directory in Directory.EnumerateDirectories(runsDirectory).OrderBy(path => path, StringComparer.Ordinal))
                    {
                        string id = Path.GetFileName(directory);
                        if (!Guid.TryParseExact(id, "N", out _)) continue;
                        try { loaded.Add(StatisticsJson.ParseRun(Read(Path.Combine(directory, "run.json")), id)); }
                        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot read run '{id}': {ex.Message}"); }
                    }
                headers = loaded;
            }
            catch (Exception ex) when (RecordFailure(ex)) { report($"cannot list statistics store: {ex.Message}"); return Array.Empty<RunRecord>(); }
        }
        var matches = headers.Where(run => run.Identity == identity).Take(2).ToArray();
        if (matches.Length > 1) report("multiple profiler runs match the selected game run");
        return matches;
    }

    private string RunDirectory(string id)
    {
        if (!Guid.TryParseExact(id, "N", out _)) throw new InvalidDataException("Invalid store run identity");
        return Path.Combine(runsDirectory, id);
    }

    private bool Write<T>(string path, T value, bool overwrite)
    {
        string temporary = path + ".tmp";
        try
        {
            var bytes = JsonSerializer.SerializeToUtf8Bytes(value, StatisticsJson.Options);
            if (bytes.Length > MaxDocumentBytes) throw new InvalidDataException("Statistics document exceeds size limit");
            Directory.CreateDirectory(Path.GetDirectoryName(path));
            // A retried immutable write may already have succeeded before an
            // interrupted caller observed its return value.
            if (!overwrite && File.Exists(path))
            {
                if (Read(path) == Utf8.GetString(bytes)) return true;
                throw new InvalidDataException("An immutable combat record already exists with different contents");
            }
            using (var file = new FileStream(temporary, FileMode.Create, FileAccess.Write, FileShare.None))
                file.Write(bytes);
            File.Move(temporary, path, overwrite);
            return true;
        }
        catch (Exception ex) when (RecordFailure(ex))
        {
            report($"cannot write statistics '{path}': {ex.Message}");
            try { File.Delete(temporary); }
            catch (Exception cleanup) when (RecordFailure(cleanup)) { }
            return false;
        }
    }

    private static string Read(string path)
    {
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (file.Length > MaxDocumentBytes) throw new InvalidDataException("Statistics document exceeds size limit");
        var bytes = new byte[checked((int)file.Length)];
        file.ReadExactly(bytes);
        if (file.ReadByte() != -1) throw new InvalidDataException("Statistics document changed while reading");
        return Utf8.GetString(bytes);
    }

    private static bool RecordFailure(Exception error)
        => error is InvalidDataException or IOException or UnauthorizedAccessException or JsonException or OverflowException
            or ArgumentException or InvalidOperationException;

    private SummaryView LoadLegacy(RunIdentity identity)
    {
        string path = Path.Combine(dataDirectory, "runs.jsonl");
        if (!File.Exists(path)) return null;
        try
        {
            LegacyRun selected = null;
            foreach (string line in Read(path).Split('\n', StringSplitOptions.RemoveEmptyEntries))
            {
                try
                {
                    var candidate = JsonSerializer.Deserialize<LegacyRun>(line, StatisticsJson.Options);
                    if (candidate == null || candidate.RunId == 0
                        || RunIdentity.Parse(candidate.Profile, candidate.Seed, candidate.StartedAt) != identity) continue;
                    if (selected != null && selected.RunId != candidate.RunId)
                        throw new InvalidDataException("Multiple legacy runs share one identity");
                    selected = candidate;
                }
                catch (JsonException ex) { report($"cannot parse legacy run: {ex.Message}"); }
            }
            if (selected == null) return null;
            if (selected.Players is { Length: 0 } && !string.IsNullOrEmpty(selected.Character))
                selected.Players = selected.Character.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                    .Take(4).Select((character, slot) => new PlayerSummary(slot, character)).ToArray();
            if (selected.Players == null || selected.Players.Length > 4 || selected.Players.Any(player => player == null
                || player.Slot is < 0 or > 3 || string.IsNullOrEmpty(player.Character))
                || selected.Players.Select(player => player.Slot).Distinct().Count() != selected.Players.Length)
                throw new InvalidDataException("Invalid legacy run roster");
            var run = new RunRecord
            {
                Profile = identity.Profile,
                Seed = identity.Seed,
                StartedAt = identity.StartedAt,
                EndedAt = selected.EndedAt,
                Character = selected.Character,
                Ascension = selected.Ascension,
                GameMode = selected.GameMode,
                Outcome = selected.Outcome,
                Players = Array.AsReadOnly(selected.Players ?? Array.Empty<PlayerSummary>())
            };
            var summary = run.EmptySummary() with { Coverage = CoverageSummary.Unknown };
            string directory = Path.Combine(dataDirectory, "runs", selected.RunId.ToString(CultureInfo.InvariantCulture));
            if (!Directory.Exists(directory)) return summary;
            foreach (var combatPath in Directory.EnumerateFiles(directory, "*.json").OrderBy(value => value, StringComparer.Ordinal))
            {
                try
                {
                    var combat = JsonSerializer.Deserialize<LegacyCombat>(Read(combatPath), StatisticsJson.Options);
                    if (combat?.Run == null || combat.Run.Seq != selected.RunId
                        || RunIdentity.Parse(combat.Run.Profile, combat.Run.Seed, combat.Run.StartedAt) != identity
                        || !uint.TryParse(Path.GetFileNameWithoutExtension(combatPath), out uint id) || combat.CombatId != id
                        || combat.DamageReceived < 0)
                        throw new InvalidDataException("Legacy combat identity differs from its run");
                    StatisticsJson.CheckRows(combat.Cards);
                    summary = summary.Add(new SummaryView
                    {
                        Cards = Array.AsReadOnly(combat.Cards),
                        Turns = combat.Turns,
                        Plays = checked((uint)combat.Cards.Sum(row => (long)row.Plays)),
                        Combats = 1,
                        DamageReceived = combat.DamageReceived,
                        Coverage = CoverageSummary.Unknown
                    });
                }
                catch (Exception ex) when (RecordFailure(ex))
                {
                    report($"cannot read legacy combat '{combatPath}': {ex.Message}");
                    summary = summary with { Coverage = summary.Coverage.WithFailure("unreadable-legacy-combat") };
                }
            }
            return summary;
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot read legacy statistics: {ex.Message}"); return null; }
    }

    private sealed class LegacyRun
    {
        public uint RunId { get; set; }
        public int Profile { get; set; } = -1;
        public string Seed { get; set; } = "";
        public long StartedAt { get; set; }
        public long EndedAt { get; set; }
        public string Character { get; set; } = "";
        public int Ascension { get; set; }
        public string GameMode { get; set; } = "";
        public string Outcome { get; set; } = "";
        public PlayerSummary[] Players { get; set; } = Array.Empty<PlayerSummary>();
    }

    private sealed class LegacyCombat
    {
        public uint CombatId { get; set; }
        public uint Turns { get; set; }
        public long DamageReceived { get; set; }
        public StatRow[] Cards { get; set; } = Array.Empty<StatRow>();
        public LegacyIdentity Run { get; set; }
    }

    private sealed class LegacyIdentity
    {
        public uint Seq { get; set; }
        public int Profile { get; set; } = -1;
        public string Seed { get; set; } = "";
        public long StartedAt { get; set; }
    }
}
