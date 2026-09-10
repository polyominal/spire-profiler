using System;
using System.IO;
using System.Linq;
using HarmonyLib;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Modding;
using MegaCrit.Sts2.Core.Runs;

namespace SpireProfiler;

/// <summary>
/// Required by the StS2 mod loader (types with [ModInitializer] are found
/// and their static initializer invoked). The profiler's real work happens
/// in the native core; this shim loads it and forwards game events
/// through the C ABI.
/// </summary>
[ModInitializer("Initialize")]
public static class SpireProfilerMod
{
    private static bool _initialized;

    public static void Initialize()
    {
        if (_initialized) return;
        _initialized = true;
        try
        {
            var assemblyDir = Path.GetDirectoryName(typeof(SpireProfilerMod).Assembly.Location);
            var modDir = !string.IsNullOrEmpty(assemblyDir)
                ? assemblyDir
                : Path.Combine(AppContext.BaseDirectory, "mods", "spire-profiler");
            // The mod bundle lives at <game dir>/mods/spire-profiler, so one
            // parent hop lands inside mods/. Climb two hops to the game dir:
            // the data files must sit at <game dir>/mod_data/spire-profiler,
            // the sibling of mods/ — the mod scanner sweeps every *.json
            // under mods/ as a manifest (see docs/game.md). SPIRE_PROFILER_DATA_DIR overrides the location:
            // headless-test points it at a scratch dir so its self-test
            // records never mix into real play data.
            var modsDir = Path.GetDirectoryName(modDir) ?? ".";
            var dataDirOverride = System.Environment.GetEnvironmentVariable("SPIRE_PROFILER_DATA_DIR");
            var dataDir = !string.IsNullOrEmpty(dataDirOverride)
                ? dataDirOverride
                : Path.Combine(Path.GetDirectoryName(modsDir) ?? ".", "mod_data", "spire-profiler");

            ProfilerNative.Load(Path.Combine(modDir, NativeLibrarySelector.FileName()));
            ProfilerNative.Init(dataDir);
            CaptureRuntime.Initialize(new NativeAttributionBackend());
            Log.Info($"[SpireProfiler] native core loaded; mod dir: {modDir}; data dir: {dataDir}");

            var harmony = new Harmony("dev.spireprofiler");
            foreach (var type in typeof(SpireProfilerMod).Assembly.GetTypes())
            {
                if (type.GetCustomAttributes(typeof(HarmonyPatch), false).Length == 0) continue;
                try
                {
                    harmony.CreateClassProcessor(type).Patch();
                }
                catch (Exception ex)
                {
                    Log.Error($"[SpireProfiler] Harmony patch failed for {type.Name}: {ex.Message}");
                }
            }
            FlowCapture.Install(harmony, message => Log.Info($"[SpireProfiler] {message}"));
            var ownedMethods = Harmony.GetAllPatchedMethods().Where(method => Harmony.GetPatchInfo(method)?.Owners.Contains(harmony.Id) == true).ToArray();
            Log.Info($"[SpireProfiler] OWN PATCHES owner={harmony.Id} methods={ownedMethods.Length}");
            // Version-drift tripwire: these five gate the run lifecycle;
            // if a game update renames them, runs would silently go unrecorded.
            foreach (var name in new[] { "SetUpNewSingleplayer", "SetUpNewMultiplayer", "SetUpSavedSingleplayer", "SetUpSavedMultiplayer", "CleanUp" })
            {
                if (!ownedMethods.Any(m => m.DeclaringType == typeof(RunManager) && m.Name == name))
                    Log.Error($"[SpireProfiler] expected patch missing: RunManager.{name}");
            }

            // The self-test only runs on request (see the headless-test build
            // step) so normal play never pollutes the data files. Use the
            // game's own arg parser (Environment.GetCommandLineArgs is
            // unreliable under Godot's Mono bootstrap).
            if (CommandLineHelper.HasArg("spire-profiler-self-test"))
            {
                ProfilerNative.SelfTest();
                Log.Info("[SpireProfiler] self-test sequence sent to native core");
            }

            // In-game panel: load the GDExtension and attach the panel to
            // the scene tree once it exists (async, so a slow tree doesn't
            // block mod initialization).
            _ = ProfilerPanels.AttachPanelAsync(modDir);
        }
        catch (Exception ex)
        {
            Log.Error($"[SpireProfiler] initialization failed: {ex}");
        }
    }
}
