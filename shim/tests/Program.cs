using System;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;

namespace SpireProfiler;

internal static class ManagedTestProgram
{
    private static int Main(string[] args)
    {
        if (args.Length != 2) throw new ArgumentException("Expected installed managed assembly directory and generated project directory");
        AppDomain.CurrentDomain.AssemblyResolve += (_, request) =>
        {
            var path = Path.Combine(args[0], new AssemblyName(request.Name).Name + ".dll");
            return File.Exists(path) ? Assembly.LoadFrom(path) : null;
        };
        return Run(args);
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static int Run(string[] args)
    {
        try { ManagedFixtures.Run(args[0], args[1]); Console.WriteLine("MANAGED CAPTURE FIXTURES PASS"); return 0; }
        catch (Exception ex) { Console.Error.WriteLine(ex); return 1; }
    }
}
