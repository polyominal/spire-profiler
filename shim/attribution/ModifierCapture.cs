namespace SpireProfiler;

internal static class ModifierCapture
{
    internal static int Additive(decimal value, bool damage)
        => CaptureRuntime.Backend.CalculateModifierCredit(0, value, damage ? 0 : 1);
    internal static int Increase(decimal basis, decimal multiplier)
        => CaptureRuntime.Backend.CalculateModifierCredit(basis, multiplier, 2);
    internal static int Product(decimal basis, decimal delta)
        => CaptureRuntime.Backend.CalculateModifierCredit(basis, delta, 3);
}
