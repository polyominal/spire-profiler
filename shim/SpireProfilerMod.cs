using System;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using HarmonyLib;
using MegaCrit.Sts2.Core.Debug;
using MegaCrit.Sts2.Core.Helpers;
using MegaCrit.Sts2.Core.Logging;
using MegaCrit.Sts2.Core.Modding;
using MegaCrit.Sts2.Core.Runs;

namespace SpireProfiler;

[ModInitializer("Initialize")]
public static class SpireProfilerMod
{
    private static bool initialized;

    public static void Initialize()
    {
        if (initialized) return;
        initialized = true;
        try
        {
            string assemblyDirectory = Path.GetDirectoryName(typeof(SpireProfilerMod).Assembly.Location);
            string modDirectory = !string.IsNullOrEmpty(assemblyDirectory) ? assemblyDirectory
                : Path.Combine(AppContext.BaseDirectory, "mods", "spire-profiler");
            string dataOverride = Environment.GetEnvironmentVariable("SPIRE_PROFILER_DATA_DIR");
            string dataDirectory = !string.IsNullOrEmpty(dataOverride) ? dataOverride
                : Path.Combine(Path.GetDirectoryName(Path.GetDirectoryName(modDirectory)) ?? ".", "mod_data", "spire-profiler");
            string gameVersion = ReleaseInfoManager.Instance.ReleaseInfo?.Version ?? "unknown";
            string modVersion = "unknown";
            string manifest = Path.Combine(modDirectory, "manifest.json");
            if (File.Exists(manifest))
            {
                using var document = JsonDocument.Parse(File.ReadAllText(manifest));
                if (document.RootElement.TryGetProperty("version", out var version)) modVersion = version.GetString() ?? "unknown";
            }
            ProfilerNative.Load(Path.Combine(modDirectory, NativeLibrarySelector.FileName()));
            ProfilerSession.Initialize(dataDirectory, gameVersion, modVersion, text => Log.Error($"[SpireProfiler] {text}"));
            CaptureRuntime.Initialize(new NativeAttributionBackend());

            var harmony = new Harmony("dev.spireprofiler");
            foreach (var type in typeof(SpireProfilerMod).Assembly.GetTypes())
            {
                if (type.GetCustomAttributes(typeof(HarmonyPatch), false).Length == 0) continue;
                try { harmony.CreateClassProcessor(type).Patch(); }
                catch (Exception error) { Log.Error($"[SpireProfiler] Harmony patch failed for {type.Name}: {error.Message}"); }
            }
            FlowCapture.Install(harmony, text => Log.Info($"[SpireProfiler] {text}"));
            var methods = Harmony.GetAllPatchedMethods().Where(method => Harmony.GetPatchInfo(method)?.Owners.Contains(harmony.Id) == true).ToArray();
            Log.Info($"[SpireProfiler] OWN PATCHES owner={harmony.Id} methods={methods.Length}");
            foreach (string name in new[] { "SetUpNewSingleplayer", "SetUpNewMultiplayer", "SetUpSavedSingleplayer", "SetUpSavedMultiplayer", "CleanUp" })
                if (!methods.Any(method => method.DeclaringType == typeof(RunManager) && method.Name == name))
                    Log.Error($"[SpireProfiler] expected patch missing: RunManager.{name}");
            bool selfTest = CommandLineHelper.HasArg("spire-profiler-self-test");
            if (selfTest) ProfilerSession.SelfTest(text => Log.Info(text));
            _ = AttachPanels(modDirectory, selfTest);
        }
        catch (Exception error) { Log.Error($"[SpireProfiler] initialization failed: {error}"); }
    }

    private static async Task AttachPanels(string modDirectory, bool selfTest)
    {
        try
        {
            await ProfilerPanels.AttachPanelAsync(modDirectory);
            if (selfTest) await ProfilerPanels.SelfTestAsync();
        }
        catch (Exception error) { Log.Error($"[SpireProfiler] managed panel initialization failed: {error}"); }
    }
}
