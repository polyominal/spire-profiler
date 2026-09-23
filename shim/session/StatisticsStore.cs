using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace SpireProfiler;

// Schema-2 records live at statistics-v2/runs/<numeric-run>/<combat>.json.
// Each combat carries the run metadata captured at its start. Finalized headers
// append to statistics-v2/runs.jsonl; the first matching header supplies history.
// Numeric IDs reserve existing directories and files, including unreadable ones.
// Unversioned and schema-1 stores are read-only inputs. Preserved run IDs
// flatten imported directory aliases; new records use numeric identities only.
internal sealed class StatisticsStore
{
    internal const int SchemaVersion = 2;
    private const int MaxDocumentBytes = 64 * 1024 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private readonly string dataDirectory;
    private readonly string preservedDirectory;
    private readonly string versionDirectory;
    private readonly string runsDirectory;
    private readonly Action<string> report;
    private StoreSnapshot history;
    internal string GameVersion { get; }
    internal string ModVersion { get; }

    private sealed record StoreSnapshot(IReadOnlyList<RunRecord> Headers, IReadOnlyList<CombatRecord> Combats, IReadOnlySet<RunIdentity> InvalidImports, uint MaximumRunId);

    internal StatisticsStore(string dataDirectory, string gameVersion, string modVersion, Action<string> report)
    {
        this.dataDirectory = dataDirectory;
        preservedDirectory = Path.Combine(dataDirectory, "statistics-v1");
        versionDirectory = Path.Combine(dataDirectory, "statistics-v2");
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
            var preservedIds = prior?.PreservedRunIds ?? Array.Empty<string>();
            if (!uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint number) || number == 0)
            {
                if (id != null) preservedIds = Array.AsReadOnly(preservedIds.Append(id).Distinct(StringComparer.Ordinal).ToArray());
                uint maximum = snapshot.MaximumRunId;
                foreach (string root in new[] { Path.Combine(dataDirectory, "runs"), Path.Combine(preservedDirectory, "runs"), runsDirectory })
                    foreach (string directory in Directories(root))
                        if (uint.TryParse(Path.GetFileName(directory), NumberStyles.None, CultureInfo.InvariantCulture, out uint reserved)) maximum = Math.Max(maximum, reserved);
                if (maximum == uint.MaxValue) { report("run IDs exhausted"); return null; }
                id = (maximum + 1).ToString(CultureInfo.InvariantCulture);
            }
            else if (id != number.ToString(CultureInfo.InvariantCulture))
            {
                preservedIds = Array.AsReadOnly(preservedIds.Append(id).Distinct(StringComparer.Ordinal).ToArray());
                id = number.ToString(CultureInfo.InvariantCulture);
            }
            preservedIds = Array.AsReadOnly(preservedIds.Where(priorId => priorId != id).ToArray());
            return requested with
            {
                RunId = id,
                PreservedRunIds = preservedIds,
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
            foreach (string root in new[] { Path.Combine(dataDirectory, "runs"), Path.Combine(preservedDirectory, "runs"), runsDirectory })
                foreach (string directory in Directories(root))
                {
                    string runId = Path.GetFileName(directory);
                    bool numeric = uint.TryParse(runId, NumberStyles.None, CultureInfo.InvariantCulture, out uint number);
                    if (!numeric && (root != Path.Combine(preservedDirectory, "runs") || !Guid.TryParseExact(runId, "N", out _))) continue;
                    string source = numeric ? Path.Combine(root, number.ToString(CultureInfo.InvariantCulture)) : directory;
                    foreach (string path in Files(source))
                    {
                        if (!uint.TryParse(Path.GetFileNameWithoutExtension(path), NumberStyles.None, CultureInfo.InvariantCulture, out uint id)) continue;
                        if (numeric) maximum = Math.Max(maximum, id);
                        else
                        {
                            try { maximum = Math.Max(maximum, StatisticsJson.ParseVersionOneCombat(Read(path), runId, id).Combat.CombatId); }
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
            foreach (var (root, version) in Stores) ReadHeaders(root, version, strictHeaders: true);
            if (!ReadCombats(run).Any(combat => MatchesStorage(combat.RunId, run))) return false;
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
            var records = ReadCombats(run);
            // GUID recordings reused native IDs after restart. Equal-key sorting
            // depends on the full archive, including unrelated records.
            if (records.Select(record => record.Combat.CombatId).Distinct().Count() != records.Count)
                records = ReadStore().Combats.Where(record => MatchesStorage(record.RunId, run)).ToList();
            foreach (var record in records)
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
        var candidates = snapshot.Headers.Where(header => header.Identity == identity)
            .Concat(snapshot.Combats.Reverse().Select(combat => combat.Run).Where(run => run?.Identity == identity)).ToArray();
        if (candidates.Length == 0) return null;
        var recordedIds = candidates.Select(run => run.RunId).Distinct(StringComparer.Ordinal).ToArray();
        var owners = candidates.Where(candidate => recordedIds.All(id => MatchesStorage(id, candidate)))
            .DistinctBy(run => run.RunId).ToArray();
        if (snapshot.InvalidImports.Contains(identity) || owners.Length != 1)
        {
            report("multiple profiler runs match the selected game run");
            return null;
        }
        return candidates[0] with { RunId = owners[0].RunId, PreservedRunIds = owners[0].PreservedRunIds };
    }

    private static bool MatchesStorage(string id, RunRecord run)
        => id == run.RunId || run.PreservedRunIds.Contains(id, StringComparer.Ordinal);

    private static bool Belongs(CombatRecord combat, RunRecord run)
        => combat.Run != null && MatchesStorage(combat.RunId, run)
            && combat.Run.Identity != null && combat.Run.Identity == run.Identity;

    private (string Root, int Version)[] Stores => new[] { (dataDirectory, 0), (preservedDirectory, 1), (versionDirectory, SchemaVersion) };

    private StoreSnapshot ReadStore(bool includeCombats = true, bool strictHeaders = false)
    {
        var headers = new List<RunRecord>();
        var combats = new List<CombatRecord>();
        var invalidImports = new HashSet<RunIdentity>();
        uint maximum = 0;
        foreach (var (root, version) in Stores)
        {
            var sourceHeaders = ReadHeaders(root, version, strictHeaders);
            var sourceCombats = ReadCombats(root, version, null, includeCombats, sourceHeaders);
            foreach (var header in sourceHeaders)
                if (uint.TryParse(header.RunId, NumberStyles.None, CultureInfo.InvariantCulture, out uint reserved)) maximum = Math.Max(maximum, reserved);
            if (version == 1) NormalizeVersionOneStore(sourceHeaders, sourceCombats, invalidImports);
            headers.AddRange(sourceHeaders);
            combats.AddRange(sourceCombats);
        }
        combats.Sort((left, right) => left.Combat.CombatId.CompareTo(right.Combat.CombatId));
        return new(headers, combats, invalidImports, maximum);
    }

    private static void NormalizeVersionOneStore(List<RunRecord> headers, List<CombatRecord> combats, HashSet<RunIdentity> invalid)
    {
        var ordered = headers.OrderBy(run => run.PreservedRunIds.Count != 0 && uint.TryParse(run.RunId, out _)).ToArray();
        headers.Clear();
        headers.AddRange(ordered);
        var candidates = headers.Concat(combats.OrderByDescending(combat => combat.Combat.CombatId).Select(combat => combat.Run)).Where(run => run?.Identity != null);
        var normalized = new Dictionary<(RunIdentity, string), (string Id, IReadOnlyList<string> Preserved)>();
        foreach (var group in candidates.GroupBy(run => run.Identity))
        {
            var aliases = group.SelectMany(run => run.PreservedRunIds.Select(id => (Prior: id, Current: run.RunId)))
                .Where(link => link.Prior != link.Current).GroupBy(link => link.Prior, StringComparer.Ordinal)
                .ToDictionary(links => links.Key, links => links.First().Current, StringComparer.Ordinal);
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
            var components = group.GroupBy(run => Canonical(run.RunId)).ToArray();
            if (components.Any(component => component.Key == null)) { invalid.Add(group.Key); continue; }
            foreach (var component in components)
            {
                var preserved = Array.AsReadOnly(component.SelectMany(run => run.PreservedRunIds.Prepend(run.RunId))
                    .Where(id => id != component.Key).Distinct(StringComparer.Ordinal).ToArray());
                foreach (var run in component) normalized[(group.Key, run.RunId)] = (component.Key, preserved);
            }
        }
        RunRecord Normalize(RunRecord run)
            => run?.Identity != null && normalized.TryGetValue((run.Identity, run.RunId), out var value)
                ? run with { RunId = value.Id, PreservedRunIds = value.Preserved } : run;
        for (int i = 0; i < headers.Count; i++) headers[i] = Normalize(headers[i]);
        for (int i = 0; i < combats.Count; i++) combats[i] = combats[i] with { Run = Normalize(combats[i].Run) };
    }

    private List<RunRecord> ReadHeaders(string root, int version, bool strictHeaders = false)
    {
        var headers = new List<RunRecord>();
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
                if (version != 0)
                {
                    header = version == 1 ? StatisticsJson.ParseVersionOneRun(line) : StatisticsJson.ParseRun(line);
                }
                else header = LegacyHeader(JsonSerializer.Deserialize<LegacyRun>(line, StatisticsJson.Options) ?? throw new InvalidDataException("Missing legacy header"));
                if (header.RunId != "0" && header.Outcome is "victory" or "defeat" or "abandoned") headers.Add(header);
            }
            catch (Exception ex) when (RecordFailure(ex)) { report($"cannot parse run header: {ex.Message}"); }
        }
        return headers;
    }

    private List<CombatRecord> ReadCombats(RunRecord selected)
    {
        var combats = new List<CombatRecord>();
        foreach (var (root, version) in Stores) combats.AddRange(ReadCombats(root, version, selected));
        combats.Sort((left, right) => left.Combat.CombatId.CompareTo(right.Combat.CombatId));
        return combats;
    }

    private List<CombatRecord> ReadCombats(string root, int version, RunRecord selected, bool includeCombats = true, List<RunRecord> headers = null)
    {
        var combats = new List<CombatRecord>();
        string[] directories;
        try { directories = Directories(Path.Combine(root, "runs")); }
        catch (Exception ex) when (RecordFailure(ex)) { report($"cannot list history: {ex.Message}"); directories = Array.Empty<string>(); }
        foreach (string directory in directories)
        {
            string id = Path.GetFileName(directory);
            bool numeric = uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint numericId);
            string sourceDirectory = numeric ? Path.Combine(root, "runs", numericId.ToString(CultureInfo.InvariantCulture)) : directory;
            if (numeric) id = numericId.ToString(CultureInfo.InvariantCulture);
            if (selected != null && !MatchesStorage(id, selected)) continue;
            RunRecord guidHeader = null;
            if (!numeric)
            {
                if (version != 1 || !Guid.TryParseExact(id, "N", out _)) continue;
                try { guidHeader = StatisticsJson.ParseVersionOneRun(Read(Path.Combine(directory, "run.json")), id); }
                catch (Exception ex) when (RecordFailure(ex)) { report($"cannot read preserved run: {ex.Message}"); continue; }
                if (guidHeader.Outcome is "victory" or "defeat" or "abandoned") headers?.Add(guidHeader);
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
                    if (version != 0)
                    {
                        string json = Read(sourcePath);
                        record = version == 1 ? StatisticsJson.ParseVersionOneCombat(json, id, ordinal) : StatisticsJson.ParseCombat(json, id, ordinal);
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
        return combats;
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
        if (!uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint number) || id != number.ToString(CultureInfo.InvariantCulture))
            throw new InvalidDataException("New records require a canonical numeric run identity");
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
