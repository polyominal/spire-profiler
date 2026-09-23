using System;

namespace SpireProfiler;

internal static class DecimalWords
{
    internal static (ulong Low, ulong High) Pack(decimal value)
    {
        Span<int> words = stackalloc int[4];
        decimal.GetBits(value, words);
        return (unchecked((uint)words[0]) | ((ulong)unchecked((uint)words[1]) << 32),
            unchecked((uint)words[2]) | ((ulong)unchecked((uint)words[3]) << 32));
    }
}
