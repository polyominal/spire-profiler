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
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

internal sealed record AuditCreature(ulong Instance, ulong NativeInstance, string Model, int Player, int Hp, int Block, int Poison, int Artifact);
internal sealed record AuditAccelerant(int Player, int Amount, AuditCreature Creature);
internal sealed record AuditSource(ulong Sequence, ulong Epoch, object Model, ulong Cause = 0);
internal sealed record AuditSourceDescriptor(string Role, ulong Identity, string Model, int Player, string Origin);
internal sealed record AuditCommand(ulong Action, ulong Epoch, string Command, PowerModel Requested, Creature Target, AuditSource Source)
{
    internal PowerModel Power = Requested;
    internal AuditCreature TargetAtStart;
    internal int LastAmount;
    internal bool LastAttached, Changed;
}
internal sealed record AuditMutation(PowerModel Power, Creature Owner, int Before, bool Attached, ulong Action, ulong Epoch, string Operation, ulong SourceFrame);
internal sealed record AuditTick(ulong Sequence, ulong Epoch, bool Poison);
internal sealed record AuditTrigger(PoisonPower Power, ulong Sequence, ulong Epoch)
{
    internal int Tick;
}

// audit-v2 journals preserve primitive game evidence independently of reducer
// inputs. Native state uses its own identities and remains explicitly labeled.
// Poison attribution policy 3 uses FIFO decay and gives Accelerant no credit.
internal static class PoisonAudit
{
    private sealed record Identity(ulong Value);
    private static ConditionalWeakTable<object, Identity> identities = new();
    // A stalled game command must not retain an unbounded number of raw scopes.
    private const int MaxPending = 256;
    private static readonly Dictionary<ulong, AuditCommand> commands = new();
    private static readonly HashSet<ulong> envenom = new();
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
        commands.Clear();
        envenom.Clear();
        directory = Path.Combine(root, "audit-v2");
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
            commands.Clear();
            envenom.Clear();
            offThread = 0;
            auditCombat = CaptureRuntime.Backend?.CurrentCombat;
            string attempt = Guid.NewGuid().ToString("N");
            string runId = run?.RunId ?? "0";
            journal = new AuditJournal(Path.Combine(directory, "run-" + runId, $"combat-{epoch}-{attempt}.audit.jsonl"), report);
            Emit("combat_start", new
            {
                AuditVersion = 2,
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
                foreach (var command in commands.Values.ToArray()) Complete(command, "interrupted", null, null);
                foreach (ulong trigger in envenom.ToArray())
                    Emit("envenom_end", new { Trigger = trigger, Outcome = "interrupted" });
                if (envenom.Count != 0) Diagnostic("audit-envenom-interrupted");
                envenom.Clear();
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
        finally { journal.Dispose(); journal = null; auditCombat = null; identities = new(); commands.Clear(); envenom.Clear(); }
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
                Player = PlayerSlot(play.Player),
                Upgrade = card.CurrentUpgradeLevel,
                Enchantment = card.Enchantment?.Id.Entry,
                Source = Describe(card),
                PlayIndex = play.PlayIndex,
                PlayCount = play.PlayCount,
                Target = Creature(play.Target)
            });
        }
        catch (Exception ex) { Diagnostic("audit-card: " + ex.Message); }
    }

    internal static AuditSource Producer(object model, MethodBase method, object[] args)
    {
        if (!Active) return null;
        try
        {
            bool trigger = model is EnvenomPower && method.Name == "AfterDamageGiven";
            var source = model is PoisonPower or EnvenomPower || model is CardModel && method.Name == "OnPlay"
                ? SourceFrame(model, "producer") : new AuditSource(0, epoch, model);
            if (!trigger || source == null) return source;
            var power = (EnvenomPower)model;
            var dealer = args[1] as Creature;
            var result = (DamageResult)args[2];
            var props = (ValueProp)args[3];
            var target = (Creature)args[4];
            if (!AcceptOrigin(target?.CombatState) || !AcceptOrigin(power.Owner?.CombatState)) return null;
            if (envenom.Count >= MaxPending) { Diagnostic("audit-envenom-pending-cap"); return source; }
            ulong sequence = Emit("envenom_trigger", new
            {
                PowerIdentity = Id(power),
                Target = Creature(target),
                Owner = Creature(power.Owner),
                Dealer = Creature(dealer),
                ResultIdentity = Id(result),
                Unblocked = result.UnblockedDamage,
                Blocked = result.BlockedDamage,
                Overkill = result.OverkillDamage,
                Props = (int)props,
                Amount = power.Amount,
                Eligible = ReferenceEquals(dealer, power.Owner) && props.IsPoweredAttack() && result.UnblockedDamage > 0,
                Card = Describe(args[5]),
                SourceFrame = source.Sequence
            });
            if (sequence == 0) return source;
            envenom.Add(sequence);
            return source with { Cause = sequence };
        }
        catch (Exception ex) { Diagnostic("audit-producer: " + ex.Message); return null; }
    }

    internal static void EnvenomEnd(string outcome, Exception error = null)
    {
        if (!Active) return;
        try
        {
            var source = FlowCapture.Current.AuditSource;
            if (source == null || source.Epoch != epoch || !envenom.Remove(source.Cause)) return;
            Emit("envenom_end", new { Trigger = source.Cause, Outcome = outcome, Error = error?.GetType().Name });
            if (outcome is "faulted" or "cancelled") Diagnostic("audit-envenom-" + outcome);
        }
        catch (Exception ex) { Diagnostic("audit-envenom-end: " + ex.Message); }
    }

    internal static AuditSource DamageSource(object[] args)
    {
        if (!Active) return null;
        try
        {
            var inherited = FlowCapture.Current.AuditSource;
            object card = args[5];
            object model = card ?? inherited?.Model;
            // Poison's supplier queue is sampled at command entry, before any
            // target's modifier hooks can change the live power.
            if (card != null || model is PoisonPower || inherited?.Sequence == 0)
                return SourceFrame(model, "damage", inherited?.Cause ?? 0);
            return inherited;
        }
        catch (Exception ex) { Diagnostic("audit-damage-source: " + ex.Message); return null; }
    }

    internal static AuditCommand Attempt(MethodBase method, object[] args)
    {
        if (!Active) return null;
        try
        {
            bool apply = method.Name == "Apply";
            if (args.Length < (apply ? 6 : 5) || args[1] is not PowerModel power || power is not (PoisonPower or EnvenomPower)) return null;
            var target = apply ? args[2] as Creature : power.Owner;
            if (!AcceptOrigin(target?.CombatState)) return null;
            if (commands.Count >= MaxPending) { Diagnostic("audit-command-pending-cap"); return null; }
            object card = args[apply ? 5 : 4];
            var inherited = FlowCapture.Current.AuditSource;
            var source = inherited != null && inherited.Sequence != 0 && (card == null || ReferenceEquals(card, inherited.Model))
                ? inherited : SourceFrame(card ?? inherited?.Model, "command", inherited?.Cause ?? 0);
            var parent = ProvenanceCapture.Pending.Audit;
            bool sameParent = parent != null && parent.Epoch == epoch && ReferenceEquals(parent.Target, target) && parent.Power.Id == power.Id;
            if (sameParent && parent.Command == "Apply") parent.Power = power;
            ulong action = Emit("power_attempt", new
            {
                Command = method.Name,
                Power = power.Id.Entry,
                PowerInstance = NativeId(power),
                PowerIdentity = Id(power),
                Target = Creature(target),
                Amount = Convert.ToDecimal(args[apply ? 3 : 2], CultureInfo.InvariantCulture).ToString(CultureInfo.InvariantCulture),
                Source = Describe(card ?? inherited?.Model),
                SourceFrame = source?.Sequence ?? 0,
                Cause = source?.Cause ?? 0,
                ParentAction = sameParent ? parent.Action : 0,
                Execution = PlayCapture.Execution.Identity,
                Native = Native()
            });
            if (action == 0) return null;
            var command = new AuditCommand(action, epoch, method.Name, power, target, source)
            { TargetAtStart = Creature(target), LastAmount = power.Amount, LastAttached = target?.Powers.Contains(power) == true };
            commands.Add(action, command);
            return command;
        }
        catch (Exception ex) { Diagnostic("audit-attempt: " + ex.Message); return null; }
    }

    internal static void Complete(AuditCommand command, string outcome, int? result = null, Exception error = null)
    {
        if (!Active || command == null || command.Epoch != epoch) return;
        try
        {
            if (!commands.Remove(command.Action)) return;
            if (outcome != "interrupted")
            {
                if (!AcceptOrigin(command.Target?.CombatState)) return;
                if (command.Target.Powers.Contains(command.Requested)) command.Power = command.Requested;
                command.LastAmount = command.Power.Amount;
                command.LastAttached = command.Target.Powers.Contains(command.Power);
            }
            Emit("command_end", new
            {
                Action = command.Action,
                Command = command.Command,
                Outcome = outcome,
                Power = command.Power.Id.Entry,
                RequestedPowerIdentity = Id(command.Requested),
                PowerIdentity = Id(command.Power),
                Target = outcome == "interrupted" ? command.TargetAtStart : Creature(command.Target),
                Amount = command.LastAmount,
                Attached = command.LastAttached,
                ObservedChange = command.Changed,
                Result = result,
                Error = error?.GetType().Name
            });
            if (outcome is "faulted" or "cancelled" or "interrupted") Diagnostic("audit-command-" + outcome);
        }
        catch (Exception ex) { Diagnostic("audit-command-end: " + ex.Message); }
    }

    internal static AuditMutation BeforeMutation(object instance, MethodBase method, object[] args)
    {
        if (!Active) return null;
        try
        {
            object candidate = method.Name == "SetAmount" ? instance : args[0];
            if (candidate is not PowerModel power || power is not (PoisonPower or EnvenomPower)) return null;
            var owner = instance as Creature ?? power.Owner ?? ProvenanceCapture.Pending.Target as Creature;
            if (!AcceptOrigin(owner?.CombatState)) return null;
            var command = ProvenanceCapture.Pending.Audit;
            bool owns = command != null && command.Epoch == epoch && (ReferenceEquals(command.Power, power) || ReferenceEquals(command.Requested, power)) && ReferenceEquals(command.Target, owner);
            if (owns) command.Power = power;
            return new(power, owner, power.Amount, owner?.Powers.Contains(power) == true, owns ? command.Action : 0, epoch, method.Name, owns ? command.Source?.Sequence ?? 0 : 0);
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
            if (commands.TryGetValue(before.Action, out var command))
            {
                command.LastAmount = before.Power.Amount;
                command.LastAttached = attached;
                command.Changed |= before.Attached != attached || attached && before.Before != before.Power.Amount;
            }
            Emit("power_change", new
            {
                Action = before.Action,
                SourceFrame = before.SourceFrame,
                Operation = before.Operation,
                Power = before.Power.Id.Entry,
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
            ulong sequence = Emit("poison_trigger", new { PowerInstance = NativeId(power), PowerIdentity = Id(power), Target = Creature(power.Owner), Accelerants = Accelerants(power.Owner) });
            return sequence == 0 ? null : new(power, sequence, epoch);
        }
        catch (Exception ex) { Diagnostic("audit-trigger: " + ex.Message); return null; }
    }

    internal static AuditTick Tick(AuditTrigger trigger, Creature target, Creature dealer, decimal requested, ValueProp props, CardModel card, AuditSource source = null)
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
                PowerIdentity = Id(trigger.Power),
                SourceFrame = source?.Sequence ?? 0,
                Target = Creature(target),
                Dealer = Creature(dealer),
                Props = (int)props,
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
                    Props = (int)props,
                    Card = card?.Id.Entry,
                    SourceFrame = source?.Sequence ?? 0,
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
                    ResultIdentity = Id(result),
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

    private static AuditSource SourceFrame(object model, string context, ulong cause = 0)
    {
        Creature owner = model is PowerModel power ? power.Owner : (model as CardModel)?.Owner?.Creature;
        if (owner != null && !AcceptOrigin(owner.CombatState)) return null;
        var source = Describe(model);
        ulong sequence = Emit("source_frame", new
        {
            Source = source,
            PowerIdentity = model is PowerModel ? Id(model) : 0,
            Target = Creature(owner),
            Amount = model is PowerModel p ? p.Amount : 0,
            Cause = cause,
            Context = context
        });
        return sequence == 0 ? null : new(sequence, epoch, model, cause);
    }

    private static AuditSourceDescriptor Describe(object model)
    {
        if (model is CardModel card)
        {
            var deck = card.DeckVersion;
            bool ordinary = deck != null && ReferenceEquals(deck.Owner, card.Owner) && deck.Id == card.Id && card.Owner.Deck.Cards.Contains(deck);
            return new("card", Id(card), card.Id.Entry, PlayerSlot(card.Owner), ordinary ? "ordinary" : "unknown");
        }
        if (model is PoisonPower or EnvenomPower)
        {
            var power = (PowerModel)model;
            return new("power", Id(power), power.Id.Entry, power.Owner?.IsPlayer == true ? PlayerSlot(power.Owner.Player) : 4, "observed");
        }
        return new("unsupported", Id(model), (model as AbstractModel)?.Id.Entry ?? "", 4, "unknown");
    }

    private static bool AcceptOrigin(object observedCombat)
    {
        // Raw facts survive unavailable native capture, but never cross attempts.
        // A stale async scope can outlive the creature's own combat reference.
        bool stale = observedCombat == null || !ReferenceEquals(observedCombat, auditCombat)
            || Stale(FlowCapture.Current.Epoch) || Stale(ProvenanceCapture.Pending.Epoch)
            || Stale(PlayCapture.Execution.Epoch) || Stale(PlayCapture.Current?.Epoch ?? default)
            || Stale(DamageCapture.Current.Epoch) || Stale(CommandCapture.Current.Epoch)
            || Foreign(FlowCapture.Current.AuditSource?.Epoch ?? 0) || Foreign(ProvenanceCapture.Pending.Audit?.Epoch ?? 0)
            || Foreign(DamageCapture.Current.AuditSource?.Epoch ?? 0);
        if (stale) Diagnostic("audit-foreign-combat-observation");
        return !stale;
    }

    private static bool Stale(CaptureEpoch origin) => (origin.Sequence != 0 && origin.Sequence != epoch)
        || (origin.Combat != null && !ReferenceEquals(origin.Combat, auditCombat));
    private static bool Foreign(ulong origin) => origin != 0 && origin != epoch;

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

    private static int PlayerSlot(Player player)
    {
        if (player != null && auditCombat is ICombatState combat && combat.RunState?.Players is { } players)
        {
            for (int index = 0; index < Math.Min(players.Count, 4); index++)
                if (ReferenceEquals(players[index], player)) return index;
        }
        return 4;
    }

    private static AuditCreature Creature(Creature creature) => creature == null ? null : new(Id(creature), NativeId(creature),
        creature.Monster?.Id.Entry ?? creature.Player?.Character.Id.Entry ?? "", creature.IsPlayer ? PlayerSlot(creature.Player) : 4,
        creature.CurrentHp, creature.Block, creature.GetPowerAmount<PoisonPower>(), creature.GetPowerAmount<ArtifactPower>());

    private static AuditAccelerant[] Accelerants(Creature target) => target?.CombatState?.GetOpponentsOf(target).Where(c => c.IsAlive)
        .Select(c => new AuditAccelerant(c.IsPlayer ? PlayerSlot(c.Player) : 4, c.GetPowerAmount<AccelerantPower>(), Creature(c))).ToArray();
}
