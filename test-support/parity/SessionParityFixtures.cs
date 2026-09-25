using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace SpireProfiler;

internal static class SessionParityFixtures
{
    internal static void Run(string referencePath, string scratchDirectory)
    {
        var expected = JsonNode.Parse(File.ReadAllText(referencePath));
        if (expected["baseline"].GetValue<string>() != "c928c477852e75ffcc35f8cd16c7ed03caad7c7f") throw new InvalidOperationException("Session baseline changed");
        if (Directory.Exists(scratchDirectory)) throw new IOException("Session oracle scratch already exists");
        Directory.CreateDirectory(scratchDirectory);
        int cases = 0;
        try
        {
            foreach (var fixture in expected["merge_cases"].AsArray())
            {
                var before = fixture["before"].Deserialize<StatRow[]>(StatisticsJson.Options);
                var incoming = fixture["incoming"].Deserialize<StatRow[]>(StatisticsJson.Options);
                var result = StatRow.MergeRows(before, incoming, fixture["team"].GetValue<bool>());
                Equal(fixture["accepted"], JsonValue.Create(result != null), fixture["name"] + ".accepted");
                Equal(fixture["after"], JsonSerializer.SerializeToNode(result ?? before, StatisticsJson.Options), fixture["name"] + ".rows");
                cases++;
            }
            ProfilerSession.Initialize(scratchDirectory, "oracle-game", "oracle-mod", _ => { });
            ProfilerSession.StartRun(Header("SESSION", 300), false);
            var lifecycle = new JsonArray(Snapshot("run_started"));
            ulong first = ProfilerSession.StartCombat("ONE", "Normal");
            Observe(first, 9);
            lifecycle.Add(Snapshot("active_first"));
            ProfilerSession.EndCombat(first);
            lifecycle.Add(Snapshot("completed_first"));
            Observe(ProfilerSession.StartCombat("DISCARDED", "Normal"), 50);
            ProfilerSession.Suspend();
            lifecycle.Add(Snapshot("suspended"));
            Equal(expected["unfinished_history"], History("SESSION", 300), "unfinished_history");
            ProfilerSession.StartRun(Header("SESSION", 300), true);
            lifecycle.Add(Snapshot("resumed"));
            ulong second = ProfilerSession.StartCombat("TWO", "Normal");
            Observe(second, 4);
            lifecycle.Add(Snapshot("active_second"));
            ProfilerSession.EndCombat(second);
            lifecycle.Add(Snapshot("completed_second"));
            ProfilerSession.EndRun(0);
            lifecycle.Add(Snapshot("ended"));
            Equal(expected["ended_history"], History("SESSION", 300), "ended_history");
            ProfilerSession.StartRun(Header("OLD", 400), false);
            Observe(ProfilerSession.StartCombat("ABORTED", "Normal"), 5);
            ProfilerSession.StartRun(Header("NEW", 500) with { Character = "DEFECT", Ascension = 1, Players = new[] { new PlayerSummary(0, "DEFECT") } }, false);
            lifecycle.Add(Snapshot("replaced_run_before_combat"));
            ProfilerSession.StartCombat("NEW_COMBAT", "Normal");
            lifecycle.Add(Snapshot("replaced_run_after_combat"));
            Equal(expected["replaced_history"], History("OLD", 400), "replaced_history");
            ProfilerSession.EndRun(1);
            Equal(expected["active_end_history"], History("NEW", 500), "active_end_history");
            Equal(expected["lifecycle"], lifecycle, "lifecycle");
            ProfilerSession.Suspend();
            ProfilerSession.StartRun(Header("BLANK", 600) with { Ascension = 0 }, false);
            ProfilerSession.EndRun(0);
            Equal(expected["blank_history"], History("BLANK", 600), "blank_history");
            ProfilerSession.StartRun(Header("SESSION", 300) with { Character = "DEFECT", Ascension = 1, Players = new[] { new PlayerSummary(0, "DEFECT") } }, true);
            ulong repeated = ProfilerSession.StartCombat("THIRD", "Normal");
            Observe(repeated, 2);
            ProfilerSession.EndCombat(repeated);
            ProfilerSession.EndRun(1);
            Equal(expected["first_header_history"], History("SESSION", 300), "first_header_history");
            ProfilerSession.StartRun(Header("REPEATED", 700), false);
            Observe(ProfilerSession.StartCombat("OLD_REPEAT", "Normal"), 11);
            ProfilerSession.StartRun(Header("REPEATED", 700), false);
            ProfilerSession.StartCombat("NEW_REPEAT", "Normal");
            Equal(expected["same_identity"], Snapshot("same_identity_interrupted"), "same_identity");
            ProfilerSession.Suspend();
            Equal(expected["same_history"], History("REPEATED", 700), "same_history");
            cases += lifecycle.Count + 8;
            Console.WriteLine($"SESSION PARITY PASS baseline=c928c477852e75ffcc35f8cd16c7ed03caad7c7f cases={cases}");
        }
        finally
        {
            ProfilerSession.Suspend();
            Directory.Delete(scratchDirectory, recursive: true);
        }
    }

    private static RunRecord Header(string seed, long start) => new()
    {
        Profile = 0,
        Seed = seed,
        StartedAt = start,
        Character = "IRONCLAD",
        Ascension = 3,
        GameMode = "Standard",
        Players = new[] { new PlayerSummary(0, "IRONCLAD") }
    };

    private static void Observe(ulong epoch, int damage)
    {
        ulong source = ProfilerNative.SourceCapture(epoch, 1, 1, "STRIKE", 0, 0, 0);
        ulong hit = ProfilerNative.DamageCalculationBegin(epoch, source, 1, 0, 99);
        if (source == 0 || hit == 0 || ProfilerNative.DamageResultAppend(hit, damage, damage, 0, 0, 4, 0) != 1
            || ProfilerNative.DamageCalculationCommit(hit) != 1 || !ProfilerSession.Refresh()) throw new InvalidOperationException("Native fixture observation failed");
    }

    private static JsonNode Snapshot(string name)
    {
        ProfilerSession.Refresh();
        var combat = ProfilerSession.CurrentCombat;
        var run = ProfilerSession.CurrentRun;
        return JsonSerializer.SerializeToNode(new
        {
            name,
            in_run = ProfilerSession.InRun,
            combat = combat == null ? null : new { cards = combat.Cards, turns = combat.Turns, result = combat.Outcome },
            run = !ProfilerSession.InRun ? null : new { cards = run.Cards, turns = run.Turns, combats = run.Combats }
        }, StatisticsJson.Options);
    }

    private static JsonNode History(string seed, long start)
    {
        ProfilerSession.SelectHistory(seed, start, 0);
        var view = ProfilerSession.SelectedHistory;
        return view == null ? null : JsonSerializer.SerializeToNode(new
        {
            character = view.Character,
            ascension = view.Ascension,
            game_mode = view.GameMode,
            outcome = view.Outcome,
            seed = view.Seed,
            started_at = view.StartedAt,
            combats = view.Combats,
            turns = view.Turns,
            damage_received = view.DamageReceived,
            cards = view.Cards,
            players = view.Players,
            player_cards = view.PlayerCards
        }, StatisticsJson.Options);
    }

    private static void Equal(JsonNode expected, JsonNode actual, string path)
    {
        if (expected is JsonObject obj && actual is JsonObject other)
        {
            if (obj.Count != other.Count) throw new InvalidOperationException(path + ": different object fields");
            foreach (var (key, value) in obj) Equal(value, other[key], path + "." + key);
        }
        else if (expected is JsonArray array && actual is JsonArray otherArray)
        {
            if (array.Count != otherArray.Count) throw new InvalidOperationException(path + $": expected {array.Count} items, found {otherArray.Count}");
            for (int i = 0; i < array.Count; i++) Equal(array[i], otherArray[i], path + "[" + i + "]");
        }
        else if (!JsonNode.DeepEquals(expected, actual)) throw new InvalidOperationException(path + ": expected " + expected + ", actual " + actual);
    }
}
