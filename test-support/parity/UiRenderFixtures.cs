using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Godot;

namespace SpireProfiler;

internal static class UiRenderFixtures
{
    private const string Baseline = "c928c477852e75ffcc35f8cd16c7ed03caad7c7f";
    private static readonly JsonSerializerOptions InputJson = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
    private static readonly JsonSerializerOptions OutputJson = new() { WriteIndented = true };

    internal static async Task RunAsync(string referencePath, string outputDirectory)
    {
        var reference = JsonNode.Parse(File.ReadAllText(referencePath));
        if (reference["baseline"].GetValue<string>() != Baseline || reference["schema"].GetValue<int>() != 1)
            throw new InvalidOperationException("Rendered parity requires the pinned original reference");
        string scriptPath = Path.Combine(Path.GetDirectoryName(referencePath), "baseline_render.gd");
        using var script = new GDScript { SourceCode = File.ReadAllText(scriptPath) };
        if (script.Reload() != Error.Ok) throw new InvalidOperationException("Original renderer GDScript did not compile");
        Directory.CreateDirectory(outputDirectory);
        var tree = Engine.GetMainLoop() as SceneTree ?? throw new InvalidOperationException("Rendered parity needs the real game scene tree");
        var results = new List<object>();
        var failures = new List<string>();
        foreach (var fixture in reference["render_cases"].AsArray())
        {
            string name = fixture["name"].GetValue<string>();
            int width = (int)fixture["viewport"][0].GetValue<float>();
            int height = (int)fixture["viewport"][1].GetValue<float>();
            var viewport = new SubViewport
            {
                Size = new Vector2I(width, height),
                Disable3D = true,
                TransparentBg = false,
                RenderTargetUpdateMode = SubViewport.UpdateMode.Always,
            };
            ProfilerPanel panel = null;
            Control original = null;
            using var theme = new PanelTheme();
            try
            {
                tree.Root.AddChild(viewport);
                var background = new ColorRect { Color = new Color(.12f, .16f, .2f), Size = new Vector2(width, height) };
                viewport.AddChild(background);
                bool flat = fixture["flat_chrome"].GetValue<bool>();
                if (flat)
                {
                    // Simulate failed optional assets without changing the production renderer.
                    foreach (string field in new[] { "_plate", "_tab", "_stroke", "_track" })
                        typeof(PanelTheme).GetField(field, BindingFlags.Instance | BindingFlags.NonPublic).SetValue(theme, null);
                }
                else if (!theme.HasPlate || !theme.HasScrollbar || !theme.HasTabs)
                    throw new InvalidOperationException("Required game rendering assets were unavailable");
                panel = new ProfilerPanel(theme, fixture["history"].GetValue<bool>());
                viewport.AddChild(panel.Backdrop);
                viewport.AddChild(panel.Root);
                var view = fixture["view"]?.Deserialize<SummaryView>(InputJson);
                panel.Show();
                panel.Refresh(1, view, view);
                if (fixture["tab"].GetValue<string>() == "Run") panel.SelectTab(UiTab.Run);
                if (fixture["player"] is { } selected)
                {
                    panel.SelectPlayer(selected.GetValue<int>());
                    long start = Stopwatch.GetTimestamp();
                    do
                    {
                        await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                        panel.Refresh(1, view, view);
                    } while (Stopwatch.GetElapsedTime(start).TotalMilliseconds < 100);
                }
                float requestedScroll = fixture["scroll"].GetValue<float>();
                var center = new UiPoint(panel.ControlRect.X + 10, panel.ControlRect.Y + 10);
                panel.QueueScroll(requestedScroll);
                panel.Interact(center, false);
                if (fixture["hover_row"] is { } hovered)
                {
                    int index = hovered.GetValue<int>();
                    var row = panel.Layout.RowHits.Single(hit => hit.FlatIndex == index);
                    panel.Interact(new(panel.ControlRect.X + 100, panel.ControlRect.Y + (row.Y0 + row.Y1) / 2 - panel.ScrollPosition), false);
                }
                else panel.Interact(new(-1, -1), false);
                await DrawFrame(tree);
                if (panel.DrawCount == 0) throw new InvalidOperationException("Managed panel did not dispatch its actual draw signals");
                using var actual = viewport.GetTexture().GetImage();
                actual.Convert(Image.Format.Rgba8);
                Save(actual, Path.Combine(outputDirectory, name + ".managed.png"));
                panel.Free();
                panel = null;
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
                original = new Control();
                original.SetScript(script);
                original.Call("configure", fixture.ToJsonString(), reference["render_contract"].ToJsonString());
                viewport.AddChild(original);
                await DrawFrame(tree);
                using var expected = viewport.GetTexture().GetImage();
                expected.Convert(Image.Format.Rgba8);
                Save(expected, Path.Combine(outputDirectory, name + ".baseline.png"));
                if (actual.GetWidth() != width || actual.GetHeight() != height || expected.GetSize() != actual.GetSize())
                    throw new InvalidOperationException("SubViewport capture dimensions differ from the virtual canvas");
                var managedBytes = actual.GetData();
                var baselineBytes = expected.GetData();
                byte[] difference = new byte[managedBytes.Length];
                long differingPixels = 0, totalDifference = 0;
                int maximumDifference = 0;
                for (int pixel = 0; pixel < difference.Length; pixel += 4)
                {
                    bool differs = false;
                    for (int channel = 0; channel < 4; channel++)
                    {
                        int delta = Math.Abs(managedBytes[pixel + channel] - baselineBytes[pixel + channel]);
                        differs |= delta != 0;
                        totalDifference += delta;
                        maximumDifference = Math.Max(maximumDifference, delta);
                        if (channel < 3) difference[pixel + channel] = (byte)Math.Min(255, delta * 8);
                    }
                    difference[pixel + 3] = 255;
                    if (differs) differingPixels++;
                }
                using var heatmap = Image.CreateFromData(width, height, false, Image.Format.Rgba8, difference);
                Save(heatmap, Path.Combine(outputDirectory, name + ".difference.png"));
                results.Add(new { name, width, height, differingPixels, maximumDifference, totalDifference });
                if (differingPixels != 0) failures.Add(name);
                GD.Print(string.Create(CultureInfo.InvariantCulture, $"UI_RENDER_CASE {name} differing_pixels={differingPixels} max_channel_delta={maximumDifference} total_delta={totalDifference}"));
            }
            finally
            {
                panel?.Free();
                if (original != null && GodotObject.IsInstanceValid(original)) original.QueueFree();
                viewport.QueueFree();
                await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
            }
        }
        File.WriteAllText(Path.Combine(outputDirectory, "report.json"), JsonSerializer.Serialize(new
        {
            baseline = Baseline,
            renderer = "actual managed ProfilerPanel versus independent original-command GDScript",
            exact = failures.Count == 0,
            cases = results
        }, OutputJson) + "\n");
        if (failures.Count != 0) throw new InvalidOperationException("Rendered parity differs: " + string.Join(", ", failures));
        GD.Print($"UI_RENDER_PASS baseline={Baseline} cases={results.Count} exact_rgba=true");
    }

    private static async Task DrawFrame(SceneTree tree)
    {
        await tree.ToSignal(tree, SceneTree.SignalName.ProcessFrame);
        await tree.ToSignal(RenderingServer.Singleton, RenderingServer.SignalName.FramePostDraw);
    }

    private static void Save(Image image, string path)
    {
        var error = image.SavePng(path);
        if (error != Error.Ok) throw new IOException($"Could not save rendered fixture {path}: {error}");
    }
}
