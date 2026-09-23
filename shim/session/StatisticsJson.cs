using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace SpireProfiler;

internal static class StatisticsJson
{
    internal static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };

    internal static CombatStatistics ParseNative(string json)
    {
        try
        {
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            if (root.ValueKind == JsonValueKind.Null) return null;
            var rows = JsonSerializer.Deserialize<StatRow[]>(root.GetProperty("cards"), Options)
                ?? throw new InvalidDataException("Combat rows are missing");
            CheckRows(rows);
            var result = root.GetProperty("result").GetString();
            if (result is not ("active" or "completed" or "defeat" or "interrupted"))
                throw new InvalidDataException("Invalid combat result");
            var coverage = root.GetProperty("coverage");
            bool complete = coverage.GetProperty("complete").GetBoolean();
            ulong failures = coverage.GetProperty("failures").GetUInt64();
            var reasons = JsonSerializer.Deserialize<string[]>(coverage.GetProperty("reasons"), Options) ?? Array.Empty<string>();
            if (complete && (failures != 0 || reasons.Length != 0) || reasons.Length > 32 || reasons.Any(string.IsNullOrEmpty))
                throw new InvalidDataException("Invalid capture coverage");
            uint id = root.GetProperty("combat_id").GetUInt32();
            int policy = root.GetProperty("policy_version").GetInt32();
            long received = root.GetProperty("damage_received").GetInt64();
            long blockTotal = root.GetProperty("block_total").GetInt64();
            long startedAt = root.GetProperty("started_at").GetInt64();
            if (id == 0 || policy <= 0 || received < 0 || blockTotal < 0 || startedAt < 0) throw new InvalidDataException("Invalid combat identity or totals");
            return new()
            {
                PolicyVersion = policy,
                CombatId = id,
                EncounterId = root.GetProperty("encounter_id").GetString() ?? "",
                EncounterType = root.GetProperty("encounter_type").GetString() ?? "",
                StartedAt = startedAt,
                Result = result,
                Turns = root.GetProperty("turns").GetUInt32(),
                Plays = root.GetProperty("plays").GetUInt32(),
                PotionsUsed = root.GetProperty("potions_used").GetUInt32(),
                DamageReceived = received,
                BlockTotal = blockTotal,
                Cards = Array.AsReadOnly(rows),
                Coverage = new()
                {
                    Quality = complete ? CaptureQuality.Complete : CaptureQuality.Partial,
                    Failures = failures,
                    Reasons = Array.AsReadOnly(reasons)
                }
            };
        }
        catch (Exception error) when (error is KeyNotFoundException or InvalidOperationException or FormatException)
        {
            throw new InvalidDataException("Malformed native attribution snapshot", error);
        }
    }

    internal static void CheckRows(IReadOnlyList<StatRow> rows)
    {
        if (rows == null) throw new InvalidDataException("Combat rows are missing");
        var keys = new HashSet<(int, int, string)>();
        foreach (var row in rows)
        {
            if (row == null || string.IsNullOrEmpty(row.Id) || row.Kind is < 0 or > 5 || row.Player is < 0 or > 4
                || !keys.Add((row.Player, row.Kind, row.Id))) throw new InvalidDataException("Invalid source identity");
            if (row.DamageDealt < 0 || row.DamageBlocked < 0 || row.DamageBlocked > row.DamageDealt
                || row.BlockGained < 0 || row.BlockEffective < 0 || row.Forge < 0 || row.DmgDirect < 0
                || row.DmgAttributed < 0 || row.DmgModifier < 0 || row.BlkModifier < 0 || row.MitigateDebuff < 0
                || row.MitigateBuff < 0 || row.MitigateStr < 0 || row.SelfDamage < 0
                || checked(row.DmgDirect + row.DmgAttributed + row.DmgModifier) != row.DamageDealt)
                throw new InvalidDataException("Invalid source accounting");
        }
        CheckAggregate(rows);
    }

    internal static void CheckAggregate(IReadOnlyList<StatRow> rows)
    {
        checked
        {
            long damage = 0, defense = 0, self = 0, forge = 0, blockGained = 0, plays = 0;
            foreach (var row in rows)
            {
                damage += row.DamageDealt;
                defense += row.BlockEffective + row.BlkModifier + row.MitigateDebuff + row.MitigateBuff + row.MitigateStr;
                self += row.SelfDamage;
                forge += row.Forge;
                blockGained += row.BlockGained;
                plays += row.Plays;
            }
        }
    }

    internal static RunRecord ParseRun(string json, string expectedId)
    {
        using var document = JsonDocument.Parse(json);
        if (document.RootElement.ValueKind != JsonValueKind.Object || !document.RootElement.TryGetProperty("schema_version", out var schema)
            || schema.ValueKind != JsonValueKind.Number || !schema.TryGetInt32(out int schemaVersion) || schemaVersion != StatisticsStore.SchemaVersion)
            throw new InvalidDataException("Missing or unsupported statistics schema");
        var run = JsonSerializer.Deserialize<RunRecord>(document.RootElement, Options)
            ?? throw new InvalidDataException("Run record is missing");
        if (run.SchemaVersion != StatisticsStore.SchemaVersion || run.RunId != expectedId
            || !(uint.TryParse(run.RunId, System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out _) || Guid.TryParseExact(run.RunId, "N", out _)) || run.StartedAt < 0 || run.EndedAt < 0
            || run.Profile < -1 || run.Seed == null || run.GameVersion == null || run.ModVersion == null || run.LegacyRunId == 0
            || run.Outcome is not ("active" or "suspended" or "victory" or "defeat" or "abandoned")
            || run.Players == null || run.Players.Count > 4)
            throw new InvalidDataException("Invalid run header");
        var slots = new HashSet<int>();
        foreach (var player in run.Players)
            if (player == null || player.Slot is < 0 or > 3 || string.IsNullOrEmpty(player.Character) || !slots.Add(player.Slot))
                throw new InvalidDataException("Invalid run roster");
        return run with { Players = Array.AsReadOnly(run.Players.ToArray()) };
    }

    internal static CombatRecord ParseCombat(string json, string runId, uint ordinal)
    {
        using var document = JsonDocument.Parse(json);
        if (document.RootElement.ValueKind != JsonValueKind.Object || !document.RootElement.TryGetProperty("schema_version", out var schema)
            || schema.ValueKind != JsonValueKind.Number || !schema.TryGetInt32(out int schemaVersion) || schemaVersion != StatisticsStore.SchemaVersion)
            throw new InvalidDataException("Missing or unsupported statistics schema");
        var record = JsonSerializer.Deserialize<CombatRecord>(document.RootElement, Options)
            ?? throw new InvalidDataException("Combat record is missing");
        var combat = record.Combat;
        if (record.SchemaVersion != StatisticsStore.SchemaVersion || record.RunId != runId || record.Ordinal != ordinal
            || combat == null || uint.TryParse(runId, out _) && combat.CombatId != ordinal || combat.CombatId == 0 || combat.PolicyVersion <= 0 || combat.StartedAt < 0
            || combat.DamageReceived < 0 || combat.Cards == null || combat.Coverage == null
            || combat.Coverage.Quality is not (CaptureQuality.Unknown or CaptureQuality.Complete or CaptureQuality.Partial)
            || combat.Coverage.Reasons == null || combat.Coverage.Reasons.Count > 32
            || combat.Coverage.Reasons.Any(string.IsNullOrEmpty)
            || combat.Coverage.Complete && (combat.Coverage.Failures != 0 || combat.Coverage.Reasons.Count != 0)
            || combat.Result is not ("completed" or "defeat" or "interrupted"))
            throw new InvalidDataException("Invalid stored combat");
        CheckRows(combat.Cards);
        return record with
        {
            Combat = combat with
            {
                Cards = Array.AsReadOnly(combat.Cards.ToArray()),
                Coverage = combat.Coverage with { Reasons = Array.AsReadOnly(combat.Coverage.Reasons.ToArray()) }
            }
        };
    }
}
