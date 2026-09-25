using System;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;

namespace SpireProfiler;

internal static class ManagedTestProgram
{
    private static int Main(string[] args)
    {
        if (args.Length != 3) throw new ArgumentException("Expected game assemblies, project directory, and native engine library");
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
        try
        {
            ProfilerNative.Load(args[2]);
            ManagedFixtures.RunAttributionParity(args[1]);
            ManagedFixtures.RunTemporalParity();
            UiParityFixtures.Run(Path.Combine(args[1], "parity", "ui_reference.json"));
            ProfilerNative.Dispose();
            ProfilerNative.Load(args[2]);
            SessionParityFixtures.Run(Path.Combine(args[1], "parity", "session_reference.json"), Path.Combine(args[1], "session-parity"));
            ProfilerNative.Dispose();
            Console.WriteLine("MANAGED BASELINE PARITY PASS");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
    }
}
