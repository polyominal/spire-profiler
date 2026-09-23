using System;
using System.Collections.Generic;
using System.Linq;
using MegaCrit.Sts2.Core.Entities.Cards;
using MegaCrit.Sts2.Core.Models;

namespace SpireProfiler;

internal static partial class BaselineTemporalAccumulator
{
    private static SourceSnapshot Take(object power, object card, CaptureEpoch epoch)
        => TemporalPowerCapture.Take(power, card, epoch);
    private static void Save(object power, object card, int amount, SourceSnapshot source, CaptureEpoch epoch)
        => TemporalPowerCapture.Save(power, card, amount, source, epoch);
    private static SourceSnapshot Combine(CaptureEpoch epoch, SourceSnapshot first, int before, SourceSnapshot second, int added)
        => throw new InvalidOperationException("Overflow must happen before source combination");
}

internal static partial class ManagedFixtures
{
    internal static void RunTemporalParity()
    {
        int before = assertions;
        string original = null, current = null;
        Test("original temporal overflow frees exactly one pending slot", () => original = TemporalOverflow(BaselineTemporalAccumulator.Accumulate));
        Test("current temporal overflow frees exactly one pending slot", () => current = TemporalOverflow(TemporalPowerCapture.Accumulate));
        Check(original == current, "Original dictionary mutation, pending capacity and diagnostics survive temporal overflow");
        Console.WriteLine($"TEMPORAL PARITY PASS: {assertions - before} assertions");
    }
    private static string TemporalOverflow(Action<Dictionary<CardModel, int>, CardModel, int> accumulate)
    {
        var power = new ProbeModel("TEMPORAL", ProducerRole.Power);
        var cards = Enumerable.Range(0, TemporalPowerCapture.MaxPending + 2).Select(_ => Card()).ToArray();
        var captured = CaptureRuntime.Epoch;
        FlowCapture.Current = Frame(power, A, ProducerRole.Power);
        for (int index = 0; index < TemporalPowerCapture.MaxPending; index++)
            TemporalPowerCapture.Save(power, cards[index], index == 0 ? int.MaxValue : 1, A, captured);
        TemporalPowerCapture.Save(power, cards[^2], 1, B, captured);
        Check(TemporalPowerCapture.Take(power, cards[^2], captured).Handle == 0, "Fixture fills the real 128-entry pending table");
        var dictionary = new Dictionary<CardModel, int> { [cards[0]] = int.MaxValue };
        accumulate(dictionary, cards[0], unchecked(int.MaxValue + 1));
        TemporalPowerCapture.Save(power, cards[^1], 1, B, captured);
        Same(TemporalPowerCapture.Take(power, cards[^1], captured), B, "Overflow removes the old entry and leaves room for a subsequent source");
        int retained = 0;
        foreach (var card in cards.Take(TemporalPowerCapture.MaxPending))
            if (TemporalPowerCapture.Take(power, card, captured).Handle != 0) retained++;
        Check(dictionary[cards[0]] == int.MinValue && retained == 127, "Physical wrapped amount remains while exactly the taken entry is absent");
        Check(backend.Calls.Contains("diagnostic:temporal-accumulate") && !backend.Calls.Contains("SourceAccumulate"),
            "Checked observer overflow precedes native combination and reports the original failure category");
        return dictionary[cards[0]] + ":" + retained + ":" + string.Join(',', backend.Calls.Where(call => call.StartsWith("diagnostic:", StringComparison.Ordinal)));
    }
}
