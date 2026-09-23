using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace SpireProfiler;

// Schema-1 records live at statistics-v1/runs/<numeric-run>/<combat>.json.
// Each combat carries the run metadata captured at its start. Finalized headers
// append to statistics-v1/runs.jsonl; the first matching header supplies history.
// Numeric IDs reserve existing directories and files, including unreadable ones.
// The unversioned store and GUID directories are read-only inputs.
internal sealed class StatisticsStore
{
    internal const int SchemaVersion = 1;
    private const int MaxDocumentBytes = 64 * 1024 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private readonly string dataDirectory;
    private readonly string versionDirectory;
    private readonly string runsDirectory;
    private readonly Action<string> report;
    private StoreSnapshot history;
    internal string GameVersion { get; }
    internal string ModVersion { get; }

    private sealed record StoreSnapshot(IReadOnlyList<RunRecord> Headers, IReadOnlyList<CombatRecord> Combats);

    internal StatisticsStore(string dataDirectory, string gameVersion, string modVersion, Action<string> report)
    {
        this.dataDirectory = dataDirectory;
        versionDirectory = Path.Combine(dataDirectory, "statistics-v1");
        runsDirectory = Path.Combine(versionDirectory, "runs");
        GameVersion = gameVersion;
        ModVersion = modVersion;
        this.report = report;
    }

    internal RunRecord OpenRun(RunRecord requested, bool continued)
    {
        try
        {
            var snapshot = ReadStore(continued, strictHeaders: true);
            var prior = continued ? Match(snapshot, requested.Identity) : null;
            string id = prior?.RunId;
            string priorId = prior?.PriorRunId;
            if (!uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint number) || number == 0)
            {
                priorId = id;
                uint maximum = 0;
                foreach (var header in snapshot.Headers)
                    if (uint.TryParse(header.RunId, NumberStyles.None, CultureInfo.InvariantCulture, out uint recorded)) maximum = Math.Max(maximum, recorded);
                foreach (string root in new[] { Path.Combine(dataDirectory, "runs"), runsDirectory })
                    foreach (string directory in Directories(root))
                        if (uint.TryParse(Path.GetFileName(directory), NumberStyles.None, CultureInfo.InvariantCulture, out uint reserved)) maximum = Math.Max(maximum, reserved);
                if (maximum == uint.MaxValue) { report("run IDs exhausted"); return null; }
                id = (maximum + 1).ToString(CultureInfo.InvariantCulture);
            }
            return requested with
            {
                RunId = id,
                PriorRunId = priorId,
                LegacyRunId = prior?.LegacyRunId,
                GameVersion = GameVersion,
                ModVersion = ModVersion,
                Outcome = "active",
                EndedAt = 0,
                StartedAt = Math.Max(0, requested.StartedAt),
                Profile = Math.Max(-1, requested.Profile)
            };
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot allocate run identity: {ex.Message}"); return null; }
    }

    internal uint? MaxCombatId()
    {
        try
        {
            uint maximum = 0;
            foreach (string root in new[] { Path.Combine(dataDirectory, "runs"), runsDirectory })
                foreach (string directory in Directories(root))
                {
                    string runId = Path.GetFileName(directory);
                    bool numeric = uint.TryParse(runId, NumberStyles.None, CultureInfo.InvariantCulture, out uint number);
                    if (!numeric && (root != runsDirectory || !Guid.TryParseExact(runId, "N", out _))) continue;
                    string source = numeric ? Path.Combine(root, number.ToString(CultureInfo.InvariantCulture)) : directory;
                    foreach (string path in Files(source))
                    {
                        if (!uint.TryParse(Path.GetFileNameWithoutExtension(path), NumberStyles.None, CultureInfo.InvariantCulture, out uint id)) continue;
                        if (numeric) maximum = Math.Max(maximum, id);
                        else
                        {
                            try { maximum = Math.Max(maximum, StatisticsJson.ParseCombat(Read(path), runId, id).Combat.CombatId); }
                            catch (Exception ex) when (RecordFailure(ex)) { report($"cannot read preserved combat identity: {ex.Message}"); }
                        }
                    }
                }
            return maximum;
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot scan combat IDs: {ex.Message}"); return null; }
    }

    internal bool SaveRun(RunRecord run)
    {
        if (run.Outcome is not ("victory" or "defeat" or "abandoned")) return false;
        try
        {
            if (!ReadStore(strictHeaders: true).Combats.Any(combat => MatchesStorage(combat.RunId, run))) return false;
            string path = Path.Combine(versionDirectory, "runs.jsonl");
            string prior = File.Exists(path) ? Read(path) : "";
            if (prior.Length != 0 && !prior.EndsWith('\n')) prior += "\n";
            bool written = WriteText(path, prior + JsonSerializer.Serialize(run, StatisticsJson.Options) + "\n", overwrite: true);
            if (written) history = null;
            return written;
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot finalize run: {ex.Message}"); return false; }
    }

    internal bool SaveCombat(CombatRecord record)
    {
        bool written = WriteText(Path.Combine(RunDirectory(record.RunId), $"{record.Ordinal}.json"), JsonSerializer.Serialize(record, StatisticsJson.Options), overwrite: false);
        if (written) history = null;
        return written;
    }

    internal void SaveTrace(string runId, uint ordinal, string recording)
    {
        try
        {
            using var document = JsonDocument.Parse(recording);
            WriteText(Path.Combine(RunDirectory(runId), $"{ordinal}.trace.json"), document.RootElement.GetRawText(), overwrite: false);
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot persist observation recording: {ex.Message}"); }
    }

    internal (SummaryView Summary, uint LastOrdinal) LoadRun(RunRecord run, bool historyView = false)
    {
        var summary = run.EmptySummary();
        uint highest = 0;
        try
        {
            var snapshot = ReadStore();
            foreach (var record in snapshot.Combats)
            {
                if (!Belongs(record, run)) continue;
                highest = Math.Max(highest, record.Ordinal);
                summary = historyView ? summary.AddHistory(record.Combat) : summary.Add(record.Combat.View(run.Players));
            }
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot load run: {ex.Message}"); }
        return (summary, highest);
    }

    internal SummaryView Select(RunIdentity identity)
    {
        if (identity == null) return null;
        try
        {
            history ??= ReadStore();
            var run = Match(history, identity);
            if (run == null) return null;
            var summary = run.EmptySummary();
            foreach (var combat in history.Combats)
                if (Belongs(combat, run)) summary = summary.AddHistory(combat.Combat);
            return summary;
        }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot select statistics: {ex.Message}"); return null; }
    }

    private RunRecord Match(StoreSnapshot snapshot, RunIdentity identity)
    {
        if (identity == null) return null;
        var candidates = snapshot.Headers.Where(header => header.Identity == identity).OrderBy(header => header.PriorRunId != null)
            .Concat(snapshot.Combats.Reverse().Select(combat => combat.Run).Where(run => run?.Identity == identity)).ToArray();
        if (candidates.Length == 0) return null;
        var links = candidates.SelectMany(run => new[]
        {
            (Prior: run.PriorRunId, Current: run.RunId),
            (Prior: run.LegacyRunId?.ToString(CultureInfo.InvariantCulture), Current: run.RunId)
        }).Where(link => link.Prior != null && link.Prior != link.Current);
        var aliases = links.GroupBy(link => link.Prior, StringComparer.Ordinal).ToDictionary(group => group.Key, group => group.First().Current, StringComparer.Ordinal);
        string Canonical(string id)
        {
            int remaining = aliases.Count;
            while (aliases.TryGetValue(id, out string mapped))
            {
                if (remaining-- == 0) return null;
                id = mapped;
            }
            return id;
        }
        string canonical = Canonical(candidates[0].RunId);
        if (canonical == null || candidates.Any(run => Canonical(run.RunId) != canonical))
        {
            report("multiple profiler runs match the selected game run");
            return null;
        }
        var selected = candidates[0];
        var owner = candidates.First(run => run.RunId == canonical);
        return selected with { RunId = canonical, PriorRunId = owner.PriorRunId, LegacyRunId = owner.LegacyRunId };
    }

    private static bool MatchesStorage(string id, RunRecord run)
        => id == run.RunId || id == run.PriorRunId || id == run.LegacyRunId?.ToString(CultureInfo.InvariantCulture);

    private static bool Belongs(CombatRecord combat, RunRecord run)
        => combat.Run != null && MatchesStorage(combat.RunId, run)
            && combat.Run.Identity != null && combat.Run.Identity == run.Identity;

    private StoreSnapshot ReadStore(bool includeCombats = true, bool strictHeaders = false)
    {
        var headers = new List<RunRecord>();
        var combats = new List<CombatRecord>();
        foreach (bool versioned in new[] { false, true })
        {
            string root = versioned ? versionDirectory : dataDirectory;
            string headerPath = Path.Combine(root, "runs.jsonl");
            string[] lines = Array.Empty<string>();
            try
            {
                if (File.Exists(headerPath) || Directory.Exists(headerPath)) lines = Read(headerPath).Split('\n', StringSplitOptions.RemoveEmptyEntries);
            }
            catch (Exception ex) when (RecordFailure(ex))
            {
                if (strictHeaders) throw;
                report($"cannot read run headers: {ex.Message}");
            }
            foreach (string line in lines)
            {
                if (string.IsNullOrWhiteSpace(line)) continue;
                try
                {
                    RunRecord header;
                    if (versioned)
                    {
                        var node = JsonSerializer.Deserialize<RunRecord>(line, StatisticsJson.Options) ?? throw new InvalidDataException("Missing header");
                        header = StatisticsJson.ParseRun(line, node.RunId);
                    }
                    else header = LegacyHeader(JsonSerializer.Deserialize<LegacyRun>(line, StatisticsJson.Options) ?? throw new InvalidDataException("Missing legacy header"));
                    if (header.RunId != "0" && header.Outcome is "victory" or "defeat" or "abandoned") headers.Add(header);
                }
                catch (Exception ex) when (RecordFailure(ex)) { report($"cannot parse run header: {ex.Message}"); }
            }
            string[] directories;
            try { directories = Directories(Path.Combine(root, "runs")); }
            catch (Exception ex) when (RecordFailure(ex)) { report($"cannot list history: {ex.Message}"); directories = Array.Empty<string>(); }
            foreach (string directory in directories)
            {
                string id = Path.GetFileName(directory);
                bool numeric = uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint numericId);
                string sourceDirectory = numeric ? Path.Combine(root, "runs", numericId.ToString(CultureInfo.InvariantCulture)) : directory;
                if (numeric) id = numericId.ToString(CultureInfo.InvariantCulture);
                RunRecord guidHeader = null;
                if (!numeric)
                {
                    if (!versioned || !Guid.TryParseExact(id, "N", out _)) continue;
                    try { guidHeader = StatisticsJson.ParseRun(Read(Path.Combine(directory, "run.json")), id); }
                    catch (Exception ex) when (RecordFailure(ex)) { report($"cannot read preserved run: {ex.Message}"); continue; }
                    if (guidHeader.Outcome is "victory" or "defeat" or "abandoned") headers.Add(guidHeader);
                }
                if (!includeCombats) continue;
                string[] paths;
                try { paths = Files(sourceDirectory); }
                catch (Exception ex) when (RecordFailure(ex)) { report($"cannot list combats: {ex.Message}"); continue; }
                foreach (string path in paths)
                {
                    if (!uint.TryParse(Path.GetFileNameWithoutExtension(path), NumberStyles.None, CultureInfo.InvariantCulture, out uint ordinal)) continue;
                    try
                    {
                        string sourcePath = numeric ? Path.Combine(sourceDirectory, $"{ordinal}.json") : path;
                        CombatRecord record;
                        if (versioned)
                        {
                            record = StatisticsJson.ParseCombat(Read(sourcePath), id, ordinal);
                            if (guidHeader != null) record = record with { Run = guidHeader with { Outcome = "", EndedAt = 0, Players = Array.Empty<PlayerSummary>() } };
                        }
                        else record = LegacyCombatRecord(Read(sourcePath), id, ordinal);
                        if (numeric && (id == "0" ? record.Run != null : record.Run?.RunId != id)) throw new InvalidDataException("Combat run identity differs from directory");
                        if (record.Run != null) record = record with { Run = record.Run with { Outcome = "", EndedAt = 0, Players = Array.Empty<PlayerSummary>() } };
                        combats.Add(record);
                    }
                    catch (Exception ex) when (RecordFailure(ex)) { report($"cannot parse combat '{path}': {ex.Message}"); }
                }
            }
        }
        combats.Sort((left, right) => left.Combat.CombatId.CompareTo(right.Combat.CombatId));
        return new(headers, combats);
    }

    private static RunRecord LegacyHeader(LegacyRun run)
    {
        if (run.Seed == null || run.Character == null || run.GameMode == null || run.Players == null
            || run.Outcome is not ("victory" or "defeat" or "abandoned")
            || run.Players.Any(player => player == null || player.Character == null || player.Slot is < 0 or > 255)) throw new InvalidDataException("Invalid legacy header");
        return new()
        {
            RunId = run.RunId.ToString(CultureInfo.InvariantCulture),
            Profile = run.Profile,
            Seed = run.Seed,
            StartedAt = run.StartedAt,
            EndedAt = run.EndedAt,
            Character = run.Character,
            Ascension = run.Ascension,
            GameMode = run.GameMode,
            Outcome = run.Outcome,
            Players = Array.AsReadOnly(run.Players)
        };
    }

    private static CombatRecord LegacyCombatRecord(string json, string runId, uint ordinal)
    {
        var combat = JsonSerializer.Deserialize<LegacyCombat>(json, StatisticsJson.Options) ?? throw new InvalidDataException("Missing legacy combat");
        if (combat.CombatId != ordinal || combat.Cards == null || combat.EncounterId == null
            || combat.Result is not ("completed" or "defeat" or "interrupted")) throw new InvalidDataException("Invalid legacy combat");
        var rows = combat.Cards.Select(row => row == null || row.Id == null || row.Kind is < 0 or > 255 || row.Player is < 0 or > 255
            ? throw new InvalidDataException("Invalid legacy row") : row with { Kind = Math.Min(row.Kind, 5), Player = Math.Min(row.Player, 4) }).ToArray();
        RunRecord run = combat.Run == null ? null : new()
        {
            RunId = combat.Run.Seq.ToString(CultureInfo.InvariantCulture),
            Character = combat.Run.Character,
            Ascension = combat.Run.Ascension,
            GameMode = combat.Run.GameMode,
            Profile = combat.Run.Profile,
            Seed = combat.Run.Seed,
            StartedAt = combat.Run.StartedAt,
            Outcome = ""
        };
        if (run != null && (run.Character == null || run.GameMode == null || run.Seed == null)) throw new InvalidDataException("Invalid legacy run identity");
        return new()
        {
            RunId = runId,
            Ordinal = ordinal,
            Run = run,
            Combat = new()
            {
                CombatId = ordinal,
                StartedAt = combat.StartedAt,
                EncounterId = combat.EncounterId,
                Result = combat.Result,
                Turns = combat.Turns,
                DamageReceived = combat.DamageReceived,
                Cards = Array.AsReadOnly(rows),
                Coverage = CoverageSummary.Unknown
            }
        };
    }

    private string RunDirectory(string id)
    {
        if (!uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out _)) throw new InvalidDataException("New records require a numeric run identity");
        return Path.Combine(runsDirectory, id);
    }

    private bool WriteText(string path, string text, bool overwrite)
    {
        string temporary = path + ".tmp";
        try
        {
            byte[] bytes = Utf8.GetBytes(text);
            if (bytes.Length > MaxDocumentBytes) throw new InvalidDataException("Statistics document exceeds size limit");
            Directory.CreateDirectory(Path.GetDirectoryName(path));
            if (!overwrite && File.Exists(path)) throw new IOException("Combat record already exists");
            File.WriteAllBytes(temporary, bytes);
            File.Move(temporary, path, overwrite);
            return true;
        }
        catch (Exception ex) when (RecordFailure(ex))
        {
            try { File.Delete(temporary); } catch (Exception cleanup) when (RecordFailure(cleanup)) { }
            report($"cannot write statistics '{path}': {ex.Message}"); return false;
        }
    }

    private static string Read(string path)
    {
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (file.Length > MaxDocumentBytes) throw new InvalidDataException("Statistics document exceeds size limit");
        byte[] bytes = new byte[checked((int)file.Length)];
        file.ReadExactly(bytes);
        return Utf8.GetString(bytes);
    }
    private static string[] Directories(string root) => Directory.Exists(root) || File.Exists(root) ? Directory.GetFileSystemEntries(root) : Array.Empty<string>();
    private static string[] Files(string root) => Directory.Exists(root) || File.Exists(root) ? Directory.GetFiles(root, "*.json") : Array.Empty<string>();
    private static bool RecordFailure(Exception error) => error is InvalidDataException or IOException or UnauthorizedAccessException or JsonException or OverflowException or ArgumentException or InvalidOperationException;

    private sealed class LegacyRun
    {
        public uint RunId { get; set; }
        public int Profile { get; set; } = -1;
        public string Seed { get; set; } = "";
        public long StartedAt { get; set; }
        public long EndedAt { get; set; }
        public string Character { get; set; } = "";
        public int Ascension { get; set; } = -1;
        public string GameMode { get; set; } = "";
        public string Outcome { get; set; } = "defeat";
        public PlayerSummary[] Players { get; set; } = Array.Empty<PlayerSummary>();
    }
    private sealed class LegacyCombat
    {
        public uint CombatId { get; set; }
        public long StartedAt { get; set; }
        public string EncounterId { get; set; } = "";
        public string Result { get; set; } = "completed";
        public uint Turns { get; set; }
        public long DamageReceived { get; set; }
        public StatRow[] Cards { get; set; } = Array.Empty<StatRow>();
        public LegacyIdentity Run { get; set; }
    }
    private sealed class LegacyIdentity
    {
        public uint Seq { get; set; }
        public string Character { get; set; } = "";
        public int Ascension { get; set; } = -1;
        public string GameMode { get; set; } = "";
        public int Profile { get; set; } = -1;
        public string Seed { get; set; } = "";
        public long StartedAt { get; set; }
    }
}
