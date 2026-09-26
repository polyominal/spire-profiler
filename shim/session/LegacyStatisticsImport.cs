using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace SpireProfiler;

// The managed importer retains the historical JSON parsers and alias rules.
// Its source metadata enters one SQLite transaction; later reads never revisit old files.
internal sealed class LegacyStatisticsImport
{
    private const int SchemaVersion = StatisticsStore.SchemaVersion;
    private const int MaxDocumentBytes = 64 * 1024 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private readonly string dataDirectory;
    private readonly string preservedDirectory;
    private readonly string versionDirectory;
    private readonly Action<string> report;
    private readonly Dictionary<string, byte[]> fingerprints = new(StringComparer.Ordinal);

    internal LegacyStatisticsImport(string dataDirectory, Action<string> report)
    {
        this.dataDirectory = dataDirectory;
        preservedDirectory = Path.Combine(dataDirectory, "statistics-v1");
        versionDirectory = Path.Combine(dataDirectory, "statistics-v2");
        this.report = report;
    }

    internal bool Import(Func<object, bool> send)
    {
        var snapshot = ReadStore();
        uint maximumRun = snapshot.MaximumRunId;
        uint maximumCombat = 0;
        foreach (var (root, version) in Stores)
        {
            try
            {
                foreach (string directory in Directories(Path.Combine(root, "runs")))
                {
                    string id = Path.GetFileName(directory);
                    bool numeric = uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint number);
                    if (numeric) maximumRun = Math.Max(maximumRun, number);
                    if (!numeric && (version != 1 || !Guid.TryParseExact(id, "N", out _))) continue;
                    try
                    {
                        foreach (string path in Files(directory))
                            if (numeric && uint.TryParse(Path.GetFileNameWithoutExtension(path), NumberStyles.None, CultureInfo.InvariantCulture, out uint ordinal))
                                maximumCombat = Math.Max(maximumCombat, ordinal);
                    }
                    catch (Exception error) when (RecordFailure(error)) { report($"cannot reserve preserved combat IDs: {error.Message}"); }
                }
            }
            catch (Exception error) when (RecordFailure(error)) { report($"cannot reserve preserved run IDs: {error.Message}"); }
        }
        foreach (var combat in snapshot.Combats) maximumCombat = Math.Max(maximumCombat, combat.CombatId);
        var identities = snapshot.Headers.Select(run => run.Identity)
            .Concat(snapshot.Combats.Select(combat => combat.Run?.Identity)).Concat(snapshot.InvalidImports)
            .Where(identity => identity != null).Distinct().ToArray();
        if (!send(new { op = "import_begin", max_run_id = maximumRun, max_combat_id = maximumCombat })) return false;
        foreach (var identity in identities)
        {
            var run = Match(snapshot, identity);
            if (run == null)
            {
                if (!send(new { op = "import_ambiguous", identity })) return false;
                continue;
            }
            string sourceKey = JsonSerializer.Serialize(new { identity, run.RunId }, StatisticsJson.Options);
            var sources = snapshot.Combats.Where(combat => Belongs(combat, run)).ToArray();
            var summary = run.EmptySummary() with { Combats = (uint)sources.Length };
            var reasons = snapshot.Read.Apply(summary, run).Coverage.Reasons;
            if (!send(new
            {
                op = "import_run",
                entry = new
                {
                    source_key = sourceKey,
                    run,
                    reasons,
                    finalized = run.Outcome is "victory" or "defeat" or "abandoned"
                }
            })) return false;
            foreach (var source in sources)
            {
                var combat = ReadCombat(source.Path, source.Version, source.RunId, source.Ordinal, source.GuidHeader);
                if (combat.Combat.CombatId != source.CombatId || combat.Run?.Identity != source.Run?.Identity)
                    throw new InvalidDataException("Legacy combat changed during import");
                combat = combat with
                {
                    RunId = run.RunId,
                    Run = source.Run with { RunId = run.RunId, PreservedRunIds = run.PreservedRunIds }
                };
                if (!send(new { op = "import_record", source_key = sourceKey, record = combat })) return false;
            }
        }
        VerifySources();
        return send(new { op = "import_end", complete = true });
    }

    private sealed record CombatSource(string Path, int Version, string RunId, uint Ordinal, uint CombatId, RunRecord Run, RunRecord GuidHeader);
    private sealed record StoreSnapshot(IReadOnlyList<RunRecord> Headers, IReadOnlyList<CombatSource> Combats, IReadOnlySet<RunIdentity> InvalidImports, uint MaximumRunId, ReadState Read);

    private sealed record ReadGap(int Version, string RunId, IReadOnlySet<RunIdentity> Owners, string Reason);

    private sealed class ReadState
    {
        internal readonly List<ReadGap> Gaps = new();
        internal readonly HashSet<RunIdentity> Finalized = new();

        internal SummaryView Apply(SummaryView summary, RunRecord run)
        {
            var reasons = Gaps.Where(gap => gap.RunId == null ? gap.Owners.Contains(run.Identity)
                : gap.Owners.Count != 0 ? gap.Owners.Contains(run.Identity)
                : gap.Version == SchemaVersion ? gap.RunId == run.RunId : run.PreservedRunIds.Contains(gap.RunId, StringComparer.Ordinal))
                .Select(gap => gap.Reason);
            if (summary.Combats == 0 && Finalized.Contains(run.Identity)) reasons = reasons.Append("statistics-record-missing");
            foreach (string reason in reasons.Distinct(StringComparer.Ordinal))
                summary = summary with { Coverage = summary.Coverage.WithFailure(reason) };
            return summary;
        }
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

    private static bool Belongs(CombatSource combat, RunRecord run)
        => combat.Run != null && MatchesStorage(combat.RunId, run)
            && combat.Run.Identity != null && combat.Run.Identity == run.Identity;

    private (string Root, int Version)[] Stores => new[] { (dataDirectory, 0), (preservedDirectory, 1), (versionDirectory, SchemaVersion) };

    private StoreSnapshot ReadStore()
    {
        var headers = new List<RunRecord>();
        var combats = new List<CombatSource>();
        var invalidImports = new HashSet<RunIdentity>();
        var read = new ReadState();
        uint maximum = 0;
        foreach (var (root, version) in Stores)
        {
            var sourceHeaders = ReadHeaders(root, version, out bool incomplete);
            var sourceCombats = ReadCombats(root, version, read, sourceHeaders);
            if (incomplete)
                read.Gaps.Add(new(version, null, sourceHeaders.Select(run => run.Identity).Concat(sourceCombats.Select(combat => combat.Run?.Identity))
                    .Where(identity => identity != null).ToHashSet(), "statistics-read-failed"));
            foreach (var header in sourceHeaders)
                if (uint.TryParse(header.RunId, NumberStyles.None, CultureInfo.InvariantCulture, out uint reserved)) maximum = Math.Max(maximum, reserved);
            if (version == 1) NormalizeVersionOneStore(sourceHeaders, sourceCombats, invalidImports);
            headers.AddRange(sourceHeaders);
            combats.AddRange(sourceCombats);
        }
        combats.Sort((left, right) => left.CombatId.CompareTo(right.CombatId));
        return new(headers, combats, invalidImports, maximum, read);
    }

    private static void NormalizeVersionOneStore(List<RunRecord> headers, List<CombatSource> combats, HashSet<RunIdentity> invalid)
    {
        var ordered = headers.OrderBy(run => run.PreservedRunIds.Count != 0 && uint.TryParse(run.RunId, out _)).ToArray();
        headers.Clear();
        headers.AddRange(ordered);
        var candidates = headers.Concat(combats.OrderByDescending(combat => combat.CombatId).Select(combat => combat.Run)).Where(run => run?.Identity != null);
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

    private List<RunRecord> ReadHeaders(string root, int version, out bool incomplete)
    {
        incomplete = false;
        var headers = new List<RunRecord>();
        string headerPath = Path.Combine(root, "runs.jsonl");
        try
        {
            if (File.Exists(headerPath) || Directory.Exists(headerPath))
                foreach (string line in ReadLines(headerPath))
                {
                    if (string.IsNullOrWhiteSpace(line)) continue;
                    try
                    {
                        RunRecord header;
                        if (version != 0)
                            header = version == 1 ? StatisticsJson.ParseVersionOneRun(line) : StatisticsJson.ParseRun(line);
                        else header = LegacyHeader(JsonSerializer.Deserialize<LegacyRun>(line, StatisticsJson.Options)
                            ?? throw new InvalidDataException("Missing legacy header"));
                        if (header.RunId != "0" && header.Outcome is "victory" or "defeat" or "abandoned") headers.Add(header);
                    }
                    catch (Exception ex) when (RecordFailure(ex)) { incomplete = true; report($"cannot parse run header: {ex.Message}"); }
                }
        }
        catch (Exception ex) when (RecordFailure(ex))
        {
            headers.Clear();
            incomplete = true;
            report($"cannot read run headers: {ex.Message}");
        }
        return headers;
    }

    private List<CombatSource> ReadCombats(string root, int version, ReadState read, List<RunRecord> headers)
    {
        var combats = new List<CombatSource>();
        string[] directories;
        try { directories = Directories(Path.Combine(root, "runs")); }
        catch (Exception ex) when (RecordFailure(ex))
        {
            report($"cannot list history: {ex.Message}");
            read.Gaps.Add(new(version, null, headers.Where(header => header.Identity != null).Select(header => header.Identity).ToHashSet(), "statistics-read-failed"));
            directories = Array.Empty<string>();
        }
        foreach (string directory in directories)
        {
            string id = Path.GetFileName(directory);
            bool numeric = uint.TryParse(id, NumberStyles.None, CultureInfo.InvariantCulture, out uint numericId);
            string sourceDirectory = numeric ? Path.Combine(root, "runs", numericId.ToString(CultureInfo.InvariantCulture)) : directory;
            if (numeric) id = numericId.ToString(CultureInfo.InvariantCulture);
            var headerOwners = headers.Where(header => header.RunId == id && header.Identity != null).Select(header => header.Identity).ToHashSet();
            var owners = headerOwners.ToHashSet();
            var reasons = new HashSet<string>(StringComparer.Ordinal);
            RunRecord guidHeader = null;
            if (!numeric)
            {
                if (version != 1 || !Guid.TryParseExact(id, "N", out _)) continue;
                try { guidHeader = StatisticsJson.ParseVersionOneRun(ReadDocument(Path.Combine(directory, "run.json")), id); }
                catch (Exception ex) when (RecordFailure(ex))
                {
                    report($"cannot read preserved run: {ex.Message}");
                    read.Gaps.Add(new(version, id, owners, "statistics-read-failed"));
                    continue;
                }
                if (guidHeader.Identity != null) owners.Add(guidHeader.Identity);
                if (guidHeader.Outcome is "victory" or "defeat" or "abandoned") headers?.Add(guidHeader);
            }
            string[] paths;
            try { paths = Files(sourceDirectory); }
            catch (Exception ex) when (RecordFailure(ex)) { report($"cannot list combats: {ex.Message}"); reasons.Add("statistics-read-failed"); paths = Array.Empty<string>(); }
            if (!Directory.Exists(sourceDirectory) && owners.Count != 0) reasons.Add("statistics-read-failed");
            foreach (string path in paths)
            {
                if (!uint.TryParse(Path.GetFileNameWithoutExtension(path), NumberStyles.None, CultureInfo.InvariantCulture, out uint ordinal)) continue;
                try
                {
                    string sourcePath = numeric ? Path.Combine(sourceDirectory, $"{ordinal}.json") : path;
                    var record = ReadCombat(sourcePath, version, id, ordinal, guidHeader);
                    if (numeric && (id == "0" ? record.Run != null : record.Run?.RunId != id)) throw new InvalidDataException("Combat run identity differs from directory");
                    if (headerOwners.Count != 0 && !headerOwners.Contains(record.Run?.Identity)) throw new InvalidDataException("Combat identity differs from finalized headers");
                    if (record.Run?.Identity != null) owners.Add(record.Run.Identity);
                    combats.Add(new(sourcePath, version, id, ordinal, record.Combat.CombatId, record.Run, guidHeader));
                }
                catch (Exception ex) when (RecordFailure(ex)) { report($"cannot parse combat '{path}': {ex.Message}"); reasons.Add("statistics-read-failed"); }
            }
            foreach (string reason in reasons) read.Gaps.Add(new(version, id, owners, reason));
        }
        read.Finalized.UnionWith(headers.Where(header => header.Identity != null).Select(header => header.Identity));
        return combats;
    }

    private CombatRecord ReadCombat(string path, int version, string id, uint ordinal, RunRecord guidHeader)
    {
        string json = ReadDocument(path);
        CombatRecord record = version == 0 ? LegacyCombatRecord(json, id, ordinal)
            : version == 1 ? StatisticsJson.ParseVersionOneCombat(json, id, ordinal)
            : StatisticsJson.ParseCombat(json, id, ordinal);
        if (guidHeader != null) record = record with { Run = guidHeader with { Outcome = "", EndedAt = 0, Players = Array.Empty<PlayerSummary>() } };
        if (record.Run != null) record = record with { Run = record.Run with { Outcome = "", EndedAt = 0, Players = Array.Empty<PlayerSummary>() } };
        return record;
    }

    private static RunRecord LegacyHeader(LegacyRun run)
    {
        if (run.Seed == null || run.Character == null || run.GameMode == null || run.Players == null
            || run.Profile < -1 || run.StartedAt < 0 || run.EndedAt < 0
            || run.Outcome is not ("victory" or "defeat" or "abandoned") || run.Players.Length > 4
            || run.Players.Any(player => player == null || string.IsNullOrEmpty(player.Character) || player.Slot is < 0 or > 3)
            || run.Players.Select(player => player.Slot).Distinct().Count() != run.Players.Length) throw new InvalidDataException("Invalid legacy header");
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
        if (combat.CombatId != ordinal || ordinal == 0 || combat.Cards == null || combat.EncounterId == null
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
        if (run != null && (run.Character == null || run.GameMode == null || run.Seed == null || run.Profile < -1 || run.StartedAt < 0)) throw new InvalidDataException("Invalid legacy run identity");
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

    private IEnumerable<string> ReadLines(string path)
    {
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        using var line = new MemoryStream();
        byte[] buffer = new byte[64 * 1024];
        int count;
        while ((count = file.Read(buffer)) != 0)
        {
            hash.AppendData(buffer, 0, count);
            int start = 0;
            for (int index = 0; index < count; index++)
            {
                if (buffer[index] != (byte)'\n') continue;
                if (line.Length + index - start > MaxDocumentBytes) throw new InvalidDataException("Statistics record exceeds size limit");
                line.Write(buffer, start, index - start);
                if (line.Length != 0) yield return Utf8.GetString(line.ToArray());
                line.SetLength(0);
                start = index + 1;
            }
            if (line.Length + count - start > MaxDocumentBytes) throw new InvalidDataException("Statistics record exceeds size limit");
            line.Write(buffer, start, count - start);
        }
        if (line.Length != 0) yield return Utf8.GetString(line.ToArray());
        fingerprints.Add(path, hash.GetHashAndReset());
    }

    private string ReadDocument(string path)
    {
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (file.Length > MaxDocumentBytes) throw new InvalidDataException("Statistics document exceeds size limit");
        byte[] bytes = new byte[checked((int)file.Length)];
        file.ReadExactly(bytes);
        byte[] hash = SHA256.HashData(bytes);
        if (fingerprints.TryGetValue(path, out byte[] expected))
        {
            if (!hash.SequenceEqual(expected)) throw new InvalidDataException("Legacy source changed during import");
        }
        else fingerprints.Add(path, hash);
        return Utf8.GetString(bytes);
    }

    private void VerifySources()
    {
        foreach (var (path, expected) in fingerprints)
        {
            using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
            if (!SHA256.HashData(file).SequenceEqual(expected))
                throw new InvalidDataException("Legacy source changed during import");
        }
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
