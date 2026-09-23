using System;
using System.Runtime.InteropServices;

namespace SpireProfiler;

[StructLayout(LayoutKind.Sequential)]
internal readonly struct ModifierObservation
{
    internal const int Additive = 0, Multiplicative = 1, Vulnerable = 2, Nested = 3;
    internal const int TopLevel = -1;
    internal readonly ulong Source, InputLow, InputHigh, OutputLow, OutputHigh;
    internal readonly int Parent, Kind;
    internal ModifierObservation(ulong source, decimal input, decimal output, int parent, int kind)
    {
        Source = source;
        (InputLow, InputHigh) = PackDecimal(input);
        (OutputLow, OutputHigh) = PackDecimal(output);
        Parent = parent;
        Kind = kind;
    }
    internal static (ulong Low, ulong High) PackDecimal(decimal value)
    {
        Span<int> words = stackalloc int[4];
        decimal.GetBits(value, words);
        return (unchecked((uint)words[0]) | ((ulong)unchecked((uint)words[1]) << 32),
            unchecked((uint)words[2]) | ((ulong)unchecked((uint)words[3]) << 32));
    }
}

[StructLayout(LayoutKind.Sequential)]
internal readonly struct ModifierCredit
{
    internal readonly ulong Source;
    internal readonly int Amount;
}
