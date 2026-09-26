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
            ProfilerNative.Load(nativeLibrary);
            ParserAndAggregateContracts();
            SqliteStore(Path.Combine(scratch, "store"));
            LegacyHistory(Path.Combine(scratch, "legacy"));
            PreservedGuidHistory(Path.Combine(scratch, "guid"));
            PreservedCombatIdCollisions(Path.Combine(scratch, "collisions"));
            ImportedDamage(Path.Combine(scratch, "damaged"));
            ImportBoundaries(Path.Combine(scratch, "boundaries"));
            UnavailableStorage(Path.Combine(scratch, "unavailable"), nativeLibrary);
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            NativeLifecycle(Path.Combine(scratch, "native"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            NativeOstySacrifice(Path.Combine(scratch, "osty-sacrifice"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            IncompleteCombat(Path.Combine(scratch, "incomplete"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            NativeBlockBatches(Path.Combine(scratch, "block-batches"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            OverlappingRunContexts(Path.Combine(scratch, "overlap"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            RepeatedHeaders(Path.Combine(scratch, "headers"));
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
            ProfilerSession.Shutdown();
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
        foreach (string field in new[] { "damage_dealt", "damage_blocked", "block_gained", "forge", "dmg_direct", "dmg_attributed", "dmg_modifier", "blk_modifier", "mitigate_debuff", "mitigate_buff", "mitigate_str", "self_damage" })
        {
            var document = valid.DeepClone();
            document["cards"][0][field] = -1;
            Reject(() => StatisticsJson.ParseNative(document.ToJsonString()), "Only effective defense permits negative accounting: " + field);
        }
        var signed = valid.DeepClone();
        signed["cards"] = JsonNode.Parse("""
            [{"id":"MINIMUM","block_gained":9,"block_effective":-9223372036854775807,"self_damage":1},
             {"id":"OTHER","block_effective":2}]
            """);
        var signedCombat = StatisticsJson.ParseNative(signed.ToJsonString());
        var signedRows = ChartProjection.Rows(signedCombat.Cards);
        Check(signedRows.Count == 2 && signedRows[0].Name == "MINIMUM" && signedRows[0].Value == long.MinValue
            && signedRows[0].SegMilli[(int)ChartSegment.SelfDamage] == 500 && signedRows[1].Value == 2,
            "The full accepted signed domain must rank by magnitude without overflowing and retain drawable segment scaling");
        var signedLayout = PanelLayout.Chart(UiTab.Combat, signedRows, new(), "");
        Check(signedLayout.Body.OfType<TextCommand>().Any(text => text.Text == "-9223372036854775808")
            && ChartProjection.Detail(signedRows, 0, signedCombat.Cards).Stats.Any(stat => stat.Label == "self dmg" && stat.Value == "1"),
            "Minimum signed defense must remain displayable in chart labels and source details");
        foreach (long credit in new[] { 2_147_484L, long.MaxValue })
        {
            signed["cards"][0]["block_effective"] = credit;
            signed["cards"][0]["self_damage"] = 0;
            signed["cards"][1]["block_effective"] = 1 - credit;
            signedCombat = StatisticsJson.ParseNative(signed.ToJsonString());
            signedRows = ChartProjection.Rows(signedCombat.Cards);
            Check(signedRows.Count == 1 && signedRows[0].ShareX10 == (Int128)credit * 1000,
                "Signed defense cancellation must preserve percentages beyond the int and long domains");
            string expectedLabel = credit == long.MaxValue ? "9223372036854775807  (922337203685477580700.0%)" : "2147484  (214748400.0%)";
            Check(PanelLayout.Chart(UiTab.Combat, signedRows, new(), "").Body.OfType<TextCommand>().Any(text => text.Text == expectedLabel),
                "Large signed-defense percentages must display their actual magnitude without wrapping or clamping");
        }
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
        Reject(() => StatisticsJson.CheckRows(new[]
        {
            new StatRow { Id = "POSITIVE", BlockEffective = long.MaxValue },
            new StatRow { Id = "NEGATIVE", Player = 1, BlockEffective = -long.MaxValue },
            new StatRow { Id = "ONE", BlockEffective = 1 }
        }), "Negative defense cannot hide an overflow exposed by player filtering");
        var run = Header("SCHEMA", 100) with { RunId = "7" };
        var runJson = JsonSerializer.SerializeToNode(run, StatisticsJson.Options);
        runJson.AsObject().Remove("schema_version");
        Reject(() => StatisticsJson.ParseRun(runJson.ToJsonString(), run.RunId), "Unversioned records must not masquerade as current schema");
        foreach (string ids in new[] { "[\"../../other\"]", "[\"0\"]", "[\"7\"]", "[\"8\",\"8\"]", "null" })
        {
            var invalid = JsonSerializer.SerializeToNode(run, StatisticsJson.Options);
            invalid["preserved_run_ids"] = JsonNode.Parse(ids);
            Reject(() => StatisticsJson.ParseRun(invalid.ToJsonString(), run.RunId), "Preserved run IDs must be bounded to distinct storage identities");
        }
        var record = Record(run, 1, 9);
        var preserved = JsonSerializer.SerializeToNode(record, StatisticsJson.Options);
        preserved["schema_version"] = 1;
        preserved["run"]["schema_version"] = 1;
        preserved["run"]["legacy_run_id"] = 0;
        Check(StatisticsJson.ParseVersionOneCombat(preserved.ToJsonString(), run.RunId, 1).Combat.Cards.Single().DamageDealt == 9,
            "A zero legacy alias in old embedded metadata must not discard its accepted combat");
        Reject(() => StatisticsJson.ParseVersionOneRun(preserved["run"].ToJsonString(), run.RunId),
            "A zero legacy alias remains invalid in old standalone run headers");
        var combatJson = JsonSerializer.SerializeToNode(record, StatisticsJson.Options);
        combatJson.AsObject().Remove("schema_version");
        Reject(() => StatisticsJson.ParseCombat(combatJson.ToJsonString(), run.RunId, 1), "Combat schema identity must be explicit");
        combatJson = JsonSerializer.SerializeToNode(record, StatisticsJson.Options);
        combatJson["combat"]["coverage"]["quality"] = "partial";
        combatJson["combat"]["coverage"]["reasons"] = JsonNode.Parse("[null]");
        Reject(() => StatisticsJson.ParseCombat(combatJson.ToJsonString(), run.RunId, 1), "Stored failure reasons must be meaningful strings");
    }

    private static void SqliteStore(string directory)
    {
        string reserved = Path.Combine(directory, "statistics-v2", "runs", "8");
        Directory.CreateDirectory(reserved);
        File.WriteAllText(Path.Combine(reserved, "99.json"), "corrupt preserved record");
        var diagnostics = new List<string>();
        using var store = new StatisticsStore(directory, "game-v", "mod-v", diagnostics.Add);
        var empty = store.OpenRun(Header("EMPTY", 100), false);
        Check(empty.RunId == "9" && !store.SaveRun(empty with { Outcome = "victory" }) && store.Select(empty.Identity) == null,
            "Imported filenames reserve identities while empty runs never publish history");
        var run = store.OpenRun(Header("EXACT", 200), false);
        Check(run.RunId == "10" && store.SaveCombat(Record(run, 100, 9)), "SQLite allocates distinct run identities and accepts immutable combat records");
        Check(!store.SaveCombat(Record(run, 100, 50)) && store.LoadRun(run).Summary.Cards.Single().DamageDealt == 9,
            "A duplicate combat cannot overwrite accepted accounting");
        Check(!store.SaveRun(run with { Seed = "UNRELATED", Outcome = "victory", EndedAt = 300 }),
            "A different game identity cannot finalize the stored run");
        Check(store.SaveRun(run with { Outcome = "victory", EndedAt = 300, Players = new[] { new PlayerSummary(2, "DEFECT") } })
            && store.SaveRun(run with { Outcome = "defeat", EndedAt = 400 }), "Run finalization accepts repeated observations");
        var selected = store.Select(run.Identity);
        var continued = store.OpenRun(Header("EXACT", 200), true);
        Check(selected.Outcome == "victory" && selected.EndedAt == 300 && selected.Players.Single().Slot == 2
            && continued.RunId == run.RunId && store.LoadRun(continued).Summary.Players.Single().Slot == 0,
            "First finalized metadata governs history while resumed live views use the current roster");
        Check(store.MaxCombatId() == 100 && store.LoadRun(continued).Summary.Combats == 1, "Stored combat IDs and totals survive continuation");
        var duplicate = store.OpenRun(Header("EXACT", 200), false);
        Check(duplicate.RunId != run.RunId && store.SaveCombat(Record(duplicate, 101, 4)) && store.Select(run.Identity) == null,
            "Separate fresh recordings of one game identity remain ambiguous");
        Check(store.OpenRun(Header("EXACT", 200), true) == null && store.OpenRun(Header("HEALTHY", 500), false).RunId != "0",
            "Ambiguous continuation refuses that identity without disabling the database");
        Check(File.Exists(Path.Combine(directory, "statistics-v3", "statistics.sqlite3")) && diagnostics.Count > 0,
            "Storage uses the versioned SQLite database and reports rejected records");
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
        using (var store = new StatisticsStore(directory, "game-v", "mod-v", _ => { }))
        {
            var selected = store.Select(RunIdentity.Parse(0, "LEGACY", 100));
            Check(selected.Character == "IRONCLAD" && selected.Outcome == "victory" && selected.EndedAt == 200,
                "Legacy duplicate headers retain their first finalized metadata");
            Check(selected.Cards.Single().Player == 4 && selected.PlayerCards[2].Single().Player == 2 && selected.Players.Single().Slot == 2,
                "Imported history retains sparse roster slots and independent team/player projections");
            Check(selected.Coverage.Quality == CaptureQuality.Unknown && store.MaxCombatId() == 7,
                "Legacy coverage remains unknown and combat identities stay reserved");
            var resumed = store.OpenRun(Header("LEGACY", 100), true);
            Check(resumed.RunId == "42" && store.SaveCombat(Record(resumed, 8, 4))
                && store.SaveRun(resumed with { Outcome = "defeat", EndedAt = 400 }), "Imported runs continue in SQLite");
            Check(store.Select(resumed.Identity).Combats == 2 && store.Select(resumed.Identity).Outcome == "victory",
                "A later SQLite finalization preserves the imported first history header");
        }
        Check(File.ReadAllBytes(runPath).SequenceEqual(oldHeader) && File.ReadAllBytes(combatPath).SequenceEqual(oldCombat),
            "Import and continuation preserve old files byte for byte");
        File.WriteAllBytes(runPath, new byte[] { 255 });
        File.Delete(combatPath);
        using var reopened = new StatisticsStore(directory, "g", "m", _ => { });
        var history = reopened.Select(RunIdentity.Parse(0, "LEGACY", 100));
        Check(history.Combats == 2 && history.Cards.Single().DamageDealt == 13 && history.Outcome == "victory"
            && reopened.OpenRun(Header("NEW", 500), false).RunId != "0",
            "A completed import makes later history and allocation independent of damaged old files");
    }

    private static void PreservedGuidHistory(string directory)
    {
        var previous = Header("GUID", 800) with { RunId = "11111111111111111111111111111111", Outcome = "victory", EndedAt = 900 };
        string root = Path.Combine(directory, "statistics-v1", "runs", previous.RunId);
        Directory.CreateDirectory(root);
        var oldHeader = JsonSerializer.SerializeToNode(previous, StatisticsJson.Options);
        oldHeader["schema_version"] = 1;
        oldHeader["legacy_run_id"] = 42;
        oldHeader.AsObject().Remove("preserved_run_ids");
        string header = oldHeader.ToJsonString();
        var oldCombat = JsonSerializer.SerializeToNode(Record(previous, 1, 9) with { Run = null, Combat = Record(previous, 8, 9).Combat }, StatisticsJson.Options);
        oldCombat["schema_version"] = 1;
        string combat = oldCombat.ToJsonString();
        string legacyDirectory = Path.Combine(directory, "runs", "42");
        Directory.CreateDirectory(legacyDirectory);
        File.WriteAllText(Path.Combine(legacyDirectory, "7.json"), """
            {"combat_id":7,"run":{"seq":42,"profile":0,"seed":"GUID","started_at":800},"cards":[{"id":"STRIKE","kind":0,"player":0,"damage_dealt":5,"dmg_direct":5}]}
            """);
        File.WriteAllText(Path.Combine(root, "run.json"), header);
        File.WriteAllText(Path.Combine(root, "00000001.json"), combat);
        var numeric = previous with { RunId = "43", Outcome = "defeat", EndedAt = 950 };
        var numericHeader = JsonSerializer.SerializeToNode(numeric, StatisticsJson.Options);
        numericHeader["schema_version"] = 1;
        numericHeader["prior_run_id"] = previous.RunId;
        numericHeader["legacy_run_id"] = 42;
        numericHeader.AsObject().Remove("preserved_run_ids");
        var numericCombat = JsonSerializer.SerializeToNode(Record(numeric, 9, 4), StatisticsJson.Options);
        numericCombat["schema_version"] = 1;
        numericCombat["run"] = numericHeader.DeepClone();
        string oldNumericDirectory = Path.Combine(directory, "statistics-v1", "runs", "43");
        Directory.CreateDirectory(oldNumericDirectory);
        string numericHeaderText = numericHeader.ToJsonString() + "\n", numericCombatText = numericCombat.ToJsonString();
        File.WriteAllText(Path.Combine(directory, "statistics-v1", "runs.jsonl"), numericHeaderText);
        File.WriteAllText(Path.Combine(oldNumericDirectory, "9.json"), numericCombatText);
        var unrelated = Header("UNRELATED", 1000) with { RunId = "42", Outcome = "victory" };
        string unrelatedDirectory = Path.Combine(directory, "statistics-v2", "runs", "42");
        Directory.CreateDirectory(unrelatedDirectory);
        File.WriteAllText(Path.Combine(directory, "statistics-v2", "runs.jsonl"), JsonSerializer.Serialize(unrelated, StatisticsJson.Options));
        File.WriteAllText(Path.Combine(unrelatedDirectory, "11.json"), "unrelated corrupt record");
        using var store = new StatisticsStore(directory, "g", "m", _ => { });
        var continued = store.OpenRun(Header("GUID", 800), true);
        Check(continued.RunId == "43" && continued.PreservedRunIds.Order(StringComparer.Ordinal).SequenceEqual(new[] { previous.RunId, "42" })
            && store.LoadRun(continued).Summary.Combats == 3, "Import flattens GUID and legacy aliases while preserving a numeric continuation");
        Check(store.SaveCombat(Record(continued, 12, 2)) && store.SaveRun(continued with { Outcome = "defeat", EndedAt = 1100 }),
            "Imported alias chains accept new SQLite records");
        var selected = store.Select(previous.Identity);
        Check(selected.Combats == 4 && selected.Cards.Single().DamageDealt == 20 && selected.Outcome == "victory"
            && selected.Coverage.Quality == CaptureQuality.Unknown,
            "Imported history keeps first finalization and is not tainted by an unrelated version reusing its alias");
        Check(store.Select(unrelated.Identity).Coverage.Quality == CaptureQuality.Partial,
            "Unreadable imported records retain loss evidence under their own identity");
        Check(File.ReadAllText(Path.Combine(root, "run.json")) == header && File.ReadAllText(Path.Combine(root, "00000001.json")) == combat
            && File.ReadAllText(Path.Combine(directory, "statistics-v1", "runs.jsonl")) == numericHeaderText
            && File.ReadAllText(Path.Combine(oldNumericDirectory, "9.json")) == numericCombatText,
            "All historical inputs remain untouched");
    }

    private static void ImportedDamage(string directory)
    {
        foreach (string damage in new[] { "malformed", "identity", "null-metadata", "negative-block", "headers", "folder" })
        {
            string root = Path.Combine(directory, damage);
            var run = Header("DAMAGED", 2200) with { RunId = "1", Outcome = "victory" };
            var healthy = Header("HEALTHY", 2300) with { RunId = "2" };
            string damagedDirectory = Path.Combine(root, "statistics-v2", "runs", "1");
            string healthyDirectory = Path.Combine(root, "statistics-v1", "runs", "2");
            Directory.CreateDirectory(damagedDirectory);
            Directory.CreateDirectory(healthyDirectory);
            File.WriteAllText(Path.Combine(root, "statistics-v2", "runs.jsonl"), JsonSerializer.Serialize(run, StatisticsJson.Options));
            File.WriteAllText(Path.Combine(damagedDirectory, "1.json"), JsonSerializer.Serialize(Record(run, 1, 9), StatisticsJson.Options));
            var second = JsonSerializer.SerializeToNode(Record(run, 4, 5), StatisticsJson.Options);
            if (damage == "identity") second["run"]["seed"] = "REPLACED";
            if (damage == "null-metadata") second["run"]["character"] = null;
            if (damage == "negative-block") second["combat"]["block_total"] = -1;
            File.WriteAllText(Path.Combine(damagedDirectory, "4.json"), damage == "malformed" ? "{truncated" : second.ToJsonString());
            if (damage == "headers") File.WriteAllBytes(Path.Combine(root, "statistics-v2", "runs.jsonl"), new byte[] { 255 });
            if (damage == "folder")
            {
                Directory.Delete(damagedDirectory, recursive: true);
                File.WriteAllText(damagedDirectory, "obstruction");
            }
            var oldHealthy = JsonSerializer.SerializeToNode(Record(healthy, 2, 7), StatisticsJson.Options);
            oldHealthy["schema_version"] = 1;
            oldHealthy["run"]["schema_version"] = 1;
            File.WriteAllText(Path.Combine(healthyDirectory, "2.json"), oldHealthy.ToJsonString());
            using var store = new StatisticsStore(root, "g", "m", _ => { });
            var selected = store.Select(run.Identity);
            uint count = damage == "folder" ? 0u : damage == "headers" ? 2u : 1u;
            Check(selected?.Combats == count && selected.Coverage.Quality == CaptureQuality.Partial,
                "Import retains surviving totals and marks damaged history partial: " + damage);
            Check(store.Select(healthy.Identity)?.Coverage.Complete == true && store.OpenRun(Header("FRESH", 2500), false).RunId != "0",
                "Damaged old files do not contaminate other runs or disable fresh capture: " + damage);
            if (damage == "identity") Check(store.Select(RunIdentity.Parse(0, "REPLACED", 2200)) == null,
                "A record contradicting its finalized header cannot create another run");
        }
    }

    private static void ImportBoundaries(string directory)
    {
        string root = Path.Combine(directory, "statistics-v2"), preserved = Path.Combine(directory, "statistics-v1");
        Directory.CreateDirectory(root);
        Directory.CreateDirectory(preserved);
        var ambiguous = Header("AMBIGUOUS", 3000) with { Outcome = "victory" };
        var cycle = Header("CYCLE", 3100) with { Outcome = "victory" };
        var lines = new List<string>();
        foreach (int id in new[] { 1, 2 })
        {
            lines.Add(JsonSerializer.Serialize(ambiguous with { RunId = id.ToString(CultureInfo.InvariantCulture) }, StatisticsJson.Options));
            var header = JsonSerializer.SerializeToNode(cycle with { RunId = (id + 2).ToString(CultureInfo.InvariantCulture) }, StatisticsJson.Options);
            header["schema_version"] = 1;
            header["prior_run_id"] = (5 - id).ToString(CultureInfo.InvariantCulture);
            File.AppendAllText(Path.Combine(preserved, "runs.jsonl"), header.ToJsonString() + "\n");
        }
        File.WriteAllLines(Path.Combine(root, "runs.jsonl"), lines);
        var wrong = Header("WRONG-VERSION", 3200) with { RunId = "9", Outcome = "victory" };
        var wrongRecord = JsonSerializer.SerializeToNode(Record(wrong, 99, 5), StatisticsJson.Options);
        wrongRecord["schema_version"] = 1;
        Directory.CreateDirectory(Path.Combine(root, "runs", "9"));
        File.WriteAllText(Path.Combine(root, "runs", "9", "99.json"), wrongRecord.ToJsonString());
        using var store = new StatisticsStore(directory, "g", "m", _ => { });
        Check(store.Select(ambiguous.Identity) == null && store.Select(cycle.Identity) == null && store.Select(wrong.Identity) == null,
            "Import refuses ambiguous aliases, cycles, and records in the wrong schema directory");
        Check(store.MaxCombatId() == 99 && store.OpenRun(Header("AFTER", 3300), false).RunId == "10",
            "Rejected old records reserve numeric directory and filename identities");
        Check(store.OpenRun(ambiguous, true) == null && store.OpenRun(cycle, true) == null,
            "Imported ambiguity remains a refusal after the original files are no longer consulted");
    }

    private static void UnavailableStorage(string directory, string nativeLibrary)
    {
        foreach (string failure in new[] { "directory", "database" })
        {
            ProfilerNative.Dispose();
            ProfilerNative.Load(nativeLibrary);
            string root = Path.Combine(directory, failure), version = Path.Combine(root, "statistics-v3");
            Directory.CreateDirectory(root);
            if (failure == "directory") File.WriteAllText(version, "obstruction");
            else
            {
                Directory.CreateDirectory(version);
                File.WriteAllText(Path.Combine(version, "statistics.sqlite3"), "not a database");
            }
            var diagnostics = new List<string>();
            ProfilerSession.Initialize(root, "g", "m", diagnostics.Add);
            var run = Header("OFFLINE", 3400);
            ProfilerSession.StartRun(run, false);
            ulong epoch = ProfilerSession.StartCombat("OFFLINE", "Normal");
            ObserveDamage(epoch, 9);
            Check(ProfilerSession.EndCombat(epoch) == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 9
                && ProfilerSession.CurrentRun.Coverage.Quality == CaptureQuality.Partial,
                "Storage initialization failure preserves live attribution and reports unsaved statistics: " + failure);
            if (failure == "directory") File.Delete(version);
            else File.Delete(Path.Combine(version, "statistics.sqlite3"));
            epoch = ProfilerSession.StartCombat("STILL-OFFLINE", "Normal");
            ObserveDamage(epoch, 4);
            ProfilerSession.EndCombat(epoch);
            ProfilerSession.EndRun(0);
            ProfilerSession.SelectHistory(run.Seed, run.StartedAt, run.Profile);
            Check(ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 13 && ProfilerSession.SelectedHistory == null
                && !File.Exists(Path.Combine(version, "statistics.sqlite3")) && diagnostics.Count > 0,
                "An unavailable store never reconnects and persists ephemeral identities during its lifetime");
            ProfilerSession.Shutdown();
        }
    }

    private static void PreservedCombatIdCollisions(string directory)
    {
        RunRecord target = null;
        for (int index = 0; index <= 32; index++)
        {
            var run = Header("COLLISION-" + index, 2000 + index) with
            {
                RunId = "a" + index.ToString("x31", CultureInfo.InvariantCulture),
                Outcome = "victory"
            };
            if (index == 0) target = run;
            string root = Path.Combine(directory, "statistics-v1", "runs", run.RunId);
            Directory.CreateDirectory(root);
            var header = JsonSerializer.SerializeToNode(run, StatisticsJson.Options);
            header["schema_version"] = 1;
            File.WriteAllText(Path.Combine(root, "run.json"), header.ToJsonString());
            for (uint ordinal = 1; ordinal <= (index == 0 ? 17 : 1); ordinal++)
            {
                var record = Record(run, ordinal, 1) with
                {
                    Run = null,
                    Combat = Record(run, 1, 1).Combat with
                    {
                        Cards = new[] { new StatRow { Id = "SOURCE-" + ordinal, DamageDealt = 1, DmgDirect = 1 } }
                    }
                };
                var node = JsonSerializer.SerializeToNode(record, StatisticsJson.Options);
                node["schema_version"] = 1;
                File.WriteAllText(Path.Combine(root, ordinal.ToString("D8", CultureInfo.InvariantCulture) + ".json"), node.ToJsonString());
            }
        }
        using var store = new StatisticsStore(directory, "g", "m", _ => { });
        var history = store.Select(target.Identity);
        var resumed = store.LoadRun(store.OpenRun(target, true));
        Check(history.Combats == 17 && resumed.Summary.Combats == 17 && resumed.LastOrdinal == 17,
            "Preserved GUID ordinals remain separate even when native combat IDs were reused");
        Check(resumed.Summary.Cards.Select(row => row.Id).SequenceEqual(history.Cards.Select(row => row.Id)),
            "A resumed run with colliding preserved IDs must retain the archive's history tie ordering");
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
        Check(ProfilerSession.CurrentRun.Combats == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 9
            && ProfilerSession.CurrentRun.Coverage.Complete, "Resume reconstructs only persisted completed combats without treating an aborted attempt as lost capture");
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

    private static void NativeBlockBatches(string directory)
    {
        ProfilerSession.Initialize(directory, "g", "m", _ => { });
        ProfilerSession.StartRun(Header("BLOCK_BATCHES", 1400), false);
        Check(ProfilerNative.RecordingBegin(), "Atomic block fixture recording begins before combat");
        ulong epoch = ProfilerSession.StartCombat("BLOCK_BATCHES", "Normal");
        ulong producer = ProfilerNative.SourceCapture(epoch, 1, 1, "PRODUCER", 0, 0, 0);
        var modifiers = Enumerable.Range(0, 16).Select(index => new BlockModifier(
            ProfilerNative.SourceCapture(epoch, 1, (ulong)index + 2, "MOD" + index, 0, 1, 0), 1)).ToArray();
        Check(ProfilerNative.BlockGained(epoch, 16, producer, 0, modifiers, false) == 1,
            "All sixteen struct entries cross the real managed/native array boundary");
        foreach (int amount in new[] { 1, 15 })
        {
            Check(ProfilerNative.DamageUnattributed(epoch, amount, 0, amount, 1, 0, 0) == 1 && ProfilerSession.Refresh(),
                "Partial block consumption crosses the native boundary");
            Check(ProfilerSession.CurrentCombat.Cards.Sum(row => row.BlockEffective + row.BlkModifier) == (amount == 1 ? 1 : 16),
                "Marshaled modifier batches conserve each physical block prefix");
        }
        var creditedModifiers = ProfilerSession.CurrentCombat.Cards.Where(row => row.Id.StartsWith("MOD", StringComparison.Ordinal)).ToArray();
        Check(creditedModifiers.Length == 16 && creditedModifiers.All(row => row.BlkModifier == 1)
            && ProfilerSession.CurrentCombat.Cards.Single(row => row.Id == "PRODUCER").BlockGained == 16,
            "Marshaled source handles and credits retain every modifier and the gross producer gain");
        Check(ProfilerNative.BlockGained(epoch, 5, producer, 0, modifiers, true) == 1
            && ProfilerNative.DamageUnattributed(epoch, 5, 0, 5, 1, 0, 0) == 1 && ProfilerSession.Refresh(),
            "The incomplete flag preserves the physical gain across the same array boundary");
        Check(ProfilerSession.CurrentCombat.Cards.Single(row => row.Id == "UNATTRIBUTED").BlockEffective == 5
            && !ProfilerSession.CurrentCombat.Coverage.Complete, "Incomplete modifier attribution degrades to Unknown");
        Check(ProfilerNative.Replay(ProfilerNative.Recording()) == ProfilerNative.Snapshot(),
            "Actual marshaled modifier batches replay exactly");
        ProfilerSession.Suspend();
    }

    private static void NativeOstySacrifice(string directory)
    {
        var messages = new List<string>();
        var run = Header("OSTY_SACRIFICE", 1350);
        ProfilerSession.Initialize(directory, "g", "m", messages.Add);
        ProfilerSession.StartRun(run, false);
        ulong epoch = ProfilerSession.StartCombat("BONE_SHARDS", "Normal");
        ulong summon = ProfilerNative.SourceCapture(epoch, 1, 100, "SUMMON", 0, 0, 0);
        ulong shards = ProfilerNative.SourceCapture(epoch, 1, 101, "BONE_SHARDS", 0, 0, 0);
        ulong play = ProfilerNative.CardPlayStarted(epoch, 1, 101, "BONE_SHARDS", 0, 0, 1, 0, shards);
        Check(play != 0 && ProfilerNative.OstySummoned(epoch, summon, 5, 0) == 1
            && ProfilerNative.BlockGained(epoch, 9, shards, 0, Array.Empty<BlockModifier>(), false) == 1
            && ProfilerNative.OstyKilled(epoch, 0, play) == 1,
            "Bone Shards gains block then sacrifices Osty's remaining five HP through the actual ABI");
        ObserveDamage(epoch, 9);
        Check(ProfilerSession.Refresh(), "A valid negative Osty credit must not reject the whole native snapshot");
        var snapshot = ProfilerSession.CurrentCombat;
        Check(snapshot.Cards.Single(row => row.Id == "BONE_SHARDS").BlockEffective == -5
            && snapshot.Cards.Single(row => row.Id == "BONE_SHARDS").BlockGained == 9
            && snapshot.Cards.Single(row => row.Id == "STRIKE").DamageDealt == 9 && snapshot.Coverage.Complete,
            "Signed sacrifice credit, gross block, and unrelated damage must survive native parsing together");
        Check(ProfilerNative.CardPlayFinished(play) == 1 && ProfilerNative.CardExecutionEnded(epoch, 1) == 1
            && ProfilerSession.EndCombat(epoch) == 1 && ProfilerSession.CurrentRun.Combats == 1
            && ProfilerSession.CurrentRun.Cards.Single(row => row.Id == "BONE_SHARDS").BlockEffective == -5,
            "The completed run must retain the sacrifice instead of an empty interrupted combat");
        ProfilerSession.EndRun(0);
        ProfilerSession.Suspend();
        ProfilerSession.StartRun(run, true);
        Check(ProfilerSession.CurrentRun.Cards.Single(row => row.Id == "STRIKE").DamageDealt == 9
            && ProfilerSession.CurrentRun.Cards.Single(row => row.Id == "BONE_SHARDS").BlockEffective == -5
            && ProfilerSession.CurrentRun.Coverage.Complete,
            "Stored signed combat accounting must survive a fresh session's continuation");
        ProfilerSession.SelectHistory(run.Seed, run.StartedAt, run.Profile);
        Check(ProfilerSession.SelectedHistory.Cards.Single(row => row.Id == "BONE_SHARDS").BlockEffective == -5
            && ProfilerSession.SelectedHistory.Coverage.Complete && messages.Count == 0,
            "History must read signed defense rows without dropping the combat or marking healthy capture partial");
        ProfilerSession.Suspend();
    }

    private static void OverlappingRunContexts(string directory)
    {
        ProfilerSession.Initialize(directory, "g", "m", _ => { });
        var oldRun = Header("OLD", 1500);
        var newRun = Header("NEW", 1600);
        ProfilerSession.StartRun(oldRun, false);
        ulong oldEpoch = ProfilerSession.StartCombat("OLD_COMBAT", "Normal");
        ObserveDamage(oldEpoch, 9);
        ProfilerSession.StartRun(newRun, false);
        ProfilerSession.StartCombat("NEW_COMBAT", "Normal");
        Check(ProfilerSession.CurrentRun.Combats == 0 && ProfilerSession.CurrentRun.Cards.Count == 0,
            "Finishing an older interrupted combat must not enter the replacement run accumulator");
        ProfilerSession.EndRun(0);
        ProfilerSession.SelectHistory(newRun.Seed, newRun.StartedAt, newRun.Profile);
        Check(ProfilerSession.SelectedHistory == null,
            "A replacement run without its own completed combat must not publish empty history from an older combat");
        ProfilerSession.SelectHistory(oldRun.Seed, oldRun.StartedAt, oldRun.Profile);
        Check(ProfilerSession.SelectedHistory?.Combats == 1 && ProfilerSession.SelectedHistory.Cards.Single().DamageDealt == 9,
            "The older interrupted combat must remain available under its own run identity");
        ProfilerSession.Suspend();
        var repeated = Header("REPEATED-FRESH", 1700);
        ProfilerSession.StartRun(repeated, false);
        oldEpoch = ProfilerSession.StartCombat("FIRST_ATTEMPT", "Normal");
        ObserveDamage(oldEpoch, 11);
        ProfilerSession.StartRun(repeated, false);
        ulong nextEpoch = ProfilerSession.StartCombat("SECOND_ATTEMPT", "Normal");
        ObserveDamage(nextEpoch, 4);
        ProfilerSession.EndCombat(nextEpoch);
        Check(ProfilerSession.CurrentRun.Combats == 1 && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 4,
            "Two explicitly fresh attempts with the same game identity must not share an active-context ID");
        ProfilerSession.EndRun(0);
        ProfilerSession.SelectHistory(repeated.Seed, repeated.StartedAt, repeated.Profile);
        Check(ProfilerSession.SelectedHistory == null,
            "Separately recorded fresh attempts with the same game identity remain ambiguous in history");
    }

    private static void IncompleteCombat(string directory)
    {
        var run = Header("INCOMPLETE", 1450);
        ProfilerSession.Initialize(directory, "g", "m", _ => { });
        ProfilerSession.StartRun(run, false);
        ulong epoch = ProfilerSession.StartCombat("MISSING", "Normal");
        ProfilerSession.ReportFailure("snapshot-read-failed");
        Check(ProfilerSession.EndCombat(epoch) == 1 && ProfilerSession.CurrentCombat.Cards.Count == 0
            && ChartProjection.Meta(ProfilerSession.CurrentCombat, UiTab.Combat).Quality == CaptureQuality.Partial,
            "Zero recorded activity must retain the capture failure for combat presentation");
        epoch = ProfilerSession.StartCombat("HEALTHY", "Normal");
        ObserveDamage(epoch, 9);
        Check(ProfilerSession.EndCombat(epoch) == 1 && ProfilerSession.CurrentCombat.Coverage.Complete
            && ProfilerSession.CurrentRun.Cards.Single().DamageDealt == 9 && ProfilerSession.CurrentRun.Combats == 2
            && ChartProjection.Meta(ProfilerSession.CurrentRun, UiTab.Run).Quality == CaptureQuality.Partial,
            "A healthy later combat cannot erase a run's earlier missing activity");
        ProfilerSession.EndRun(0);
        ProfilerSession.Suspend();
        ProfilerSession.StartRun(run, true);
        Check(ChartProjection.Meta(ProfilerSession.CurrentRun, UiTab.Run).Quality == CaptureQuality.Partial,
            "Continuation must restore the incomplete run warning from stored coverage");
        ProfilerSession.SelectHistory(run.Seed, run.StartedAt, run.Profile);
        Check(ChartProjection.Meta(ProfilerSession.SelectedHistory, UiTab.Run).Quality == CaptureQuality.Partial
            && ProfilerSession.SelectedHistory.Coverage.Reasons.Contains("snapshot-read-failed"),
            "History must warn about the same incomplete combat while retaining available totals");
        ProfilerSession.Suspend();
    }

    private static void RepeatedHeaders(string directory)
    {
        ProfilerSession.Initialize(directory, "g", "m", _ => { });
        var run = Header("CONTINUED", 700);
        ProfilerSession.StartRun(run, false);
        ulong first = ProfilerSession.StartCombat("FIRST", "Normal");
        ObserveDamage(first, 9);
        ProfilerSession.EndCombat(first);
        ProfilerSession.EndRun(0);
        ProfilerSession.StartRun(run, true);
        ulong next = ProfilerSession.StartCombat("NEXT", "Normal");
        ObserveDamage(next, 4);
        ProfilerSession.EndCombat(next);
        ProfilerSession.EndRun(1);
        ProfilerSession.SelectHistory(run.Seed, run.StartedAt, run.Profile);
        Check(ProfilerSession.SelectedHistory.Outcome == "victory" && ProfilerSession.SelectedHistory.Cards.Single().DamageDealt == 13,
            "A later finalization of a continued run does not replace the first history header");
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
