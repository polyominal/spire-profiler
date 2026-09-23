using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Serialization;

namespace SpireProfiler;

internal enum CaptureQuality { Unknown, Complete, Partial }

internal sealed record CoverageSummary
{
    public CaptureQuality Quality { get; init; }
    public ulong Failures { get; init; }
    public IReadOnlyList<string> Reasons { get; init; } = Array.Empty<string>();
    [JsonIgnore] public bool Known => Quality != CaptureQuality.Unknown;
    [JsonIgnore] public bool Complete => Quality == CaptureQuality.Complete;
    internal static readonly CoverageSummary Unknown = new();
    internal static readonly CoverageSummary Healthy = new() { Quality = CaptureQuality.Complete };

    internal CoverageSummary WithFailure(string reason) => new()
    {
        Quality = CaptureQuality.Partial,
        Failures = Failures == ulong.MaxValue ? Failures : Failures + 1,
        Reasons = Array.AsReadOnly(Reasons.Append(reason).Distinct(StringComparer.Ordinal).Take(32).ToArray())
    };

    internal CoverageSummary Merge(CoverageSummary other) => new()
    {
        Quality = Quality == CaptureQuality.Partial || other.Quality == CaptureQuality.Partial
            ? CaptureQuality.Partial : Complete && other.Complete ? CaptureQuality.Complete : CaptureQuality.Unknown,
        Failures = ulong.MaxValue - Failures < other.Failures ? ulong.MaxValue : Failures + other.Failures,
        Reasons = Array.AsReadOnly(Reasons.Concat(other.Reasons).Distinct(StringComparer.Ordinal).Take(32).ToArray())
    };
}

internal sealed record PlayerSummary(int Slot, string Character);

internal sealed record StatRow
{
    public string Id { get; init; } = "";
    public int Kind { get; init; }
    public int Player { get; init; }
    public uint Plays { get; init; }
    public long DamageDealt { get; init; }
    public long DamageBlocked { get; init; }
    public long BlockGained { get; init; }
    public long BlockEffective { get; init; }
    public long Forge { get; init; }
    public long DmgDirect { get; init; }
    public long DmgAttributed { get; init; }
    public long DmgModifier { get; init; }
    public long BlkModifier { get; init; }
    public long MitigateDebuff { get; init; }
    public long MitigateBuff { get; init; }
    public long MitigateStr { get; init; }
    public long SelfDamage { get; init; }

    internal StatRow Add(StatRow other)
    {
        checked
        {
            return this with
            {
                Plays = Plays + other.Plays,
                DamageDealt = DamageDealt + other.DamageDealt,
                DamageBlocked = DamageBlocked + other.DamageBlocked,
                BlockGained = BlockGained + other.BlockGained,
                BlockEffective = BlockEffective + other.BlockEffective,
                Forge = Forge + other.Forge,
                DmgDirect = DmgDirect + other.DmgDirect,
                DmgAttributed = DmgAttributed + other.DmgAttributed,
                DmgModifier = DmgModifier + other.DmgModifier,
                BlkModifier = BlkModifier + other.BlkModifier,
                MitigateDebuff = MitigateDebuff + other.MitigateDebuff,
                MitigateBuff = MitigateBuff + other.MitigateBuff,
                MitigateStr = MitigateStr + other.MitigateStr,
                SelfDamage = SelfDamage + other.SelfDamage
            };
        }
    }
}

internal sealed record SummaryView
{
    public int? PolicyVersion { get; init; }
    public string Title { get; init; } = "";
    public string Subtitle { get; init; } = "";
    public string Seed { get; init; } = "";
    public string Character { get; init; } = "";
    public int Ascension { get; init; } = -1;
    public string GameMode { get; init; } = "";
    public IReadOnlyList<StatRow> Cards { get; init; } = Array.Empty<StatRow>();
    public IReadOnlyDictionary<int, IReadOnlyList<StatRow>> PlayerCards { get; init; }
        = new System.Collections.ObjectModel.ReadOnlyDictionary<int, IReadOnlyList<StatRow>>(new Dictionary<int, IReadOnlyList<StatRow>>());
    public IReadOnlyList<PlayerSummary> Players { get; init; } = Array.Empty<PlayerSummary>();
    public uint Turns { get; init; }
    public uint Plays { get; init; }
    public uint Combats { get; init; }
    public uint PotionsUsed { get; init; }
    public long DamageReceived { get; init; }
    public long BlockTotal { get; init; }
    public CoverageSummary Coverage { get; init; } = CoverageSummary.Unknown;
    public long StartedAt { get; init; }
    public long EndedAt { get; init; }
    public string Outcome { get; init; } = "";

    internal SummaryView Add(SummaryView combat)
    {
        try
        {
            var rows = new Dictionary<(int Player, int Kind, string Id), StatRow>();
            var order = new List<(int Player, int Kind, string Id)>();
            foreach (var row in Cards.Concat(combat.Cards))
            {
                var key = (row.Player, row.Kind, row.Id);
                if (rows.TryGetValue(key, out var existing)) rows[key] = existing.Add(row);
                else { order.Add(key); rows.Add(key, row); }
            }
            checked
            {
                var merged = this with
                {
                    PolicyVersion = PolicyVersion ?? combat.PolicyVersion,
                    Cards = Array.AsReadOnly(order.Select(key => rows[key]).ToArray()),
                    Turns = Turns + combat.Turns,
                    Plays = Plays + combat.Plays,
                    Combats = Combats + combat.Combats,
                    PotionsUsed = PotionsUsed + combat.PotionsUsed,
                    DamageReceived = DamageReceived + combat.DamageReceived,
                    BlockTotal = BlockTotal + combat.BlockTotal,
                    Coverage = Coverage.Merge(combat.Coverage)
                };
                if (PolicyVersion.HasValue && combat.PolicyVersion.HasValue && PolicyVersion != combat.PolicyVersion)
                    merged = merged with { Coverage = merged.Coverage.WithFailure("mixed-attribution-policies") };
                StatisticsJson.CheckAggregate(merged.Cards);
                return merged;
            }
        }
        catch (OverflowException) { return this with { Coverage = Coverage.WithFailure("summary-overflow") }; }
    }
}

internal sealed record RunIdentity
{
    public int Profile { get; }
    public string Seed { get; }
    public long StartedAt { get; }
    private RunIdentity(int profile, string seed, long startedAt) => (Profile, Seed, StartedAt) = (profile, seed, startedAt);
    internal static RunIdentity Parse(int profile, string seed, long startedAt)
        => profile >= 0 && !string.IsNullOrEmpty(seed) && startedAt > 0 ? new(profile, seed, startedAt) : null;
}

internal sealed record RunRecord
{
    public int SchemaVersion { get; init; } = StatisticsStore.SchemaVersion;
    public string GameVersion { get; init; } = "";
    public string ModVersion { get; init; } = "";
    public string RunId { get; init; } = "";
    public int Profile { get; init; } = -1;
    public string Seed { get; init; } = "";
    public long StartedAt { get; init; }
    public long EndedAt { get; init; }
    public string Character { get; init; } = "";
    public int Ascension { get; init; }
    public string GameMode { get; init; } = "";
    public string Outcome { get; init; } = "active";
    public IReadOnlyList<PlayerSummary> Players { get; init; } = Array.Empty<PlayerSummary>();
    internal RunIdentity Identity => RunIdentity.Parse(Profile, Seed, StartedAt);

    internal SummaryView EmptySummary() => new()
    {
        Title = Character,
        Character = Character,
        Ascension = Ascension,
        GameMode = GameMode,
        Subtitle = $"{GameMode} · Ascension {Ascension}",
        Seed = Seed,
        Players = Players,
        StartedAt = StartedAt,
        EndedAt = EndedAt,
        Outcome = Outcome,
        Coverage = CoverageSummary.Healthy
    };
}

internal sealed record CombatStatistics
{
    public int PolicyVersion { get; init; }
    public uint CombatId { get; init; }
    public string EncounterId { get; init; } = "";
    public string EncounterType { get; init; } = "";
    public long StartedAt { get; init; }
    public string Result { get; init; } = "active";
    public uint Turns { get; init; }
    public uint Plays { get; init; }
    public uint PotionsUsed { get; init; }
    public long DamageReceived { get; init; }
    public long BlockTotal { get; init; }
    public IReadOnlyList<StatRow> Cards { get; init; } = Array.Empty<StatRow>();
    public CoverageSummary Coverage { get; init; } = CoverageSummary.Unknown;

    internal SummaryView View(IReadOnlyList<PlayerSummary> players, RunRecord run = null) => new()
    {
        PolicyVersion = PolicyVersion,
        Title = EncounterId,
        Subtitle = EncounterType,
        Character = run?.Character ?? "",
        Ascension = run?.Ascension ?? -1,
        GameMode = run?.GameMode ?? "",
        Cards = Cards,
        Players = players,
        StartedAt = StartedAt,
        Outcome = Result,
        Turns = Turns,
        Plays = Plays,
        Combats = 1,
        PotionsUsed = PotionsUsed,
        DamageReceived = DamageReceived,
        BlockTotal = BlockTotal,
        Coverage = Coverage
    };
}

internal sealed record CombatRecord
{
    public int SchemaVersion { get; init; } = StatisticsStore.SchemaVersion;
    public string GameVersion { get; init; } = "";
    public string ModVersion { get; init; } = "";
    public string RunId { get; init; } = "";
    public uint Ordinal { get; init; }
    public CombatStatistics Combat { get; init; }
}
