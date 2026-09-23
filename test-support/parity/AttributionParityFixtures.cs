using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Text.Json;
using HarmonyLib;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Entities.Creatures;
using MegaCrit.Sts2.Core.Entities.Players;
using MegaCrit.Sts2.Core.Entities.Powers;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;
using MegaCrit.Sts2.Core.Models.Relics;
using MegaCrit.Sts2.Core.ValueProps;

namespace SpireProfiler;

internal sealed class ParityPower : PowerModel
{
    public override PowerType Type => PowerType.Buff;
    public override PowerStackType StackType => PowerStackType.Counter;
    internal List<string> Log;
    internal decimal Addition, Multiplier;
    internal int Calls, ThrowAt;
    private decimal Observe(string kind, decimal amount, CardModel card, CardPlay play)
    {
        Log.Add($"call:{Id.Entry}:{kind}:{amount}:{card != null}:{play != null}:{++Calls}");
        if (Calls == ThrowAt) throw new InvalidOperationException("callback-failure");
        return kind.EndsWith("add", StringComparison.Ordinal) ? Addition : Multiplier + (Calls - 1) / 10m;
    }
    public override decimal ModifyDamageAdditive(Creature target, decimal amount, ValueProp props, Creature dealer, CardModel cardSource, CardPlay cardPlay)
        => Observe("damage-add", amount, cardSource, cardPlay);
    public override decimal ModifyDamageMultiplicative(Creature target, decimal amount, ValueProp props, Creature dealer, CardModel cardSource, CardPlay cardPlay)
        => Observe("damage-multiply", amount, cardSource, cardPlay);
    public override decimal ModifyBlockAdditive(Creature target, decimal block, ValueProp props, CardModel cardSource, CardPlay cardPlay)
        => Observe("block-add", block, cardSource, cardPlay);
    public override decimal ModifyBlockMultiplicative(Creature target, decimal block, ValueProp props, CardModel cardSource, CardPlay cardPlay)
        => Observe("block-multiply", block, cardSource, cardPlay);
}

internal static partial class ManagedFixtures
{
    internal static void RunAttributionParity(string directory)
    {
        Test("original .NET decimal credit oracle", NativeModifierProjection);
        Test("original modifier callback and incremental credit oracle", OriginalModifierSequences);
        Test("original nested Vulnerable decomposition oracle", OriginalVulnerable);
        WriteOriginalMixtures(directory);
        Console.WriteLine($"ATTRIBUTION PARITY PASS: {assertions} assertions");
    }
    private static void WriteOriginalMixtures(string directory)
    {
        var random = new Random(0x328747);
        var cases = new List<object>();
        for (int test = 0; test < 512; test++)
        {
            ulong Weight() => test % 4 == 0 ? (ulong)random.NextInt64(1, long.MaxValue) : (ulong)random.Next(1, 100);
            var first = new[] { new OriginalSources.SourceShare((7ul << 32) | 16, Weight()), new OriginalSources.SourceShare(7ul << 32, 1) };
            var second = new[] { new OriginalSources.SourceShare((7ul << 32) | 8, Weight()), new OriginalSources.SourceShare((7ul << 32) | 16, 1) };
            int before = random.Next(1, int.MaxValue), added = random.Next(1, int.MaxValue);
            var output = OriginalSources.BaselineTemporal.Combine(new(7, new object()), OriginalSources.SourceSnapshot.Create(7, first), before,
                OriginalSources.SourceSnapshot.Create(7, second), added);
            cases.Add(new
            {
                first = first.Select(share => new[] { share.Destination, share.Weight }).ToArray(),
                second = second.Select(share => new[] { share.Destination, share.Weight }).ToArray(),
                before,
                added,
                expected = output.Epoch == 0 ? null : Enumerable.Range(0, output.Count).Select(index => new[] { output[index].Destination, output[index].Weight }).ToArray()
            });
        }
        File.WriteAllText(Path.Combine(directory, "source-mixture-oracle.json"), JsonSerializer.Serialize(cases));
        Console.WriteLine("ORIGINAL SOURCE MIXTURE ORACLE: " + cases.Count + " cases");
    }
    private static void OriginalModifierSequences()
    {
        var random = new Random(0x328747);
        var card = Mutable<MegaCrit.Sts2.Core.Models.Cards.StrikeIronclad>("CARD");
        var play = (CardPlay)RuntimeHelpers.GetUninitializedObject(typeof(CardPlay));
        for (int test = 0; test < 300; test++)
        {
            var models = Enumerable.Range(0, random.Next(1, 72)).Select(index =>
            {
                var model = Mutable<ParityPower>("POWER" + index);
                model.Addition = random.Next(4) == 0 ? 0 : random.Next(-3, 4) / 2m;
                model.Multiplier = random.Next(1, 5) / 2m;
                model.ThrowAt = test % 5 == 0 ? random.Next(1, 4) : 0;
                return model;
            }).ToArray();
            decimal start = test % 9 == 0 ? 0.9999999999999999999999999999m : random.Next(-10, 20);
            decimal result = random.Next(1, 40);
            foreach (bool damage in new[] { true, false })
            {
                List<string> Execute(bool original)
                {
                    var log = new List<string>();
                    foreach (var model in models) { model.Log = log; model.Calls = 0; }
                    int credits = 0;
                    void Contribute(AbstractModel model, int amount)
                    {
                        log.Add($"credit:{model.Id.Entry}:{amount}");
                        if (test % 7 == 0 && ++credits == 2) throw new InvalidOperationException("credit-failure");
                    }
                    try
                    {
                        if (damage)
                        {
                            if (original) BaselineAttribution.Decompose(models, start, result, null, null, ValueProp.Move, card, Contribute);
                            else DamageCapture.Decompose(models, start, result, null, null, ValueProp.Move, card, Contribute);
                        }
                        else if (original) BaselineAttribution.DecomposeBlock(models, start, result, null, ValueProp.Move, card, play, Contribute);
                        else CommandCapture.DecomposeBlock(models, start, result, null, ValueProp.Move, card, play, Contribute);
                    }
                    catch (Exception exception) { log.Add("throw:" + exception.GetType().Name + ":" + exception.Message); }
                    return log;
                }
                var expected = Execute(true); var actual = Execute(false);
                Check(expected.SequenceEqual(actual), $"Original ordered callbacks/credits case {test}, damage={damage}\n{string.Join('\n', expected)}\n---\n{string.Join('\n', actual)}");
            }
        }
    }
    private static void OriginalVulnerable()
    {
        var world = GameWorld();
        var vulnerable = GamePower<VulnerablePower>(world.Enemy, 1, A);
        var cruelty = GamePower<CrueltyPower>(world.Owner.Creature, 25, B);
        var debilitate = GamePower<DebilitatePower>(world.Enemy, 1, B);
        AccessTools.Field(typeof(Creature), "_powers").SetValue(world.Owner.Creature, new List<PowerModel> { cruelty });
        AccessTools.Field(typeof(Creature), "_powers").SetValue(world.Enemy, new List<PowerModel> { vulnerable, debilitate });
        var phrog = Mutable<PaperPhrog>("PHROG"); phrog.Owner = world.Owner;
        AccessTools.Field(typeof(Player), "_relics").SetValue(world.Owner, new List<RelicModel> { phrog });
        foreach (decimal basis in new[] { 1m, 3m, 20m, 0.9999999999999999999999999999m })
        {
            var expected = new List<(AbstractModel, int)>(); var actual = new List<(AbstractModel, int)>();
            BaselineAttribution.Decompose(new[] { vulnerable }, basis, basis * 3, world.Enemy, world.Owner.Creature, ValueProp.Move, null, (model, amount) => expected.Add((model, amount)));
            DamageCapture.Decompose(new[] { vulnerable }, basis, basis * 3, world.Enemy, world.Owner.Creature, ValueProp.Move, null, (model, amount) => actual.Add((model, amount)));
            Check(expected.SequenceEqual(actual), "Original nested supplier order and decimal credits at " + basis);
        }
    }
}
