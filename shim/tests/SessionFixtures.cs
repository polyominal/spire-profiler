using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

#pragma warning disable CA1861

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
            NumericStore(Path.Combine(scratch, "store"));
            LegacyHistory(Path.Combine(scratch, "legacy"));
            PreservedGuidHistory(Path.Combine(scratch, "guid"));
            FailedIdScans(Path.Combine(scratch, "scan"));
            ProfilerNative.Load(nativeLibrary);
            NativeLifecycle(Path.Combine(scratch, "native"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            FailedWriteResume(Path.Combine(scratch, "failed-write"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            InterruptedAndRepeatedHeaders(Path.Combine(scratch, "interrupted"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            var messages = new List<string>();
            ProfilerSession.Initialize(Path.Combine(scratch, "self-test"), "fixture-game", "fixture-mod", messages.Add);
            ProfilerSession.SelfTest(messages.Add);
            Check(messages.Contains("[SpireProfiler] managed session self-test: PASS") && messages.Contains("[SpireProfiler] managed records: PASS"),
                "Production self-test must verify native replay and persisted accounting");
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

    private static void NumericStore(string directory)
    {
        var diagnostics = new List<string>();
        var store = new StatisticsStore(directory, "game-v", "mod-v", diagnostics.Add);
        var empty = store.OpenRun(Header("EMPTY", 100), false);
        Check(empty.RunId == "1" && !store.SaveRun(empty with { Outcome = "victory" }), "An empty run must not publish an end header");
        var run = store.OpenRun(Header("EXACT", 200), false);
        Check(run.RunId == empty.RunId && store.Select(empty.Identity) == null, "An unpersisted run must leave its ID available");
        Check(store.SaveCombat(Record(run, 1, 9)), "First numeric combat must persist");
        var header = run with { Outcome = "victory", EndedAt = 300 };
        Check(store.SaveRun(header), "A run with a stored combat may publish its end header");
        string headerPath = Path.Combine(directory, "statistics-v1", "runs.jsonl");
        byte[] before = File.ReadAllBytes(headerPath);
        Directory.CreateDirectory(headerPath + ".tmp");
        Check(!store.SaveRun(run with { Outcome = "defeat", EndedAt = 400 }), "An obstructed staged header must report failure");
        Check(File.ReadAllBytes(headerPath).SequenceEqual(before), "Failed staging must preserve the published header");
        Directory.Delete(headerPath + ".tmp");
        Check(store.SaveRun(run with { Outcome = "defeat", EndedAt = 400 }), "Later finalization may append another header");
        Check(store.Select(run.Identity).Outcome == "victory", "History must select the first finalized header");
        var resumed = store.OpenRun(Header("EXACT", 200), true);
        Check(resumed.RunId == run.RunId && store.LoadRun(resumed).Summary.Combats == 1, "Resume joins the exact recorded identity");
        var duplicate = store.OpenRun(Header("EXACT", 200), false);
        Check(duplicate.RunId == "2" && store.SaveCombat(Record(duplicate, 2, 4)), "A fresh recording reserves the next durable ID");
        Check(store.Select(run.Identity) == null, "Multiple numeric IDs with one identity must remain ambiguous");
        Directory.CreateDirectory(Path.Combine(directory, "statistics-v1", "runs", "8"));
        File.WriteAllText(Path.Combine(directory, "statistics-v1", "runs", "8", "99.json"), "corrupt");
        Check(store.MaxCombatId() == 99 && store.OpenRun(Header("NEXT", 500), false).RunId == "9", "Directory and filename reservations survive corrupt contents");
        Check(diagnostics.Count >= 2, "Write and ambiguity failures must be diagnosed");
    }

    private static void LegacyHistory(string directory)
    {
        string combatDirectory = Path.Combine(directory, "runs", "42");
        Directory.CreateDirectory(combatDirectory);
        string runPath = Path.Combine(directory, "runs.jsonl"), combatPath = Path.Combine(combatDirectory, "7.json");
        const string first = """
            {"run_id":42,"profile":0,"seed":"LEGACY","started_at":100,"ended_at":200,"character":"IRONCLAD","game_mode":"standard","outcome":"victory","players":[{"slot":2,"character":"IRONCLAD"}]}
            """;
        const string second = """
            {"run_id":42,"profile":0,"seed":"LEGACY","started_at":100,"ended_at":300,"character":"DEFECT","outcome":"defeat"}
            """;
        const string combat = """
            {"combat_id":7,"started_at":110,"result":"completed","turns":2,"damage_received":3,"run":{"seq":42,"seed":"LEGACY","profile":0,"started_at":100},"cards":[{"id":"STRIKE","kind":0,"player":2,"plays":1,"damage_dealt":9,"dmg_direct":9}]}
            """;
        File.WriteAllText(runPath, first + "\n" + second + "\n");
        File.WriteAllText(combatPath, combat);
        byte[] oldHeader = File.ReadAllBytes(runPath), oldCombat = File.ReadAllBytes(combatPath);
        var store = new StatisticsStore(directory, "game-v", "mod-v", _ => { });
        var selected = store.Select(RunIdentity.Parse(0, "LEGACY", 100));
        Check(selected.Character == "IRONCLAD" && selected.Outcome == "victory" && selected.EndedAt == 200,
            "Legacy duplicate headers must use the first finalized metadata");
        Check(selected.Cards.Single().Player == 4 && selected.PlayerCards[2].Single().Player == 2 && selected.Players.Single().Slot == 2,
            "History must retain sparse roster slots and independent team/player projections");
        Check(selected.Coverage.Quality == CaptureQuality.Unknown && store.MaxCombatId() == 7, "Legacy coverage and global combat reservation must be retained");
        var resumed = store.OpenRun(Header("LEGACY", 100), true);
        Check(resumed.RunId == "42" && store.LoadRun(resumed).Summary.Cards.Single().DamageDealt == 9, "Versioned continuation must rejoin the original numeric run");
        Check(store.SaveCombat(Record(resumed, 8, 4)) && store.SaveRun(resumed with { Outcome = "defeat", EndedAt = 400 }), "Continuation writes only versioned records");
        selected = store.Select(resumed.Identity);
        Check(selected.Combats == 2 && selected.Cards.Single().DamageDealt == 13 && selected.Outcome == "victory", "Legacy first header governs the combined history");
        Check(File.ReadAllBytes(runPath).SequenceEqual(oldHeader) && File.ReadAllBytes(combatPath).SequenceEqual(oldCombat), "Legacy inputs must remain byte-for-byte unchanged");
        File.WriteAllBytes(runPath, new byte[] { 255 });
        File.Delete(Path.Combine(directory, "statistics-v1", "runs.jsonl"));
        var damagedHeaderStore = new StatisticsStore(directory, "g", "m", _ => { });
        selected = damagedHeaderStore.Select(resumed.Identity);
        Check(selected.Outcome == "" && selected.Combats == 2 && selected.Players.Count == 0,
            "Unreadable end headers must leave combat-only history selectable");
        Check(damagedHeaderStore.OpenRun(Header("BLOCKED", 1), false) == null,
            "Unreadable run headers must still block fresh numeric ID allocation");

    }

    private static void PreservedGuidHistory(string directory)
    {
        var previous = Header("GUID", 800) with { RunId = "11111111111111111111111111111111", LegacyRunId = 42, Outcome = "victory", EndedAt = 900 };
        string root = Path.Combine(directory, "statistics-v1", "runs", previous.RunId);
        Directory.CreateDirectory(root);
        string header = JsonSerializer.Serialize(previous, StatisticsJson.Options);
        string combat = JsonSerializer.Serialize(Record(previous, 1, 9) with { Run = null, Combat = Record(previous, 8, 9).Combat }, StatisticsJson.Options);
        string legacyDirectory = Path.Combine(directory, "runs", "42");
        Directory.CreateDirectory(legacyDirectory);
        const string legacy = """
            {"combat_id":7,"run":{"seq":42,"profile":0,"seed":"GUID","started_at":800},"cards":[{"id":"STRIKE","kind":0,"player":0,"damage_dealt":5,"dmg_direct":5}]}
            """;
        File.WriteAllText(Path.Combine(legacyDirectory, "7.json"), legacy);
        File.WriteAllText(Path.Combine(root, "run.json"), header);
        File.WriteAllText(Path.Combine(root, "00000001.json"), combat);
        var store = new StatisticsStore(directory, "g", "m", _ => { });
        Check(store.Select(previous.Identity).Cards.Single().DamageDealt == 14 && store.MaxCombatId() == 8, "Earlier GUID statistics and their read-only legacy observations remain readable");
        var continued = store.OpenRun(Header("GUID", 800), true);
        Check(continued.RunId == "43" && continued.PriorRunId == previous.RunId && store.LoadRun(continued).Summary.Combats == 2,
            "A GUID continuation writes into a new numeric directory while reading its prior observations");
        Check(store.SaveCombat(Record(continued, 9, 4)) && store.SaveRun(continued with { Outcome = "defeat", EndedAt = 1000 }),
            "Numeric continuation records can append independently");
        var selected = store.Select(previous.Identity);
        Check(selected.Combats == 3 && selected.Cards.Single().DamageDealt == 18 && selected.Outcome == "victory",
            "Continued GUID observations retain the first finalized history metadata");
        Check(File.ReadAllText(Path.Combine(root, "run.json")) == header && File.ReadAllText(Path.Combine(root, "00000001.json")) == combat,
            "Preserved GUID files must remain untouched");
    }

    private static void FailedIdScans(string directory)
    {
        Directory.CreateDirectory(directory);
        string legacyRuns = Path.Combine(directory, "runs");
        File.WriteAllText(legacyRuns, "obstruction");
        var store = new StatisticsStore(directory, "g", "m", _ => { });
        Check(store.MaxCombatId() == null && store.OpenRun(Header("BLOCKED", 1), false) == null,
            "Failed directory scans cannot seed IDs or start a run");
        File.Delete(legacyRuns);
        File.WriteAllText(Path.Combine(directory, "runs.jsonl"), "{\"run_id\":4294967295}");
        Check(store.OpenRun(Header("EXHAUSTED", 2), false) == null, "The final reserved run ID prevents fresh allocation");
        File.Delete(Path.Combine(directory, "runs.jsonl"));
        Directory.CreateDirectory(Path.Combine(legacyRuns, "1"));
        File.WriteAllText(Path.Combine(legacyRuns, "1", "4294967295.json"), "corrupt");
        Check(store.MaxCombatId() == uint.MaxValue, "Corrupt combat filenames still reserve the final combat ID");
    }

    private static void NativeLifecycle(string directory)
    {
        ProfilerSession.Initialize(directory, "g", "m", _ => { });
        var run = Header("SESSION", 300);
        ProfilerSession.StartRun(run, false);
        ulong first = ProfilerSession.StartCombat("ONE", "Normal");
        ObserveDamage(first, 9);
        ProfilerSession.Refresh();
        var savedView = ProfilerSession.CurrentCombat;
        Check(ProfilerSession.CurrentRun.Combats == 0 && ProfilerSession.CurrentRun.Cards.Count == 0, "Active combat must not enter the completed run view");
        Check(ProfilerSession.EndCombat(first) == 1 && ProfilerSession.EndCombat(first) == 0, "Combat completion is accepted once");
        ulong discarded = ProfilerSession.StartCombat("DISCARDED", "Normal");
        ObserveDamage(discarded, 50);
        ProfilerSession.Suspend();
        Check(!ProfilerSession.InRun && ProfilerSession.CurrentCombat == null && ProfilerNative.Snapshot() == "null", "Suspend discards its active combat");
        ProfilerSession.StartRun(run, true);
        Check(ProfilerSession.CurrentRun.Combats == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 9, "Resume reconstructs only persisted completed combats");
        ulong next = ProfilerSession.StartCombat("TWO", "Normal");
        ObserveDamage(next, 4);
        Check(next > discarded && ProfilerSession.EndCombat(discarded) == 0 && ProfilerSession.EndCombat(next) == 1, "Resumed combat ignores stale callbacks");
        Check(savedView.Cards.Single().DamageDealt == 9 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 13, "Published views remain immutable while totals advance");
        ProfilerSession.EndRun(0);
        ProfilerSession.SelectHistory("SESSION", 300, 0);
        Check(ProfilerSession.HistoryOpen && ProfilerSession.SelectedHistory.Combats == 2 && ProfilerSession.SelectedHistory.Outcome == "victory", "History selects the completed stored session");
        ProfilerSession.ClearHistory();
        Check(!ProfilerSession.HistoryOpen && ProfilerSession.SelectedHistory == null, "Closing history clears selection");
    }

    private static void FailedWriteResume(string directory)
    {
        var diagnostics = new List<string>();
        ProfilerSession.Initialize(directory, "g", "m", diagnostics.Add);
        var run = Header("FAILED", 400);
        ProfilerSession.StartRun(run, false);
        ulong first = ProfilerSession.StartCombat("ONE", "Normal");
        ObserveDamage(first, 9);
        ProfilerSession.EndCombat(first);
        string runDirectory = Path.Combine(directory, "statistics-v1", "runs", "1");
        Directory.CreateDirectory(Path.Combine(runDirectory, "2.json.tmp"));
        ulong failed = ProfilerSession.StartCombat("FAILED", "Normal");
        ObserveDamage(failed, 5);
        ProfilerSession.EndCombat(failed);
        Check(ProfilerSession.CurrentRun.Combats == 2 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 14, "Live accounting precedes persistence success");
        ProfilerSession.Suspend();
        ProfilerSession.StartRun(run, true);
        Check(ProfilerSession.CurrentRun.Combats == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 9, "Resume excludes the failed stored combat");
        Directory.Delete(Path.Combine(runDirectory, "2.json.tmp"));
        ulong next = ProfilerSession.StartCombat("NEXT", "Normal");
        ObserveDamage(next, 4);
        ProfilerSession.EndCombat(next);
        Check(!File.Exists(Path.Combine(runDirectory, "2.json")) && File.Exists(Path.Combine(runDirectory, "3.json")), "No background retry may publish the previously failed combat");
        Check(diagnostics.Count > 0, "Failed writes are reported");
        ProfilerSession.Suspend();
    }

    private static void InterruptedAndRepeatedHeaders(string directory)
    {
        ProfilerSession.Initialize(directory, "g", "m", _ => { });
        ProfilerSession.StartRun(Header("OLD", 500), false);
        ObserveDamage(ProfilerSession.StartCombat("OLD", "Normal"), 5);
        ProfilerSession.StartRun(Header("NEW", 600), false);
        ProfilerSession.StartCombat("NEW", "Normal");
        ProfilerSession.SelectHistory("OLD", 500, 0);
        Check(ProfilerSession.SelectedHistory.Outcome == "" && ProfilerSession.SelectedHistory.Combats == 1, "Interrupted prior run is visible as unfinished");
        Check(ProfilerSession.CurrentRun.Combats == 0, "A different run identity excludes interrupted prior totals");
        ProfilerSession.EndRun(1);
        ProfilerSession.SelectHistory("NEW", 600, 0);
        Check(ProfilerSession.SelectedHistory.Outcome == "defeat" && ProfilerSession.SelectedHistory.Combats == 0, "Shared reserved directory permits the new end header with no matching combats");
        ProfilerSession.Suspend();
        ProfilerSession.StartRun(Header("REPEATED", 700), false);
        ObserveDamage(ProfilerSession.StartCombat("OLD", "Normal"), 11);
        ProfilerSession.StartRun(Header("REPEATED", 700), false);
        ulong next = ProfilerSession.StartCombat("NEW", "Normal");
        Check(ProfilerSession.CurrentRun.Combats == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 11, "Same identity and reused numeric run ID merge the interrupted combat");
        ObserveDamage(next, 2);
        ProfilerSession.EndCombat(next);
        ProfilerSession.EndRun(0);
        ProfilerSession.StartRun(Header("REPEATED", 700), true);
        ProfilerSession.EndRun(1);
        ProfilerSession.SelectHistory("REPEATED", 700, 0);
        Check(ProfilerSession.SelectedHistory.Outcome == "victory" && ProfilerSession.SelectedHistory.Cards.Single().DamageDealt == 13, "A later finalization does not replace the first history header");
    }

    private static void ObserveDamage(ulong epoch, int amount)
    {
        ulong source = ProfilerNative.SourceCapture(epoch, 1, 1, "STRIKE", 0, 0, 0);
        ulong hit = ProfilerNative.DamageCalculationBegin(epoch, source, 1, 0, 99);
        Check(epoch != 0 && source != 0 && hit != 0 && ProfilerNative.DamageResultAppend(hit, amount, amount, 0, 0, 4, 0) == 1
            && ProfilerNative.DamageCalculationCommit(hit) == 1, "Fixture damage crosses the actual native ABI");
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
    private static CombatRecord Record(RunRecord run, uint id, int damage) => new()
    {
        RunId = run.RunId,
        Ordinal = id,
        Run = run,
        GameVersion = "game-v",
        ModVersion = "mod-v",
        Combat = new()
        {
            PolicyVersion = 1,
            CombatId = id,
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
        try { action(); } catch (Exception ex) when (ex is InvalidDataException or JsonException or OverflowException) { rejected = true; }
        Check(rejected, message);
    }
    private static void Check(bool condition, string message)
    {
        assertions++;
        if (!condition) throw new InvalidOperationException(message);
    }
}
