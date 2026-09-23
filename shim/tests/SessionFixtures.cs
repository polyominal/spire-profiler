using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

#pragma warning disable CA1861 // Fixture data stays beside the assertions it explains.

namespace SpireProfiler;

internal static class SessionFixtures
{
    private static int assertions;

    internal static void Run(string projectDirectory, string nativeLibrary)
    {
        string scratch = Path.Combine(projectDirectory, "session-fixtures");
        if (Directory.Exists(scratch)) throw new IOException("Session fixture scratch already exists");
        Directory.CreateDirectory(scratch);
        assertions = 0;
        try
        {
            ParserAndAggregateContracts();
            AtomicStoreAndIdentity(Path.Combine(scratch, "store"));
            SelectedHistoryAndCorruption(Path.Combine(scratch, "history"));
            LegacyHistory(Path.Combine(scratch, "legacy"));
            ProfilerNative.Load(nativeLibrary);
            NativeSessionLifecycle(Path.Combine(scratch, "session"));
            PendingWritesSurviveResume(Path.Combine(scratch, "retry-session"));
            InterruptedCombatRecovery(Path.Combine(scratch, "interrupted-session"));
            var messages = new List<string>();
            ProfilerSession.Initialize(Path.Combine(scratch, "self-test"), "fixture-game", "fixture-mod", messages.Add);
            try { ProfilerSession.SelfTest(messages.Add); }
            catch (Exception error)
            {
                throw new InvalidOperationException($"Native self-test diagnostics: {string.Join("; ", messages)}; snapshot: {JsonSerializer.Serialize(ProfilerSession.CurrentCombat, StatisticsJson.Options)}", error);
            }
            Check(messages.Contains("[SpireProfiler] managed session self-test: PASS")
                && messages.Contains("[SpireProfiler] managed records: PASS"), "Production self-test must prove actual native replay and persisted accounting");
            Console.WriteLine($"MANAGED SESSION FIXTURES PASS ({assertions} assertions)");
        }
        finally
        {
            ProfilerSession.Suspend();
            ProfilerNative.Dispose();
            Directory.Delete(scratch, recursive: true);
        }
    }

    private static void ParserAndAggregateContracts()
    {
        var valid = JsonNode.Parse("""
            {"policy_version":1,"combat_id":7,"encounter_id":"CULTIST","encounter_type":"Normal",
            "started_at":123,"result":"completed","turns":2,"plays":1,"potions_used":0,"damage_received":3,"block_total":17,
            "cards":[{"id":"STRIKE","kind":0,"player":0,"plays":1,"damage_dealt":9,"damage_blocked":2,"dmg_direct":6,"dmg_attributed":2,"dmg_modifier":1}],
            "coverage":{"complete":true,"failures":0,"reasons":[]}}
            """);
        var parsed = StatisticsJson.ParseNative(valid.ToJsonString());
        Check(parsed.Cards.Single().DamageDealt == 9 && parsed.Coverage.Complete, "Native boundary must retain observed accounting and coverage");
        var metadata = Header("METADATA", 123) with { Character = "DEFECT", Ascension = 7, GameMode = "Standard" };
        var combatView = parsed.View(metadata.Players, metadata);
        Check(combatView.BlockTotal == 17 && combatView.Ascension == 7 && combatView.GameMode == "Standard"
            && combatView.Character == "DEFECT" && combatView.Title == "CULTIST",
            "Combat presentation needs measured total block and the original run metadata independently of source rows");
        Check(parsed.Cards is not StatRow[] && parsed.Coverage.Reasons is not string[], "Published native collections must not expose mutable backing arrays");
        foreach (var invalid in new Action<JsonNode>[]
        {
            node => node["coverage"]["failures"] = 1,
            node => node["coverage"]["reasons"] = JsonNode.Parse("[\"lost-hook\"]"),
            node => { node["coverage"]["complete"] = false; node["coverage"]["reasons"] = JsonNode.Parse("[null]"); },
            node => node["cards"][0]["player"] = 5,
            node => node["cards"][0]["kind"] = 6,
            node => node["cards"][0]["damage_blocked"] = 10,
            node => node["cards"][0]["dmg_direct"] = 5,
            node => node["cards"][0]["self_damage"] = -1,
            node => node["cards"].AsArray().Add(node["cards"][0].DeepClone()),
            node => node["result"] = "victory",
        })
        {
            var document = valid.DeepClone();
            invalid(document);
            Reject(() => StatisticsJson.ParseNative(document.ToJsonString()), "Malformed native snapshot must fail before publication");
        }
        Check(parsed.Cards.Single().DmgDirect == 6 && parsed.Coverage.Complete, "Rejected parses must not change a prior immutable snapshot");
        var huge = new SummaryView
        {
            Cards = new[] { new StatRow { Id = "A", DamageDealt = long.MaxValue, DmgDirect = long.MaxValue } },
            Turns = 4,
            Combats = 1,
            Coverage = CoverageSummary.Healthy
        };
        var incoming = new SummaryView
        {
            Cards = new[] { new StatRow { Id = "EARLY", Forge = 2 }, new StatRow { Id = "B", DamageDealt = 1, DmgDirect = 1 } },
            Turns = 3,
            Combats = 1,
            Coverage = CoverageSummary.Healthy
        };
        var failed = huge.Add(incoming);
        Check(failed.Cards.Count == 1 && failed.Cards[0].DamageDealt == long.MaxValue
            && failed.Turns == 4 && failed.Combats == 1 && !failed.Coverage.Complete,
            "Late aggregate overflow must reject the whole merge and mark its retained result incomplete");
        Check(huge.Coverage.Complete && incoming.Cards.Count == 2, "Rejected aggregation must leave both input summaries unchanged");
        Reject(() => StatisticsJson.CheckRows(new[]
        {
            new StatRow { Id = "A", BlockGained = long.MaxValue }, new StatRow { Id = "B", BlockGained = 1 }
        }), "Accepted block totals must fit the UI and aggregate accounting domain");
        var run = Header("SCHEMA", 100) with { RunId = "11111111111111111111111111111111" };
        var runJson = JsonSerializer.SerializeToNode(run, StatisticsJson.Options);
        runJson.AsObject().Remove("schema_version");
        Reject(() => StatisticsJson.ParseRun(runJson.ToJsonString(), run.RunId), "Unversioned records must not masquerade as current schema");
        var record = Record(run, 1, 9);
        var combatJson = JsonSerializer.SerializeToNode(record, StatisticsJson.Options);
        combatJson.AsObject().Remove("schema_version");
        Reject(() => StatisticsJson.ParseCombat(combatJson.ToJsonString(), run.RunId, 1), "Combat schema identity must be explicit");
        combatJson = JsonSerializer.SerializeToNode(record, StatisticsJson.Options);
        combatJson["combat"]["coverage"]["quality"] = "partial";
        combatJson["combat"]["coverage"]["reasons"] = JsonNode.Parse("[null]");
        Reject(() => StatisticsJson.ParseCombat(combatJson.ToJsonString(), run.RunId, 1), "Stored failure reasons must be meaningful strings");
    }

    private static void AtomicStoreAndIdentity(string directory)
    {
        var diagnostics = new List<string>();
        var store = new StatisticsStore(directory, "game-v", "mod-v", diagnostics.Add);
        var run = store.OpenRun(Header("EXACT", 100), continued: false);
        Check(store.SaveRun(run), "New run header must persist");
        var combat = Record(run, 1, 9);
        Check(store.SaveCombat(combat), "Completed combat must persist");
        string path = Path.Combine(directory, "statistics-v1", "runs", run.RunId, "00000001.json");
        byte[] original = File.ReadAllBytes(path);
        Check(store.SaveCombat(combat), "Identical immutable write retry must be idempotent");
        Check(!store.SaveCombat(Record(run, 1, 10)), "Conflicting immutable retry must fail");
        Check(File.ReadAllBytes(path).SequenceEqual(original), "Conflicting retry must preserve the original bytes");
        string headerPath = Path.Combine(Path.GetDirectoryName(path), "run.json");
        byte[] headerBytes = File.ReadAllBytes(headerPath);
        Directory.CreateDirectory(headerPath + ".tmp");
        Check(!store.SaveRun(run with { Outcome = "victory", EndedAt = 200 }), "Failed staging write must report failure");
        Check(File.ReadAllBytes(headerPath).SequenceEqual(headerBytes), "Failed staging must leave the published header intact");
        Directory.Delete(headerPath + ".tmp");
        Check(store.SaveRun(run with { Outcome = "suspended" }), "A later header retry must recover after transient failure");
        Check(!Directory.EnumerateFiles(Path.GetDirectoryName(path), "*.tmp").Any(), "Successful atomic writes must not retain staging files");
        var resumed = store.OpenRun(Header("EXACT", 100), continued: true);
        Check(resumed.RunId == run.RunId && store.LoadRun(resumed).Summary.Combats == 1, "Exact resume identity must reopen completed combat history");
        foreach (var different in new[] { Header("EXACT", 101), Header("OTHER", 100), Header("EXACT", 100) with { Profile = 2 } })
            Check(store.OpenRun(different, continued: true).RunId != run.RunId, "Seed, start time, and profile must all match before resume");
        Check(store.Select(RunIdentity.Parse(0, "EXACT", 101)) == null, "History must never use a near identity match");
        var duplicate = store.OpenRun(Header("EXACT", 100), continued: false);
        Check(duplicate.RunId != run.RunId && store.SaveRun(duplicate), "Independent recordings may share a game identity without overwriting files");
        Check(store.Select(run.Identity) == null, "Ambiguous recordings must not silently select one run");
        Check(store.OpenRun(Header("EXACT", 100), continued: true).RunId != run.RunId, "Ambiguous resume must not append to an arbitrary recording");
        Check(diagnostics.Count >= 2, "Write failure and ambiguous identity must be diagnosed");
    }

    private static void SelectedHistoryAndCorruption(string directory)
    {
        var diagnostics = new List<string>();
        var store = new StatisticsStore(directory, "game-v", "mod-v", diagnostics.Add);
        var first = store.OpenRun(Header("FIRST", 100), continued: false);
        var second = store.OpenRun(Header("SECOND", 200), continued: false);
        Check(store.SaveRun(first) && store.SaveRun(second) && store.SaveCombat(Record(first, 1, 9)) && store.SaveCombat(Record(second, 1, 4)),
            "History fixture must persist two independent runs");
        string secondPath = Path.Combine(directory, "statistics-v1", "runs", second.RunId, "00000001.json");
        File.WriteAllText(secondPath, "{corrupt");
        var selected = store.Select(first.Identity);
        Check(selected.Combats == 1 && selected.Cards.Single().DamageDealt == 9 && diagnostics.Count == 0,
            "Selecting one run must not eagerly read another run's damaged combats");
        selected = store.Select(second.Identity);
        Check(selected.Combats == 0 && selected.Coverage.Quality == CaptureQuality.Partial && diagnostics.Count == 1,
            "Corrupt selected combat must expose incomplete coverage without manufacturing rows");
        string firstPath = Path.Combine(directory, "statistics-v1", "runs", first.RunId, "00000001.json");
        File.WriteAllText(firstPath, JsonSerializer.Serialize(Record(first, 1, 12), StatisticsJson.Options));
        selected = store.Select(first.Identity);
        Check(selected.Cards.Single().DamageDealt == 12, "Selecting history again must reload combat data rather than retaining all historical rows");
        string damagedPath = Path.Combine(Path.GetDirectoryName(firstPath), "00000003.json");
        File.WriteAllText(damagedPath, "null");
        var loaded = store.LoadRun(first);
        Check(loaded.LastOrdinal == 3 && loaded.Summary.Combats == 1 && !loaded.Summary.Coverage.Complete,
            "Corrupt record ordinals must remain reserved when resuming");
        Check(store.SaveCombat(Record(first, 4, 2) with { Combat = Record(first, 4, 2).Combat with { PolicyVersion = 2 } }), "Mixed-policy fixture must persist");
        loaded = store.LoadRun(first);
        Check(loaded.Summary.Coverage.Reasons.Contains("mixed-attribution-policies"), "Mixed attribution policies must be visibly incomparable");
    }

    private static void LegacyHistory(string directory)
    {
        Directory.CreateDirectory(Path.Combine(directory, "runs", "42"));
        string runPath = Path.Combine(directory, "runs.jsonl");
        string combatPath = Path.Combine(directory, "runs", "42", "7.json");
        const string header = """
            {"run_id":42,"profile":0,"seed":"LEGACY","started_at":100,"ended_at":200,"character":"IRONCLAD","game_mode":"standard","outcome":"victory","players":[{"slot":0,"character":"IRONCLAD"}]}
            """;
        const string combat = """
            {"combat_id":7,"started_at":110,"result":"completed","turns":2,"damage_received":3,"run":{"seq":42,"seed":"LEGACY","profile":0,"started_at":100},"cards":[{"id":"STRIKE","kind":0,"player":0,"plays":1,"damage_dealt":9,"dmg_direct":9}]}
            """;
        File.WriteAllText(runPath, header + "\n");
        File.WriteAllText(combatPath, combat);
        byte[] beforeRun = File.ReadAllBytes(runPath), beforeCombat = File.ReadAllBytes(combatPath);
        var diagnostics = new List<string>();
        var store = new StatisticsStore(directory, "game-v", "mod-v", diagnostics.Add);
        var selected = store.Select(RunIdentity.Parse(0, "LEGACY", 100));
        Check(selected != null && selected.Combats == 1 && selected.Cards.Single().DamageDealt == 9
            && selected.Coverage.Quality == CaptureQuality.Unknown, "Legacy totals remain readable with unknown provenance and coverage");
        var noRoster = JsonNode.Parse(header);
        noRoster.AsObject().Remove("players");
        noRoster["character"] = "IRONCLAD, SILENT";
        File.WriteAllText(runPath, noRoster.ToJsonString() + "\n");
        selected = store.Select(RunIdentity.Parse(0, "LEGACY", 100));
        Check(selected.Players.Count == 2 && selected.Players[1] == new PlayerSummary(1, "SILENT"),
            "Legacy character lists must retain their per-player filters without an explicit roster");
        File.WriteAllBytes(runPath, beforeRun);
        var fresh = store.OpenRun(Header("LEGACY", 100), continued: true);
        Check(store.SaveRun(fresh), "Resuming a legacy identity must open a versioned store record");
        Check(File.ReadAllBytes(runPath).SequenceEqual(beforeRun) && File.ReadAllBytes(combatPath).SequenceEqual(beforeCombat),
            "Versioned storage and legacy inspection must preserve old files byte-for-byte");
        Check(diagnostics.Count == 0, "Valid legacy files should not be reported as corrupt");
        Check(store.SaveRun(store.OpenRun(Header("LEGACY", 100), continued: false)), "Ambiguous versioned fixture must persist independently");
        Check(store.Select(RunIdentity.Parse(0, "LEGACY", 100)) == null,
            "Ambiguous versioned history must not silently fall back to an older legacy recording");
        var damaged = JsonNode.Parse(combat);
        damaged["cards"] = null;
        File.WriteAllText(combatPath, damaged.ToJsonString());
        var legacyOnly = new StatisticsStore(directory, "game-v", "mod-v", diagnostics.Add);
        Directory.Delete(Path.Combine(directory, "statistics-v1"), recursive: true);
        selected = legacyOnly.Select(RunIdentity.Parse(0, "LEGACY", 100));
        Check(selected?.Coverage.Quality == CaptureQuality.Partial && selected.Combats == 0,
            "Malformed legacy rows must become incomplete coverage instead of escaping as an exception");
    }

    private static void NativeSessionLifecycle(string directory)
    {
        var diagnostics = new List<string>();
        ProfilerSession.Initialize(directory, "game-v", "mod-v", diagnostics.Add);
        var requested = Header("SESSION", 300);
        ProfilerSession.StartRun(requested, continued: false);
        ulong first = ProfilerSession.StartCombat("ONE", "Normal");
        ObserveDamage(first, 9);
        ProfilerSession.Refresh();
        var firstSnapshot = ProfilerSession.CurrentCombat;
        Check(firstSnapshot?.Cards.Single().DamageDealt == 9 && firstSnapshot.Cards.Single().Id == "STRIKE", "Managed session must publish native accounting");
        Check(ProfilerSession.CurrentRun.Combats == 0 && ProfilerSession.CurrentRun.Cards.Count == 0,
            "The live run tab contains only completed combats even while combat statistics refresh");
        Check(ProfilerSession.EndCombat(first) == 1 && ProfilerSession.EndCombat(first) == 0,
            "Combat completion must be accepted exactly once");
        ulong discarded = ProfilerSession.StartCombat("DISCARDED", "Normal");
        ObserveDamage(discarded, 50);
        ProfilerSession.Suspend();
        Check(!ProfilerSession.InRun && ProfilerSession.CurrentCombat == null && ProfilerNative.Snapshot() == "null",
            "Suspending must discard active combat state instead of publishing partial resumed credit");
        string firstPath = Directory.GetFiles(Path.Combine(directory, "statistics-v1"), "00000001.json", SearchOption.AllDirectories).Single();
        var priorPolicy = JsonNode.Parse(File.ReadAllText(firstPath));
        priorPolicy["combat"]["policy_version"] = firstSnapshot.PolicyVersion + 1;
        File.WriteAllText(firstPath, priorPolicy.ToJsonString());
        ProfilerSession.StartRun(requested, continued: true);
        Check(ProfilerSession.CurrentRun.Combats == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 9,
            "Resume must restore completed combats once and exclude the suspended fight");
        ulong resumed = ProfilerSession.StartCombat("TWO", "Normal");
        Check(resumed > discarded && ProfilerSession.EndCombat(discarded) == 0, "Stale combat callbacks must not finish the resumed fight");
        ObserveDamage(resumed, 4);
        Check(ProfilerSession.EndCombat(resumed) == 1 && ProfilerSession.CurrentRun.Combats == 2
            && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 13, "Resumed accounting must add exactly one new completed combat");
        Check(ProfilerSession.CurrentRun.Coverage.Reasons.Contains("mixed-attribution-policies"),
            "Live resumed summary must flag incompatible attribution policies before a history reload");
        Check(firstSnapshot.Cards.Single().DamageDealt == 9, "Later native events must not mutate already published snapshots");
        ProfilerSession.EndRun(0);
        ProfilerSession.SelectHistory("SESSION", 300, 0);
        Check(ProfilerSession.HistoryOpen && ProfilerSession.SelectedHistory?.Combats == 2
            && ProfilerSession.SelectedHistory.Cards.Single().DamageDealt == 13
            && ProfilerSession.SelectedHistory.Outcome == "victory", "Selected history must load the persisted completed session");
        ProfilerSession.ClearHistory();
        Check(!ProfilerSession.HistoryOpen && ProfilerSession.SelectedHistory == null, "Closing history must release its selected summary");
        Check(diagnostics.Count == 0, "Ordinary managed/native lifecycle must not emit diagnostics");
    }

    private static void PendingWritesSurviveResume(string directory)
    {
        var diagnostics = new List<string>();
        ProfilerSession.Initialize(directory, "game-v", "mod-v", diagnostics.Add);
        var requested = Header("RETRY", 400);
        ProfilerSession.StartRun(requested, continued: false);
        ulong first = ProfilerSession.StartCombat("ONE", "Normal");
        ObserveDamage(first, 9);
        Check(ProfilerSession.EndCombat(first) == 1, "Retry fixture must save its first combat");
        string runDirectory = Directory.GetDirectories(Path.Combine(directory, "statistics-v1", "runs")).Single();
        string obstruction = Path.Combine(runDirectory, "00000002.json.tmp");
        Directory.CreateDirectory(obstruction);
        ulong pending = ProfilerSession.StartCombat("PENDING", "Normal");
        ObserveDamage(pending, 5);
        Check(ProfilerSession.EndCombat(pending) == 1 && !ProfilerSession.CurrentRun.Coverage.Complete,
            "Failed persistence must retain accounted combat and expose incomplete storage");
        ProfilerSession.Suspend();
        ProfilerSession.StartRun(requested, continued: true);
        Check(ProfilerSession.CurrentRun.Combats == 2 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 14,
            "Same-process resume must retain still-pending completed combat credit");
        ulong next = ProfilerSession.StartCombat("NEXT", "Normal");
        ObserveDamage(next, 4);
        Check(ProfilerSession.EndCombat(next) == 1, "New combat must finish while an earlier write is pending");
        Directory.Delete(obstruction);
        ProfilerSession.Suspend();
        ProfilerSession.StartRun(requested, continued: true);
        Check(ProfilerSession.CurrentRun.Combats == 3 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 18,
            "Recovered pending write and subsequent combat must retain distinct ordinals and exact credit");
        Check(File.Exists(Path.Combine(runDirectory, "00000002.json")) && File.Exists(Path.Combine(runDirectory, "00000003.json")),
            "Pending ordinals must remain reserved across resume");
        Check(diagnostics.Count > 0, "Failed writes must be diagnosed");
        ProfilerSession.Suspend();
    }

    private static void InterruptedCombatRecovery(string directory)
    {
        var diagnostics = new List<string>();
        string previous = Environment.GetEnvironmentVariable("SPIRE_PROFILER_RECORD");
        try
        {
            Environment.SetEnvironmentVariable("SPIRE_PROFILER_RECORD", "1");
            ProfilerSession.Initialize(directory, "game-v", "mod-v", diagnostics.Add);
        }
        finally { Environment.SetEnvironmentVariable("SPIRE_PROFILER_RECORD", previous); }
        ProfilerSession.StartRun(Header("INTERRUPTED", 500), continued: false);
        ulong first = ProfilerSession.StartCombat("INTERRUPTED", "Normal");
        ObserveDamage(first, 9);
        ulong second = ProfilerSession.StartCombat("REPLACEMENT", "Normal");
        ObserveDamage(second, 4);
        Check(ProfilerSession.EndCombat(second) == 1, "Replacement combat must remain observable after an interrupted predecessor");
        string runDirectory = Directory.GetDirectories(Path.Combine(directory, "statistics-v1", "runs")).Single();
        string trace = File.ReadAllText(Path.Combine(runDirectory, "00000002.trace.json"));
        using (var document = JsonDocument.Parse(trace))
            Check(document.RootElement.GetProperty("observations").EnumerateArray()
                .Count(entry => entry.GetProperty("observation").GetProperty("operation").GetString() == "combat_started") == 1,
                "Each replacement combat recording must start independently of the interrupted trace");
        Check(ProfilerNative.Replay(trace) == ProfilerNative.Snapshot(), "Replacement recording must reproduce its own completed native snapshot");
        using (var document = JsonDocument.Parse(File.ReadAllText(Path.Combine(runDirectory, "00000001.json"))))
            Check(document.RootElement.GetProperty("combat").GetProperty("result").GetString() == "interrupted",
                "Interrupted observed combat must be retained with an explicit terminal status");
        ulong failedEnd = ProfilerSession.StartCombat("NATIVE-RESET", "Normal");
        ObserveDamage(failedEnd, 6);
        ProfilerSession.Refresh();
        ProfilerNative.CombatDiscard();
        Check(ProfilerSession.EndCombat(failedEnd) == 1 && ProfilerSession.CurrentCombat.Outcome == "interrupted"
            && !ProfilerSession.CurrentCombat.Coverage.Complete && ProfilerSession.CurrentCombat.Cards.Single().DamageDealt == 6,
            "Failed native completion must preserve the last validated observations with partial coverage");
        Check(ProfilerSession.CurrentRun.Combats == 3 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 19,
            "Interrupted and failed-completion records must each contribute their observed totals once");
        ProfilerSession.Suspend();
    }

    private static void ObserveDamage(ulong epoch, int amount)
    {
        ulong source = ProfilerNative.SourceCapture(epoch, 1, 1, "STRIKE", 0, 0, 0);
        ulong hit = ProfilerNative.DamageCalculationBegin(epoch, source, 1, 0, 99);
        Check(epoch != 0 && source != 0 && hit != 0
            && ProfilerNative.DamageResultAppend(hit, amount, amount, 0, 0, 4, 0) == 1
            && ProfilerNative.DamageCalculationCommit(hit) == 1, "Fixture damage must cross the real observation ABI");
    }

    private static RunRecord Header(string seed, long startedAt) => new()
    {
        Profile = 0,
        Seed = seed,
        StartedAt = startedAt,
        Character = "IRONCLAD",
        GameMode = "standard",
        Players = Array.AsReadOnly(new[] { new PlayerSummary(0, "IRONCLAD") })
    };

    private static CombatRecord Record(RunRecord run, uint ordinal, int damage) => new()
    {
        RunId = run.RunId,
        Ordinal = ordinal,
        GameVersion = "game-v",
        ModVersion = "mod-v",
        Combat = new CombatStatistics
        {
            PolicyVersion = 1,
            CombatId = ordinal,
            StartedAt = 120,
            Result = "completed",
            Turns = 2,
            Plays = 1,
            Cards = Array.AsReadOnly(new[] { new StatRow { Id = "STRIKE", DamageDealt = damage, DmgDirect = damage, Plays = 1 } }),
            Coverage = CoverageSummary.Healthy
        }
    };

    private static void Reject(Action action, string message)
    {
        bool rejected = false;
        try { action(); }
        catch (Exception ex) when (ex is InvalidDataException or JsonException or OverflowException) { rejected = true; }
        Check(rejected, message);
    }

    private static void Check(bool condition, string message)
    {
        assertions++;
        if (!condition) throw new InvalidOperationException(message);
    }
}
