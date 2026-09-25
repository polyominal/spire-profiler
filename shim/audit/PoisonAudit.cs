using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Threading;
using MegaCrit.Sts2.Core.Combat;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

internal sealed record AuditCreature(ulong Instance, ulong NativeInstance, string Model, int Player, int Hp, int Block, int Poison, int Artifact);
internal sealed record AuditAccelerant(int Player, int Amount, AuditCreature Creature);
internal sealed record AuditMutation(PoisonPower Power, Creature Owner, int Before, bool Attached, ulong Action, ulong Epoch, string Operation);
internal sealed record AuditTick(ulong Sequence, ulong Epoch, bool Poison);
internal sealed record AuditTrigger(PoisonPower Power, ulong Sequence, ulong Epoch)
{
    internal int Tick;
}

// audit-v1 journals preserve primitive game evidence independently of reducer
// inputs. Native state uses its own identities and remains explicitly labeled.
// Poison attribution policy 3 uses FIFO decay and gives Accelerant no credit.
internal static class PoisonAudit
{
    private sealed record Identity(ulong Value);
    private static ConditionalWeakTable<object, Identity> identities = new();
    private static AuditJournal journal;
    private static object auditCombat;
    private static string directory, gameVersion, modVersion;
    private static Action<string> report;
    private static ulong epoch, nextIdentity;
    private static uint turn;
    private static int ownerThread, offThread;
    internal static bool Enabled { get; private set; }
    internal static bool Active
    {
        get
        {
            if (journal?.Open != true) return false;
            if (Environment.CurrentManagedThreadId == ownerThread) return true;
            // Only the diagnostic gap flag crosses threads; game state and the
            // journal remain owned by the capture thread.
            Interlocked.Exchange(ref offThread, 1);
            return false;
        }
    }

    internal static void Initialize(string root, string game, string mod, Action<string> diagnostic, bool enabled)
    {
        journal?.Dispose();
        journal = null;
        auditCombat = null;
        directory = Path.Combine(root, "audit-v1");
        gameVersion = game;
        modVersion = mod;
        report = message => { try { diagnostic(message); } catch (Exception) { } };
        ownerThread = Environment.CurrentManagedThreadId;
        Enabled = enabled;
    }

    internal static void Start(RunRecord run, ulong combatEpoch, string encounter)
    {
        if (!Enabled) return;
        try
        {
            Finish("interrupted", null);
            epoch = combatEpoch;
            turn = 0;
            nextIdentity = 0;
            identities = new();
            offThread = 0;
            auditCombat = CaptureRuntime.Backend?.CurrentCombat;
            string attempt = Guid.NewGuid().ToString("N");
            string runId = run?.RunId ?? "0";
            journal = new AuditJournal(Path.Combine(directory, "run-" + runId, $"combat-{epoch}-{attempt}.audit.jsonl"), report);
            Emit("combat_start", new
            {
                AuditVersion = 1,
                PolicyVersion = 3,
                GameVersion = gameVersion,
                ModVersion = modVersion,
                RunId = runId,
                Seed = run?.Seed ?? "",
                Epoch = epoch,
                AttemptId = attempt,
                Encounter = encounter,
                Players = run?.Players
            });
        }
        catch (Exception ex) { journal?.Dispose(); journal = null; auditCombat = null; report($"cannot start audit trace: {ex.Message}"); }
    }

    internal static void Finish(string result, CoverageSummary coverage)
    {
        if (journal == null) return;
        try
        {
            if (Active)
            {
                var native = Native();
                Emit("combat_end", new
                {
                    Result = result,
                    Coverage = coverage,
                    Complete = journal.Complete && offThread == 0 && coverage?.Complete == true && result != "interrupted",
                    Native = native
                });
            }
        }
        catch (Exception ex) { Diagnostic("audit-finish: " + ex.Message); }
        finally { journal.Dispose(); journal = null; auditCombat = null; identities = new(); }
    }

    internal static void EndCheckpoint()
    {
        if (!Active) return;
        try
        {
            if (CaptureRuntime.Backend.CurrentCombat is ICombatState combat)
                Checkpoint("combat_checkpoint", combat, combat.CurrentSide);
        }
        catch (Exception ex) { Diagnostic("audit-combat-end: " + ex.Message); }
    }

    internal static void Checkpoint(string kind, object combatState, CombatSide side)
    {
        if (!Active || combatState is not ICombatState combat) return;
        try
        {
            if (!AcceptOrigin(combat)) return;
            if (kind == "turn" && side == CombatSide.Player) turn++;
            Emit(kind, new { Side = side.ToString(), Creatures = combat.Creatures.Select(Creature).ToArray(), Native = Native() });
        }
        catch (Exception ex) { Diagnostic("audit-checkpoint: " + ex.Message); }
    }

    internal static void Card(CardPlay play)
    {
        if (!Active) return;
        try
        {
            if (!AcceptOrigin(play.Player.Creature.CombatState)) return;
            var card = play.Card;
            Emit("card_play", new
            {
                Execution = PlayCapture.Execution.Identity,
                Instance = Id(card),
                Card = card.Id.Entry,
                Player = RunContext.CreditorSlot(play.Player),
                Upgrade = card.CurrentUpgradeLevel,
                Enchantment = card.Enchantment?.Id.Entry,
                PlayIndex = play.PlayIndex,
                PlayCount = play.PlayCount,
                Target = Creature(play.Target)
            });
        }
        catch (Exception ex) { Diagnostic("audit-card: " + ex.Message); }
    }

    internal static ulong Attempt(MethodBase method, object[] args)
    {
        if (!Active) return 0;
        try
        {
            bool apply = method.Name == "Apply";
            if (args.Length < (apply ? 6 : 5) || args[1] is not PoisonPower power) return 0;
            var target = apply ? args[2] as Creature : power.Owner;
            if (!AcceptOrigin(target?.CombatState)) return 0;
            object source = args[apply ? 5 : 4] ?? FlowCapture.Current.Model;
            return Emit("poison_attempt", new
            {
                Command = method.Name,
                PowerInstance = NativeId(power),
                PowerIdentity = Id(power),
                Target = Creature(target),
                Amount = Convert.ToDecimal(args[apply ? 3 : 2], CultureInfo.InvariantCulture).ToString(CultureInfo.InvariantCulture),
                Source = (source as AbstractModel)?.Id.Entry ?? "",
                Execution = PlayCapture.Execution.Identity,
                Native = Native()
            });
        }
        catch (Exception ex) { Diagnostic("audit-attempt: " + ex.Message); return 0; }
    }

    internal static AuditMutation BeforeMutation(object instance, MethodBase method, object[] args)
    {
        if (!Active) return null;
        try
        {
            object candidate = method.Name == "SetAmount" ? instance : args[0];
            if (candidate is not PoisonPower power) return null;
            var owner = instance as Creature ?? power.Owner ?? ProvenanceCapture.Pending.Target as Creature;
            if (!AcceptOrigin(owner?.CombatState)) return null;
            return new(power, owner, power.Amount, owner?.Powers.Contains(power) == true, ProvenanceCapture.Pending.AuditAction, epoch, method.Name);
        }
        catch (Exception ex) { Diagnostic("audit-mutation-before: " + ex.Message); return null; }
    }

    internal static void AfterMutation(AuditMutation before, int? status)
    {
        if (!Active || before == null || before.Epoch != epoch) return;
        try
        {
            if (!AcceptOrigin(before.Owner?.CombatState)) return;
            bool attached = before.Owner?.Powers.Contains(before.Power) == true;
            Emit("poison_change", new
            {
                Action = before.Action,
                Operation = before.Operation,
                PowerInstance = NativeId(before.Power),
                PowerIdentity = Id(before.Power),
                Target = Creature(before.Owner),
                Before = before.Before,
                After = before.Power.Amount,
                BeforeAttached = before.Attached,
                AfterAttached = attached,
                Status = status,
                Native = Native()
            });
        }
        catch (Exception ex) { Diagnostic("audit-mutation-after: " + ex.Message); }
    }

    internal static AuditTrigger Trigger(object model, MethodBase method)
    {
        if (!Active || method.Name != "Trigger" || model is not PoisonPower power) return null;
        try
        {
            if (!AcceptOrigin(power.Owner?.CombatState)) return null;
            ulong sequence = Emit("poison_trigger", new { PowerInstance = NativeId(power), Target = Creature(power.Owner), Accelerants = Accelerants(power.Owner) });
            return sequence == 0 ? null : new(power, sequence, epoch);
        }
        catch (Exception ex) { Diagnostic("audit-trigger: " + ex.Message); return null; }
    }

    internal static AuditTick Tick(AuditTrigger trigger, Creature target, Creature dealer, decimal requested, ValueProp props, CardModel card)
    {
        if (!Active) return null;
        try
        {
            if (!AcceptOrigin(target?.CombatState)) return null;
            bool poison = trigger != null && trigger.Epoch == epoch && ReferenceEquals(trigger.Power.Owner, target);
            ulong sequence = poison ? Emit("poison_tick", new
            {
                TriggerSeq = trigger.Sequence,
                TickIndex = ++trigger.Tick,
                PowerInstance = NativeId(trigger.Power),
                Target = Creature(target),
                Requested = requested.ToString(CultureInfo.InvariantCulture),
                PoisonBefore = trigger.Power.Amount,
                HpBefore = target.CurrentHp,
                BlockBefore = target.Block,
                Accelerants = Accelerants(target),
                Native = Native()
            })
                : Emit("damage_begin", new
                {
                    Target = Creature(target),
                    Dealer = Creature(dealer),
                    Requested = requested.ToString(CultureInfo.InvariantCulture),
                    Props = props.ToString(),
                    Card = card?.Id.Entry,
                    Execution = PlayCapture.Execution.Identity
                });
            return sequence == 0 ? null : new(sequence, epoch, poison);
        }
        catch (Exception ex) { Diagnostic("audit-tick: " + ex.Message); return null; }
    }

    internal static void Damage(AuditTick tick, IReadOnlyList<DamageResult> results)
    {
        if (!Active || tick == null || tick.Epoch != epoch) return;
        try
        {
            int index = 0;
            var native = tick.Poison ? Native() : JsonSerializer.SerializeToElement<object>(null);
            foreach (var result in results)
            {
                if (!AcceptOrigin(result.Receiver?.CombatState)) continue;
                Emit(tick.Poison ? "poison_damage" : "damage_result", new
                {
                    TickSeq = tick.Sequence,
                    Target = Creature(result.Receiver),
                    HpAfter = result.Receiver.CurrentHp,
                    ReceiverSide = result.Receiver.IsPlayer || result.Receiver.PetOwner != null ? "player" : "enemy",
                    ReceiverKind = result.Receiver.IsPlayer ? "player" : result.Receiver.PetOwner != null ? "pet" : "monster",
                    GroupIndex = index++,
                    GroupCount = results.Count,
                    Unblocked = result.UnblockedDamage,
                    Blocked = result.BlockedDamage,
                    Overkill = result.OverkillDamage,
                    Native = native
                });
            }
        }
        catch (Exception ex) { Diagnostic("audit-damage: " + ex.Message); }
    }

    internal static void Diagnostic(string reason)
    {
        if (Active) Emit("diagnostic", new { Reason = reason });
    }

    private static bool AcceptOrigin(object observedCombat)
    {
        // Raw facts survive unavailable native capture, but never cross attempts.
        // A stale async scope can outlive the creature's own combat reference.
        bool stale = observedCombat == null || !ReferenceEquals(observedCombat, auditCombat)
            || Stale(FlowCapture.Current.Epoch) || Stale(ProvenanceCapture.Pending.Epoch)
            || Stale(PlayCapture.Execution.Epoch) || Stale(PlayCapture.Current?.Epoch ?? default)
            || Stale(DamageCapture.Current.Epoch) || Stale(CommandCapture.Current.Epoch);
        if (stale) Diagnostic("audit-foreign-combat-observation");
        return !stale;
    }

    private static bool Stale(CaptureEpoch origin) => (origin.Sequence != 0 && origin.Sequence != epoch)
        || (origin.Combat != null && !ReferenceEquals(origin.Combat, auditCombat));

    private static ulong Emit(string kind, object data)
    {
        if (!Active) return 0;
        if (Interlocked.Exchange(ref offThread, 0) != 0) journal.Append(turn, "diagnostic", new { Reason = "audit-off-thread-observation" });
        return journal.Append(turn, kind, data);
    }

    private static JsonElement Native()
    {
        try { using var parsed = JsonDocument.Parse(ProfilerNative.AuditSnapshot()); return parsed.RootElement.Clone(); }
        catch (Exception ex) { Diagnostic("audit-native-snapshot: " + ex.Message); return JsonSerializer.SerializeToElement<object>(null); }
    }

    private static ulong Id(object model)
    {
        if (model == null) return 0;
        if (identities.TryGetValue(model, out var identity)) return identity.Value;
        if (nextIdentity == 16_384) throw new InvalidOperationException("Audit identity capacity reached");
        var added = new Identity(++nextIdentity);
        identities.Add(model, added);
        return added.Value;
    }

    private static ulong NativeId(object model) => IdentityCapture.Existing(model, epoch);

    private static AuditCreature Creature(Creature creature) => creature == null ? null : new(Id(creature), NativeId(creature),
        creature.Monster?.Id.Entry ?? creature.Player?.Character.Id.Entry ?? "", creature.IsPlayer ? RunContext.CreditorSlot(creature.Player) : 4,
        creature.CurrentHp, creature.Block, creature.GetPowerAmount<PoisonPower>(), creature.GetPowerAmount<ArtifactPower>());

    private static AuditAccelerant[] Accelerants(Creature target) => target?.CombatState?.GetOpponentsOf(target).Where(c => c.IsAlive)
        .Select(c => new AuditAccelerant(c.IsPlayer ? RunContext.CreditorSlot(c.Player) : 4, c.GetPowerAmount<AccelerantPower>(), Creature(c))).ToArray();
}
