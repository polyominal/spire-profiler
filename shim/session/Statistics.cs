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
    internal const int RunRowLimit = 1024;
    private const int UnknownRowReserve = 5;
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

    internal static IReadOnlyList<StatRow> MergeRows(IReadOnlyList<StatRow> existing, IEnumerable<StatRow> incoming, bool team = false)
    {
        var rows = existing.ToList();
        int ordinary = rows.Count(row => row.Kind != 5);
        foreach (var row in incoming)
        {
            int index = rows.FindIndex(value => value.Id == row.Id && value.Kind == row.Kind
                && (value.Player == row.Player || team && row.Kind != 5));
            if (index >= 0)
            {
                try { rows[index] = rows[index].Add(row); continue; }
                catch (OverflowException) { }
            }
            else if ((row.Kind == 5 || ordinary < RunRowLimit - UnknownRowReserve) && rows.Count < RunRowLimit)
            {
                rows.Add(row);
                if (row.Kind != 5) ordinary++;
                continue;
            }
            int slot = Math.Clamp(row.Player, 0, 4);
            index = rows.FindIndex(value => value.Kind == 5 && value.Player == slot);
            if (index >= 0)
            {
                try { rows[index] = rows[index].Add(row); }
                catch (OverflowException) { return null; }
            }
            else if (rows.Count < RunRowLimit)
                rows.Add(row with { Id = "UNATTRIBUTED", Kind = 5, Player = slot });
            else return null;
        }
        try { CheckRepresentable(rows); }
        catch (OverflowException) { return null; }
        return Array.AsReadOnly(rows.ToArray());
    }

    internal static void CheckRepresentable(IReadOnlyList<StatRow> rows)
    {
        Span<Int128> positive = stackalloc Int128[15];
        Span<Int128> negative = stackalloc Int128[15];
        Span<Int128> values = stackalloc Int128[15];
        positive.Clear();
        negative.Clear();
        foreach (var row in rows)
        {
            long damage, defense;
            checked
            {
                damage = row.DmgDirect + row.DmgAttributed + row.DmgModifier;
                defense = row.BlockEffective + row.BlkModifier + row.MitigateDebuff + row.MitigateBuff + row.MitigateStr;
                _ = defense - row.SelfDamage;
            }
            values[0] = row.DamageDealt;
            values[1] = row.DamageBlocked;
            values[2] = row.BlockGained;
            values[3] = row.BlockEffective;
            values[4] = row.DmgDirect;
            values[5] = row.DmgAttributed;
            values[6] = row.DmgModifier;
            values[7] = row.BlkModifier;
            values[8] = row.MitigateDebuff;
            values[9] = row.MitigateBuff;
            values[10] = row.MitigateStr;
            values[11] = row.SelfDamage;
            values[12] = row.Forge;
            values[13] = damage;
            values[14] = defense;
            for (int i = 0; i < values.Length; i++)
            {
                positive[i] += Int128.Max(values[i], 0);
                negative[i] += Int128.Min(values[i], 0);
                if (positive[i] > long.MaxValue || negative[i] < long.MinValue) throw new OverflowException("Run totals exceed the ledger domain");
            }
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
        var rows = StatRow.MergeRows(Cards, combat.Cards);
        if (rows == null) return RejectCombat(combat.Coverage);
        try
        {
            // Run chart plays come from rows, independently of observed combat plays.
            _ = CheckedRowPlays(rows);
            checked
            {
                var merged = this with
                {
                    PolicyVersion = PolicyVersion ?? combat.PolicyVersion,
                    Cards = rows,
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
                return merged;
            }
        }
        catch (OverflowException) { return RejectCombat(combat.Coverage); }
    }

    internal SummaryView AddHistory(CombatStatistics combat)
    {
        int? policy = combat.PolicyVersion > 0 ? combat.PolicyVersion : null;
        var team = StatRow.MergeRows(Cards, combat.Cards.Select(row => row with { Player = 4 }), team: true);
        if (team == null) return RejectCombat(combat.Coverage);
        var players = new Dictionary<int, IReadOnlyList<StatRow>>();
        foreach (var player in Players)
        {
            var prior = PlayerCards.TryGetValue(player.Slot, out var rows) ? rows : Array.Empty<StatRow>();
            var merged = StatRow.MergeRows(prior, combat.Cards.Where(row => row.Player == player.Slot), team: true);
            if (merged == null) return RejectCombat(combat.Coverage);
            players[player.Slot] = merged;
        }
        try
        {
            uint plays = CheckedRowPlays(team);
            checked
            {
                var merged = this with
                {
                    PolicyVersion = PolicyVersion ?? policy,
                    Cards = team,
                    PlayerCards = new System.Collections.ObjectModel.ReadOnlyDictionary<int, IReadOnlyList<StatRow>>(players),
                    Turns = Turns + combat.Turns,
                    Combats = Combats + 1,
                    Plays = plays,
                    DamageReceived = DamageReceived + combat.DamageReceived,
                    EndedAt = Outcome is "active" or "suspended" or "" ? Math.Max(EndedAt, combat.StartedAt) : EndedAt,
                    Coverage = Coverage.Merge(combat.Coverage)
                };
                if (PolicyVersion.HasValue && policy.HasValue && PolicyVersion != policy)
                    merged = merged with { Coverage = merged.Coverage.WithFailure("mixed-attribution-policies") };
                return merged;
            }
        }
        catch (OverflowException) { return RejectCombat(combat.Coverage); }
    }

    private SummaryView RejectCombat(CoverageSummary incoming)
        => this with { Coverage = Coverage.Merge(incoming).WithFailure("summary-overflow") };

    private static uint CheckedRowPlays(IReadOnlyList<StatRow> rows)
    {
        uint plays = 0;
        checked { foreach (var row in rows) plays += row.Plays; }
        return plays;
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
    public IReadOnlyList<string> PreservedRunIds { get; init; } = Array.Empty<string>();
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
        PlayerCards = Players.GroupBy(player => player.Slot).ToDictionary(group => group.Key, _ => (IReadOnlyList<StatRow>)Array.Empty<StatRow>()),
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
        PolicyVersion = PolicyVersion > 0 ? PolicyVersion : null,
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
    public RunRecord Run { get; init; }
    public CombatStatistics Combat { get; init; }
}
