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
    private enum Installation { Uninitialized, Installing, Active, Failed }
    private static Installation installation;

    public static void Initialize()
    {
        if (installation != Installation.Uninitialized) return;
        installation = Installation.Installing;
        var harmony = new Harmony("dev.spireprofiler");
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

            foreach (var type in typeof(SpireProfilerMod).Assembly.GetTypes())
                if (type.GetCustomAttributes(typeof(HarmonyPatch), false).Length != 0)
                    harmony.CreateClassProcessor(type).Patch();
            FlowCapture.Install(harmony, text => Log.Info($"[SpireProfiler] {text}"));
            var methods = Harmony.GetAllPatchedMethods().Where(method => Harmony.GetPatchInfo(method)?.Owners.Contains(harmony.Id) == true).ToArray();
            foreach (string name in new[] { "SetUpNewSingleplayer", "SetUpNewMultiplayer", "SetUpSavedSingleplayer", "SetUpSavedMultiplayer", "CleanUp", "OnEnded" })
                if (!methods.Any(method => method.DeclaringType == typeof(RunManager) && method.Name == name))
                    throw new InvalidOperationException($"Expected patch missing: RunManager.{name}");
            Log.Info($"[SpireProfiler] OWN PATCHES owner={harmony.Id} methods={methods.Length}");
            installation = Installation.Active;
            bool selfTest = CommandLineHelper.HasArg("spire-profiler-self-test");
            if (selfTest) ProfilerSession.SelfTest(text => Log.Info(text));
            _ = AttachPanels(modDirectory, selfTest);
        }
        catch (Exception error)
        {
            installation = Installation.Failed;
            try
            {
                CaptureRuntime.InvalidateEpoch();
                foreach (var method in Harmony.GetAllPatchedMethods().ToArray())
                    try
                    {
                        if (Harmony.GetPatchInfo(method)?.Owners.Contains(harmony.Id) == true)
                            harmony.Unpatch(method, HarmonyPatchType.All, harmony.Id);
                    }
                    catch (Exception rollback) { Log.Error($"[SpireProfiler] patch rollback failed: {rollback}"); }
            }
            catch (Exception rollback) { Log.Error($"[SpireProfiler] installation rollback failed: {rollback}"); }
            finally
            {
                try { ProfilerNative.Dispose(); }
                catch (Exception rollback) { Log.Error($"[SpireProfiler] native disposal failed: {rollback}"); }
            }
            Log.Error($"[SpireProfiler] initialization failed; capture disabled: {error}");
        }
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
