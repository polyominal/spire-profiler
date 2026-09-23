namespace SpireProfiler;

internal static class ModifierCapture
{
    internal static int Additive(decimal value, bool damage)
        => CaptureRuntime.Backend.CalculateModifierCredit(0, value, damage ? 0 : 1, 0);
    internal static int Increase(decimal basis, decimal multiplier, decimal result)
        => CaptureRuntime.Backend.CalculateModifierCredit(basis, multiplier, 2, result);
    internal static int Product(decimal basis, decimal delta, decimal result)
        => CaptureRuntime.Backend.CalculateModifierCredit(basis, delta, 3, result);
}
