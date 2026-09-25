using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using HarmonyLib;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Commands;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.GameActions.Multiplayer;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Cards;
using MegaCrit.Sts2.Core.Models.Characters;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Rooms;
using MegaCrit.Sts2.Core.Runs;
using MegaCrit.Sts2.Core.ValueProps;

#pragma warning disable CA1861 // Expected packets stay beside their behavioral assertions.

namespace SpireProfiler;

internal static class AuditFixtures
{
    private static int assertions;
    private static readonly RunRecord Header = new()
    {
        RunId = "1",
        Profile = 97,
        Seed = "AUDIT-FIXTURE",
        StartedAt = 100,
        Character = "SILENT",
        GameMode = "Standard",
        Players = new[] { new PlayerSummary(0, "SILENT") }
    };

    internal static void Run(string generatedDirectory, string nativeLibrary)
    {
        string root = Path.Combine(generatedDirectory, "audit-fixtures");
        Directory.CreateDirectory(root);
        JournalPrefix(Path.Combine(root, "prefix.audit.jsonl"));
        JournalLimits(root);
        JournalFailures(root);
        CaptureRuntime.Initialize(new NativeAttributionBackend());
        var world = new World();
        var baseline = Record(world, Path.Combine(root, "disabled"), nativeLibrary, false);
        var traced = Record(world, Path.Combine(root, "enabled"), nativeLibrary, true);
        Check(baseline == traced, "Audit capture must not change exact native summaries or replay bytes");
        Check(!Directory.Exists(Path.Combine(root, "disabled", "audit-v1")), "Disabled audit must create no files");
        var trace = Directory.GetFiles(Path.Combine(root, "enabled"), "*.audit.jsonl", SearchOption.AllDirectories).Single();
        var events = Read(trace);
        Check(events.First().GetProperty("event").GetString() == "combat_start"
            && events.Last().GetProperty("event").GetString() == "combat_end"
            && events.Last().GetProperty("data").GetProperty("complete").GetBoolean(), "Complete fixtures need a healthy footer");
        Check(events.Count(item => item.GetProperty("event").GetString() == "poison_tick") == 5
            && events.Count(item => item.GetProperty("event").GetString() == "poison_damage") == 5,
            "Every physical poison tick must retain its request and result");
        var changes = events.Where(item => item.GetProperty("event").GetString() == "poison_change").Select(item => item.GetProperty("data")).ToArray();
        Check(changes.Length == 7 && changes.All(item => item.GetProperty("status").GetInt32() == 1)
            && changes.Last().GetProperty("after").GetInt32() == 0, "Accepted applications and FIFO decay must remain independently visible");
        Check(events.Where(item => item.GetProperty("event").GetString() == "poison_attempt").All(item => item.GetProperty("data").GetProperty("amount").ValueKind == JsonValueKind.String),
            "Raw requested decimals remain exact text instead of floating-point values");
        Check(events.Any(item => item.GetProperty("event").GetString() == "poison_tick"
            && item.GetProperty("data").GetProperty("accelerants").EnumerateArray().Any(power => power.GetProperty("amount").GetInt32() == 2)),
            "Tick evidence must expose the actual Accelerant amount without crediting it");
        UnknownRoster(Path.Combine(root, "unknown-roster"), world);
        AttemptsAndThreads(Path.Combine(root, "attempts"));
        StartFailure(Path.Combine(root, "start-failure"));
        DelayedCombatCallbacks(Path.Combine(root, "delayed"));
        PoisonAudit.Initialize(root, "fixture", "fixture", _ => { }, false);
        CaptureRuntime.InvalidateEpoch();
        RunContext.CaptureRunPlayers(null);
        ProfilerNative.Dispose();
        ProfilerNative.Load(nativeLibrary);
        Console.WriteLine($"MANAGED AUDIT FIXTURES PASS ({assertions} assertions); report input: {trace}");
    }

    private static void Check(bool condition, string reason)
    {
        assertions++;
        if (!condition) throw new InvalidOperationException("Audit fixture: " + reason);
    }

    private static JsonElement[] Read(string path) => File.ReadAllLines(path).Select(line =>
    {
        using var document = JsonDocument.Parse(line);
        return document.RootElement.Clone();
    }).ToArray();

    private static void JournalPrefix(string path)
    {
        using var journal = new AuditJournal(path, _ => { });
        Check(journal.Append(1, "fact", new { Text = "蛇毒", Amount = "1.25" }) == 1, "The first fact gets sequence one");
        var prefix = Read(path);
        Check(prefix.Length == 1 && prefix[0].GetProperty("data").GetProperty("text").GetString() == "蛇毒",
            "The live journal must flush a complete UTF-8 JSON line before disposal");
        Check(journal.Append(1, "fact", new { Amount = "2" }) == 2 && Read(path).Length == 2,
            "A reader can inspect the growing prefix while capture remains open");
    }

    private static void JournalLimits(string root)
    {
        foreach (var sample in new[] { (Name: "events", Limit: 2, Bytes: 4096L, Payload: "x"), (Name: "bytes", Limit: 100, Bytes: 2048L, Payload: new string('界', 120)), (Name: "event-size", Limit: 100, Bytes: 4 * 1024 * 1024L, Payload: new string('x', 1024 * 1024)) })
        {
            string path = Path.Combine(root, sample.Name + ".audit.jsonl");
            var messages = new List<string>();
            using var journal = new AuditJournal(path, messages.Add, sample.Limit, sample.Bytes);
            for (int i = 0; i < 4 && journal.Open; i++) journal.Append(0, "fact", new { sample.Payload });
            Check(!journal.Open && !journal.Complete && journal.Append(0, "late", new { }) == 0, "Capacity must close the journal without pretending capture completed");
            var lines = Read(path);
            Check(lines.Count(item => item.GetProperty("event").GetString() == "trace_truncated") == 1
                && lines.Last().GetProperty("event").GetString() == "trace_truncated"
                && lines.Select(item => item.GetProperty("seq").GetUInt64()).SequenceEqual(Enumerable.Range(1, lines.Length).Select(i => (ulong)i)),
                "A bounded prefix must end with exactly one ordered truncation marker");
            Check(new FileInfo(path).Length <= sample.Bytes && messages.Count == 1, "Limits bound disk bytes and report the cutoff once");
        }
    }

    private static void JournalFailures(string root)
    {
        string path = Path.Combine(root, "write-failure.audit.jsonl");
        var messages = new List<string>();
        using var journal = new AuditJournal(path, messages.Add);
        journal.Append(0, "kept", new { });
        // Closing the owned OS stream injects a write failure without changing
        // permissions or relying on a platform-specific failing device.
        var writer = (StreamWriter)AccessTools.Field(typeof(AuditJournal), "writer").GetValue(journal);
        writer.BaseStream.Dispose();
        Check(journal.Append(0, "lost", new { }) == 0 && !journal.Open && !journal.Complete,
            "File failure must close capture and never escape into gameplay");
        Check(Read(path).Length == 1 && messages.Count >= 1, "A failed write must preserve the prior readable prefix");
        using var throwingReporter = new AuditJournal(Path.Combine(root, "reporter-failure.audit.jsonl"), _ => throw new InvalidOperationException("diagnostic failed"), 1);
        throwingReporter.Append(0, "kept", new { });
        Check(throwingReporter.Append(0, "cutoff", new { }) == 0 && !throwingReporter.Complete,
            "A failed diagnostic sink cannot throw from tracing");
    }

    private static void AttemptsAndThreads(string root)
    {
        PoisonAudit.Initialize(root, "fixture", "fixture", _ => { }, true);
        PoisonAudit.Start(Header, 9, "FIRST");
        PoisonAudit.Start(Header, 9, "RESTARTED");
        var worker = new Thread(() => PoisonAudit.Diagnostic("worker observation"));
        worker.Start();
        worker.Join();
        PoisonAudit.Finish("completed", CoverageSummary.Healthy);
        var attempts = Directory.GetFiles(root, "*.audit.jsonl", SearchOption.AllDirectories).Select(Read).ToArray();
        Check(attempts.Length == 2 && attempts.Select(items => items[0].GetProperty("data").GetProperty("attempt_id").GetString()).Distinct().Count() == 2,
            "Restarting the same epoch must preserve two distinct attempt files");
        var first = attempts.Single(items => items[0].GetProperty("data").GetProperty("encounter").GetString() == "FIRST");
        Check(first.Last().GetProperty("data").GetProperty("result").GetString() == "interrupted"
            && !first.Last().GetProperty("data").GetProperty("complete").GetBoolean(), "Replacing an open attempt must mark its footer interrupted");
        var second = attempts.Single(items => items != first);
        Check(second.Count(item => item.GetProperty("event").GetString() == "diagnostic"
            && item.GetProperty("data").GetProperty("reason").GetString() == "audit-off-thread-observation") == 1
            && !second.Last().GetProperty("data").GetProperty("complete").GetBoolean(),
            "An off-thread observation must become an owner-thread gap and an incomplete footer");
        PoisonAudit.Start(Header, 10, "PARTIAL");
        PoisonAudit.Finish("completed", CoverageSummary.Healthy.WithFailure("missing-capture"));
        var partial = Directory.GetFiles(root, "*.audit.jsonl", SearchOption.AllDirectories).Select(Read)
            .Single(items => items[0].GetProperty("data").GetProperty("encounter").GetString() == "PARTIAL");
        Check(!partial.Last().GetProperty("data").GetProperty("complete").GetBoolean(), "Successful journal writes cannot erase incomplete native coverage");
    }

    private static void UnknownRoster(string root, World world)
    {
        var run = (RunState)AccessTools.Field(typeof(RunContext), "_runState").GetValue(null);
        PoisonAudit.Initialize(root, "fixture", "fixture", _ => { }, true);
        PoisonAudit.Start(Header, 1, "UNKNOWN_ROSTER");
        try
        {
            RunContext.CaptureRunPlayers(null);
            PoisonAudit.Card(world.Play(world.Envenom));
            PoisonAudit.Checkpoint("combat_checkpoint", world.Combat, CombatSide.Player);
            PoisonAudit.Trigger(world.Poison, AccessTools.DeclaredMethod(typeof(PoisonPower), "Trigger"));
            PoisonAudit.Finish("completed", CoverageSummary.Healthy);
            var events = Read(Directory.GetFiles(root, "*.audit.jsonl", SearchOption.AllDirectories).Single());
            var card = events.Single(item => item.GetProperty("event").GetString() == "card_play").GetProperty("data");
            var player = events.Single(item => item.GetProperty("event").GetString() == "combat_checkpoint").GetProperty("data")
                .GetProperty("creatures").EnumerateArray().Single(creature => creature.GetProperty("model").GetString() == "SILENT");
            var accelerant = events.Single(item => item.GetProperty("event").GetString() == "poison_trigger").GetProperty("data")
                .GetProperty("accelerants").EnumerateArray().Single();
            Check(card.GetProperty("player").GetInt32() == 4 && player.GetProperty("player").GetInt32() == 4
                && accelerant.GetProperty("player").GetInt32() == 4 && accelerant.GetProperty("creature").GetProperty("player").GetInt32() == 4,
                "An unavailable roster must preserve Unknown instead of asserting player one for raw game evidence");
        }
        finally { RunContext.CaptureRunPlayers(run); }
        Check(RunContext.CreditorSlot(world.Envenom.Owner) == 0, "Roster restoration preserves the known player identity");
    }

    private static void StartFailure(string root)
    {
        Directory.CreateDirectory(root);
        File.WriteAllText(Path.Combine(root, "audit-v1"), "blocking file");
        var messages = new List<string>();
        PoisonAudit.Initialize(root, "fixture", "fixture", messages.Add, true);
        var before = (ProfilerNative.Revision, ProfilerNative.Snapshot(), ProfilerNative.Recording());
        PoisonAudit.Start(Header, 11, "UNWRITABLE");
        PoisonAudit.Diagnostic("after failure");
        PoisonAudit.Finish("completed", CoverageSummary.Healthy);
        Check(!PoisonAudit.Active && messages.Count == 1 && before == (ProfilerNative.Revision, ProfilerNative.Snapshot(), ProfilerNative.Recording()),
            "An unwritable trace directory must disable that attempt without changing attribution");
    }

    private static void DelayedCombatCallbacks(string root)
    {
        var old = new World();
        old.Reset();
        ProfilerNative.CombatDiscard();
        Check(ProfilerNative.CombatStarted(12, "OLD", "normal", 100, 1) == 12, "Old native combat starts");
        CaptureRuntime.Register(new NativeAttributionBackend(), 12, old.Combat);
        PoisonAudit.Initialize(root, "fixture", "fixture", _ => { }, true);
        PoisonAudit.Start(Header, 12, "OLD");
        old.Change(3, old.Envenom);
        FlowCapture.Prefix(old.Poison, AccessTools.DeclaredMethod(typeof(PoisonPower), "Trigger"), Array.Empty<object>(), out var prior);
        var tick = PoisonAudit.Tick(FlowCapture.Current.Audit, old.Enemy, null, 3, ValueProp.Unblockable, null);
        var mutation = PoisonAudit.BeforeMutation(old.Poison, AccessTools.DeclaredMethod(typeof(PowerModel), "SetAmount"), new object[] { 2, false });
        var pause = new TaskCompletionSource();
        var context = new FixtureContext();
        var previousContext = SynchronizationContext.Current;
        SynchronizationContext.SetSynchronizationContext(context);
        Creature replacementTarget = null;
        async Task Delayed()
        {
            await pause.Task;
            PoisonAudit.Card(old.Play(old.Envenom));
            Check(PoisonAudit.Tick(null, replacementTarget, null, 1, ValueProp.Unblockable, null) == null,
                "An inherited old producer scope cannot relabel a new creature as a new-combat event");
            PoisonAudit.Damage(tick, new[] { new DamageResult(old.Enemy, ValueProp.Unblockable) { UnblockedDamage = 3 } });
            PoisonAudit.AfterMutation(mutation, 1);
        }
        var delayed = Delayed();
        FlowCapture.Finalizer(prior);
        try
        {
            var replacement = new World();
            replacement.Reset();
            replacementTarget = replacement.Enemy;
            ProfilerNative.CombatDiscard();
            Check(ProfilerNative.CombatStarted(13, "NEW", "normal", 100, 1) == 13, "Replacement native combat starts");
            CaptureRuntime.Register(new NativeAttributionBackend(), 13, replacement.Combat);
            PoisonAudit.Start(Header, 13, "NEW");
            pause.SetResult();
            context.Complete(delayed);
            Check(PoisonAudit.Trigger(old.Poison, AccessTools.DeclaredMethod(typeof(PoisonPower), "Trigger")) == null,
                "An old creature is rejected even after its stale scope has unwound");
            CaptureRuntime.InvalidateEpoch();
            var raw = PoisonAudit.Tick(null, replacement.Enemy, null, 1, ValueProp.Unblockable, null);
            Check(raw != null,
                "Unavailable attribution capture must not suppress same-combat raw game evidence");
            var changingOwner = PoisonAudit.BeforeMutation(replacement.Poison, AccessTools.DeclaredMethod(typeof(PowerModel), "SetAmount"), new object[] { 2, false });
            Check(changingOwner != null, "A same-combat physical mutation is observable before capture admission");
            replacement.Enemy.CombatState = old.Combat;
            PoisonAudit.AfterMutation(changingOwner, 1);
            PoisonAudit.Damage(raw, new[] { new DamageResult(replacement.Enemy, ValueProp.Unblockable) { UnblockedDamage = 1 } });
            replacement.Enemy.CombatState = replacement.Combat;
            PoisonAudit.Finish("completed", CoverageSummary.Healthy);
            var events = Directory.GetFiles(root, "*.audit.jsonl", SearchOption.AllDirectories).Select(Read)
                .Single(items => items[0].GetProperty("data").GetProperty("encounter").GetString() == "NEW");
            Check(events.Count(item => item.GetProperty("event").GetString() == "diagnostic") == 5
                && events.Count(item => item.GetProperty("event").GetString() == "damage_begin") == 1
                && !events.Any(item => item.GetProperty("event").GetString() is "card_play" or "poison_damage" or "damage_result" or "poison_change" or "poison_trigger")
                && !events.Last().GetProperty("data").GetProperty("complete").GetBoolean(),
                "Delayed callbacks leave explicit gaps without contaminating replacement-combat facts");
        }
        finally { SynchronizationContext.SetSynchronizationContext(previousContext); FlowCapture.Current = ProducerFrame.Barrier; }
    }

    private static (string Snapshot, string Recording) Record(World world, string root, string nativeLibrary, bool enabled)
    {
        ProfilerNative.Dispose();
        ProfilerNative.Load(nativeLibrary);
        Check(ProfilerNative.RecordingBegin() && ProfilerNative.CombatStarted(1, "AUDIT_FIXTURE", "normal", 100, 1) == 1, "Native fixture combat starts with recording");
        world.Reset();
        CaptureRuntime.Register(new NativeAttributionBackend(), 1, world.Combat);
        FlowCapture.Current = ProducerFrame.Barrier;
        PoisonAudit.Initialize(root, "0.111.0", "audit-fixture", _ => { }, enabled);
        PoisonAudit.Start(Header, 1, "AUDIT_FIXTURE");
        Check(ProfilerNative.TurnStarted(1) == 1, "The physical player turn reaches the native ledger");
        PoisonAudit.Checkpoint("turn", world.Combat, CombatSide.Player);
        var retained = new List<SourceSnapshot>();
        foreach (var (card, amount) in new[] { (world.Envenom, 3), (world.Snakebite, 2) })
        {
            var source = FlowCapture.Source(card, CaptureRuntime.Epoch);
            retained.Add(source);
            var play = world.Play(card);
            PoisonAudit.Card(play);
            ulong token = ProfilerNative.CardPlayStarted(1, (ulong)amount, IdentityCapture.Existing(card, 1), card.Id.Entry, 0, 0, 1, 0, source.Handle);
            Check(token != 0, "Fixture card play starts");
            world.Change(amount, card);
            Check(ProfilerNative.CardPlayFinished(token) == 1 && ProfilerNative.CardExecutionEnded(1, (ulong)amount) == 1, "Fixture card play completes");
        }
        foreach (int initial in new[] { 5, 2 })
        {
            PoisonAudit.Checkpoint("turn", world.Combat, CombatSide.Enemy);
            FlowCapture.Prefix(world.Poison, AccessTools.DeclaredMethod(typeof(PoisonPower), "Trigger"), Array.Empty<object>(), out var producer);
            try
            {
                retained.Add(FlowCapture.Current.Source);
                for (int amount = initial; amount > Math.Max(0, initial - 3); amount--)
                {
                    var tick = PoisonAudit.Tick(FlowCapture.Current.Audit, world.Enemy, null, amount, ValueProp.Unblockable, null);
                    var source = FlowCapture.Source(world.Poison, CaptureRuntime.Epoch);
                    retained.Add(source);
                    ulong damage = ProfilerNative.DamageCalculationBegin(1, source.Handle, 2, 1, IdentityCapture.Existing(world.Enemy, 1));
                    Check(damage != 0 && ProfilerNative.DamageResultAppend(damage, amount, amount, 0, 0, 4, 0) == 1
                        && ProfilerNative.DamageCalculationCommit(damage) == 1, "The actual native ledger accepts each physical tick");
                    AccessTools.Field(typeof(Creature), "_currentHp").SetValue(world.Enemy, world.Enemy.CurrentHp - amount);
                    PoisonAudit.Damage(tick, new[] { new DamageResult(world.Enemy, ValueProp.Unblockable) { UnblockedDamage = amount } });
                    world.Change(-1, null);
                }
            }
            finally { FlowCapture.Finalizer(producer); }
        }
        PoisonAudit.Checkpoint("combat_checkpoint", world.Combat, CombatSide.Enemy);
        Check(ProfilerNative.CombatEnded(1) == 1, "Fixture combat completes");
        PoisonAudit.Finish("completed", CoverageSummary.Healthy);
        string snapshot = ProfilerNative.Snapshot(), recording = ProfilerNative.Recording();
        var summary = StatisticsJson.ParseNative(snapshot);
        Check(summary.Cards.Single(row => row.Id == "ENVENOM").DmgAttributed == 6
            && summary.Cards.Single(row => row.Id == "SNAKEBITE").DmgAttributed == 9 && summary.Coverage.Complete,
            "Earlier three stacks earn six points while later two stacks earn nine under FIFO decay");
        Check(ProfilerNative.Replay(recording) == snapshot, "The native recording remains exactly replayable with or without auditing");
        GC.KeepAlive(retained);
        return (snapshot, recording);
    }

    private sealed class World
    {
        internal readonly CombatState Combat = (CombatState)RuntimeHelpers.GetUninitializedObject(typeof(CombatState));
        internal readonly Creature Enemy = (Creature)RuntimeHelpers.GetUninitializedObject(typeof(Creature));
        private readonly Creature playerCreature = (Creature)RuntimeHelpers.GetUninitializedObject(typeof(Creature));
        private readonly Player player = (Player)RuntimeHelpers.GetUninitializedObject(typeof(Player));
        internal readonly PoisonPower Poison = ManagedFixtures.Mutable<PoisonPower>("POISON_POWER");
        internal readonly CardModel Envenom = ManagedFixtures.Mutable<Envenom>("ENVENOM");
        internal readonly CardModel Snakebite = ManagedFixtures.Mutable<Snakebite>("SNAKEBITE");
        private readonly List<PowerModel> powers = new();

        internal World()
        {
            var run = (RunState)RuntimeHelpers.GetUninitializedObject(typeof(RunState));
            var room = (CombatRoom)RuntimeHelpers.GetUninitializedObject(typeof(CombatRoom));
            AccessTools.Field(typeof(RunState), "_players").SetValue(run, new List<Player> { player });
            AccessTools.Field(typeof(RunState), "_currentRooms").SetValue(run, new List<AbstractRoom> { room });
            AccessTools.Field(typeof(CombatRoom), "<CombatState>k__BackingField").SetValue(room, Combat);
            AccessTools.Field(typeof(Player), "<Creature>k__BackingField").SetValue(player, playerCreature);
            AccessTools.Field(typeof(Player), "<Character>k__BackingField").SetValue(player, ManagedFixtures.Mutable<Silent>("SILENT"));
            AccessTools.Field(typeof(Player), "_runState").SetValue(player, run);
            AccessTools.Field(typeof(Creature), "<Player>k__BackingField").SetValue(playerCreature, player);
            var accelerant = ManagedFixtures.Mutable<AccelerantPower>("ACCELERANT_POWER");
            AccessTools.Field(typeof(PowerModel), "_owner").SetValue(accelerant, playerCreature);
            AccessTools.Field(typeof(PowerModel), "_amount").SetValue(accelerant, 2);
            AccessTools.Field(typeof(Creature), "_powers").SetValue(playerCreature, new List<PowerModel> { accelerant });
            AccessTools.Field(typeof(Creature), "_currentHp").SetValue(playerCreature, 100);
            AccessTools.Field(typeof(Creature), "<Side>k__BackingField").SetValue(playerCreature, CombatSide.Player);
            AccessTools.Field(typeof(Creature), "_powers").SetValue(Enemy, powers);
            AccessTools.Field(typeof(Creature), "<Side>k__BackingField").SetValue(Enemy, CombatSide.Enemy);
            playerCreature.CombatState = Enemy.CombatState = Combat;
            AccessTools.Field(typeof(CombatState), "_allies").SetValue(Combat, new List<Creature> { playerCreature });
            AccessTools.Field(typeof(CombatState), "_enemies").SetValue(Combat, new List<Creature> { Enemy });
            Envenom.Owner = Snakebite.Owner = player;
            RunContext.CaptureRunPlayers(run);
        }

        internal void Reset()
        {
            powers.Clear();
            AccessTools.Field(typeof(PowerModel), "_owner").SetValue(Poison, Enemy);
            AccessTools.Field(typeof(PowerModel), "_amount").SetValue(Poison, 0);
            AccessTools.Field(typeof(Creature), "_currentHp").SetValue(Enemy, 100);
        }

        internal CardPlay Play(CardModel card)
        {
            var play = (CardPlay)RuntimeHelpers.GetUninitializedObject(typeof(CardPlay));
            AccessTools.Field(typeof(CardPlay), "<Card>k__BackingField").SetValue(play, card);
            AccessTools.Field(typeof(CardPlay), "<Target>k__BackingField").SetValue(play, Enemy);
            AccessTools.Field(typeof(CardPlay), "<Player>k__BackingField").SetValue(play, player);
            AccessTools.Field(typeof(CardPlay), "<PlayCount>k__BackingField").SetValue(play, 1);
            return play;
        }

        internal void Change(int delta, CardModel source)
        {
            bool attach = !powers.Contains(Poison);
            var command = attach
                ? AccessTools.DeclaredMethod(typeof(PowerCmd), "Apply", new[] { typeof(PlayerChoiceContext), typeof(PowerModel), typeof(Creature), typeof(decimal), typeof(Creature), typeof(CardModel), typeof(bool) })
                : AccessTools.DeclaredMethod(typeof(PowerCmd), "ModifyAmount", new[] { typeof(PlayerChoiceContext), typeof(PowerModel), typeof(decimal), typeof(Creature), typeof(CardModel), typeof(bool) });
            object[] args = attach ? new object[] { null, Poison, Enemy, (decimal)delta, playerCreature, source, false }
                : new object[] { null, Poison, (decimal)delta, playerCreature, source, false };
            ProvenanceCapture.CommandPrefix(command, args, out var pending);
            try
            {
                var mutation = attach ? AccessTools.DeclaredMethod(typeof(Creature), "ApplyPowerInternal") : AccessTools.DeclaredMethod(typeof(PowerModel), "SetAmount");
                object instance = attach ? Enemy : Poison;
                if (attach) AccessTools.Field(typeof(PowerModel), "_amount").SetValue(Poison, delta);
                ProvenanceCapture.MutationPrefix(instance, mutation, attach ? new object[] { Poison } : new object[] { Poison.Amount + delta, false }, out var before);
                // Physical game writes are injected; the production capture and
                // journal observe actual game objects around those writes.
                if (attach) powers.Add(Poison);
                else AccessTools.Field(typeof(PowerModel), "_amount").SetValue(Poison, Poison.Amount + delta);
                ProvenanceCapture.MutationFinalizer(before);
            }
            finally { ProvenanceCapture.CommandFinalizer(pending); }
        }
    }
}
