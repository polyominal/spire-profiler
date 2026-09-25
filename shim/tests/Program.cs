using System;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;

namespace SpireProfiler;

internal static class ManagedTestProgram
{
    private static int Main(string[] args)
    {
        if (args.Length != 3) throw new ArgumentException("Expected game assemblies, generated project directory, and native engine library");
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
            ManagedFixtures.Run(args[1]);
            ProfilerNative.Dispose();
            PanelFixtures.Run();
            SessionFixtures.Run(args[1], args[2]);
            Console.WriteLine("MANAGED CAPTURE FIXTURES PASS");
            return 0;
        }
        catch (Exception ex) { Console.Error.WriteLine(ex); return 1; }
    }
}
