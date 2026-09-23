using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;

namespace SpireProfiler;

internal static partial class ManagedFixtures
{
    internal static void RunAttributionParity(string directory)
    {
        Test("original .NET decimal credit oracle", NativeModifierProjection);
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
}
