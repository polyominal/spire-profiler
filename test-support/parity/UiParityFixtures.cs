using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace SpireProfiler;

internal static class UiParityFixtures
{
    private const string Baseline = "c928c477852e75ffcc35f8cd16c7ed03caad7c7f";
    internal static void Run(string referencePath)
    {
        var reference = JsonNode.Parse(File.ReadAllText(referencePath));
        if (Text(reference, "baseline") != Baseline || Number(reference, "schema") != 1)
            throw new InvalidOperationException("UI oracle baseline or schema changed");
        var failures = new List<string>();
        int cases = 0;
        foreach (var fixture in reference["cases"].AsArray())
        {
            string name = Text(fixture, "name");
            try { Equal(fixture["expected"], Evaluate(Text(fixture, "kind"), fixture["input"]), name); }
            catch (Exception ex) { failures.Add(name + ": " + ex.Message); }
            cases++;
        }
        if (failures.Count != 0) throw new InvalidOperationException($"UI parity failed in {failures.Count}/{cases} reference cases:\n" + string.Join("\n", failures));
        Console.WriteLine($"UI PARITY PASS baseline={Baseline} cases={cases}");
    }
    internal static JsonNode Evaluate(string kind, JsonNode input) => kind switch
    {
        "snapshot" => Snapshot(input),
        "state" => State(input),
        "chart" => Chart(input),
        "history" => History(input),
        "geometry" => Geometry(input),
        "hover" => Hover(input),
        "scroll_events" => Node(input.AsArray().Select(item => PanelGeometry.EventScrollDelta(Number(item, "button"), Flag(item, "pressed"), Float(item, "pan"))).ToArray()),
        "scroll_drag_state" => Node(input.AsArray().Select(item => PanelGeometry.DragState(item[0].GetValue<bool>(), item[1].GetValue<bool>(), item[2].GetValue<bool>(), item[3].GetValue<bool>())).ToArray()),
        "scroll_clamps" => Node(input.AsArray().Select(item => PanelGeometry.ApplyScroll(item[0].GetValue<float>(), item[1].GetValue<float>(), item[2].GetValue<float>(), item[3].GetValue<float>())).ToArray()),
        _ => throw new InvalidOperationException("Unrecognized reference case kind: " + kind)
    };
    private static JsonNode Snapshot(JsonNode input)
    {
        var cards = Cards(input["cards"]);
        var rows = ChartProjection.Rows(cards);
        return Node(new { rows = rows.Select(Row).ToArray(), details = Details(rows, cards) });
    }
    private static JsonNode State(JsonNode input)
    {
        var cards = Cards(input["cards"]);
        var tab = Enum.Parse<UiTab>(Text(input, "tab"));
        var view = new SummaryView
        {
            Cards = cards,
            Title = Text(input, "encounter"),
            Turns = (uint)Number(input, tab == UiTab.Run ? "run_turns" : "turns"),
            Plays = (uint)Number(input, "plays"),
            Combats = tab == UiTab.Run ? (uint)Number(input, "run_combats") : 0,
            DamageReceived = tab == UiTab.Run ? 0 : Number(input, "damage_received"),
            BlockTotal = Number(input, "block_total"),
            PotionsUsed = (uint)Number(input, "potions_used")
        };
        var rows = ChartProjection.Rows(cards, OptionalInt(input, "player"));
        return Node(new { rows = rows.Select(Row).ToArray(), meta = Meta(ChartProjection.Meta(view, tab)), footer = ChartProjection.Footer(view, tab), details = Details(rows, cards) });
    }
    private static JsonNode Chart(JsonNode input)
    {
        var layout = PanelLayout.Chart(Enum.Parse<UiTab>(Text(input, "tab")), ReadRows(input["rows"]), ReadMeta(input["meta"]), Text(input, "footer"),
            OptionalInt(input, "hover_row"), Flag(input, "skip_chrome"), Avatars(input["avatars"]), Flag(input, "flat_chrome"), Flag(input, "tab_sprites"), Float(input, "width"), Float(input, "right_gutter"));
        return ChartLayout(layout);
    }
    private static JsonNode History(JsonNode input)
    {
        var allCards = Cards(input["cards"]);
        var selected = input["player_rollups"].AsArray().FirstOrDefault(item => Number(item, "slot") == OptionalInt(input, "player"));
        var cards = selected == null ? allCards : Cards(selected["cards"]);
        var view = new SummaryView
        {
            Cards = cards,
            Title = "Run Summary",
            Ascension = (int)Number(input, "ascension"),
            GameMode = Text(input, "game_mode"),
            Outcome = Text(input, "outcome"),
            Seed = Text(input, "seed"),
            Turns = (uint)input["combats"].AsArray().Sum(item => Number(item, "turns")),
            Combats = (uint)input["combats"].AsArray().Count,
            DamageReceived = input["combats"].AsArray().Sum(item => Number(item, "damage_taken"))
        };
        bool missing = Flag(input, "missing");
        var rows = missing ? Array.Empty<ChartRow>() : ChartProjection.Rows(cards);
        var layout = PanelLayout.History(missing ? null : view, rows, ChartProjection.Meta(view, UiTab.Run), Avatars(input["portraits"]),
            OptionalInt(input, "hover_row"), Float(input, "width"), Flag(input, "flat_chrome"), Float(input, "right_gutter"));
        return Node(new
        {
            width = layout.Width,
            height = layout.Height,
            header_bottom = layout.HeaderBottom,
            content = Content(layout.Content),
            has_chart = layout.HasChart,
            chart_rows = layout.RowHits.Count,
            header = layout.Header.Select(Command).ToArray(),
            body = layout.Body.Select(Command).ToArray(),
            portrait_paths = layout.PortraitPaths,
            avatar_hits = layout.AvatarHits.Select(AvatarHit).ToArray(),
            row_hits = layout.RowHits.Select(RowHit).ToArray(),
            rows = rows.Select(Row).ToArray(),
            details = Details(rows, cards)
        });
    }
    private static JsonNode Geometry(JsonNode input)
    {
        var viewport = new UiPoint(input["viewport"][0].GetValue<float>(), input["viewport"][1].GetValue<float>());
        float contentHeight = Float(input, "content_height");
        var size = new UiPoint(Float(input, "width"), Math.Min(PanelGeometry.HeightCap(viewport.Y), contentHeight));
        var position = PanelGeometry.Center(viewport, size);
        var panel = new UiRect(position.X, position.Y, size.X, size.Y);
        float scroll = Math.Min(Float(input, "requested_scroll"), Math.Max(0, contentHeight - size.Y));
        bool plate = Flag(input, "plate");
        var band = PanelGeometry.BodyBand(size.Y, plate, Float(input, "header_bottom"));
        var scrollbar = PanelGeometry.Scrollbar(size, plate, band, contentHeight, scroll);
        var legendPlate = PanelGeometry.LegendPlate(plate);
        var legend = PanelGeometry.PlaceLegend(viewport, panel, legendPlate.Size);
        var tip = PanelGeometry.PlaceTip(viewport, panel, position.Y + Float(input, "row_offset"), new UiPoint(360, TooltipLayout.Height((int)Number(input, "tip_lines"))), legend);
        var frame = PanelGeometry.Frame(panel, legend, tip);
        var legendCommands = PanelLayout.Legend(legendPlate.Origin);
        return Node(new
        {
            panel = Rect(panel),
            scroll,
            band = new[] { band.Top, band.Bottom },
            gutter = contentHeight > size.Y ? 32 : 0,
            scrollbar = scrollbar == null ? null : new { track = Rect(scrollbar.Track), body = Rect(scrollbar.Body), cap_top = Rect(scrollbar.CapTop), cap_bottom = Rect(scrollbar.CapBottom), grabber = Rect(scrollbar.Grabber) },
            legend = Rect(legend),
            tip = Rect(tip),
            frame = Rect(frame.Rect),
            origin_x = frame.OriginX,
            legend_commands = legendCommands.Select(Command).ToArray()
        });
    }
    private static JsonNode Hover(JsonNode input)
    {
        var p = input["panel"];
        var panel = new UiRect(Float(p, "x"), Float(p, "y"), Float(p, "w"), Float(p, "h"));
        var hits = input["hits"].AsArray().Select(h => new RowHit(Float(h, "y0"), Float(h, "y1"), (int)Number(h, "flat_index"))).ToArray();
        var band = (input["band"][0].GetValue<float>(), input["band"][1].GetValue<float>());
        return Node(input["queries"].AsArray().Select(query =>
        {
            var mouse = new UiPoint(query["mouse"][0].GetValue<float>(), query["mouse"][1].GetValue<float>());
            return new { over = panel.Contains(mouse.X, mouse.Y), hover = PanelGeometry.Hover(hits, panel, mouse, Float(query, "scroll"), band) };
        }).ToArray());
    }
    private static JsonNode ChartLayout(PanelLayout layout) => Node(new
    {
        width = layout.Width,
        height = layout.Height,
        header_bottom = layout.HeaderBottom,
        strip_h = layout.StripH,
        content = Content(layout.Content),
        header = layout.Header.Select(Command).ToArray(),
        body = layout.Body.Select(Command).ToArray(),
        row_hits = layout.RowHits.Select(RowHit).ToArray(),
        tab_hits = layout.TabHits.Select(hit => new { x0 = hit.X0, y0 = hit.Y0, x1 = hit.X1, y1 = hit.Y1, tab = hit.Tab.ToString() }).ToArray(),
        avatar_hits = layout.AvatarHits.Select(AvatarHit).ToArray(),
        portrait_paths = layout.PortraitPaths
    });
    private static object RowHit(RowHit hit) => new { y0 = hit.Y0, y1 = hit.Y1, flat_index = hit.FlatIndex };
    private static object AvatarHit(AvatarHit hit) => new { x0 = hit.X0, y0 = hit.Y0, x1 = hit.X1, y1 = hit.Y1, slot = hit.Slot };
    private static object Content(ContentBox content) => new { x = content.X, top = content.Top, w = content.W, outer_bottom_pad = content.OuterBottomPad };
    private static object Rect(UiRect rectangle) => new { x = rectangle.X, y = rectangle.Y, w = rectangle.W, h = rectangle.H };
    private static float[] Color(UiColor color) => new[] { color.R, color.G, color.B, color.A };
    private static object Command(DrawCommand command) => command switch
    {
        RectCommand r => new { type = "rect", x = r.X, y = r.Y, w = r.W, h = r.H, color = Color(r.Color) },
        TextureCommand t => new { type = "texture", x = t.X, y = t.Y, w = t.W, h = t.H, icon = t.Icon.ToString(), slot = t.Icon == IconId.Character ? (int?)t.Slot : null },
        TextCommand t => new
        {
            type = "text",
            x = t.X,
            y = t.Y,
            size = t.Size,
            color = Color(t.Color),
            role = t.Role.ToString(),
            effect = t.Effect.ToString(),
            align = t.Align == TextAlign.Left ? (object)new { kind = "Left" } : new { kind = t.Align.ToString(), width = t.Width },
            text = t.Text
        },
        _ => throw new InvalidOperationException("Unknown managed command")
    };
    private static object Row(ChartRow row) => new
    {
        section = row.Section.ToString(),
        kind = row.Source.Kind,
        player = row.Source.Player,
        flags = row.Flags,
        id = row.Name,
        name_len = Encoding.UTF8.GetByteCount(row.Name),
        plays = row.Source.Plays,
        value = row.Value,
        share_x10 = row.ShareX10,
        seg_milli = row.SegMilli
    };
    private static object Meta(ChartMeta meta) => new
    {
        turns = meta.Turns,
        plays = meta.Plays,
        combats = meta.Combats,
        total_damage = meta.TotalDamage,
        damage_taken = meta.DamageTaken,
        dps_x10 = meta.DpsX10,
        encounter = meta.Encounter
    };
    private static object[] Details(IReadOnlyList<ChartRow> rows, IReadOnlyList<StatRow> cards) => Enumerable.Range(0, rows.Count).Select(index =>
    {
        var detail = ChartProjection.Detail(rows, index, cards);
        return (object)new { index, title = detail.Title, lines = TooltipLayout.Shape(detail, 64).Select(Line).ToArray(), truncated_lines = TooltipLayout.Shape(detail, 3).Select(Line).ToArray() };
    }).ToArray();
    private static object Line(TipLine line) => new { text = line.Text, title = line.Title, color = Color(line.Color), value = line.Value == null ? null : new { text = line.Value.Text, color = Color(line.Value.Color) } };
    private static StatRow[] Cards(JsonNode array) => array.AsArray().Select(c => new StatRow
    {
        Id = Text(c, "id"),
        Player = (int)Number(c, "player"),
        Kind = (int)Number(c, "kind"),
        Plays = (uint)Number(c, "plays"),
        DamageDealt = Number(c, "damage_dealt"),
        DamageBlocked = Number(c, "damage_blocked"),
        BlockGained = Number(c, "block_gained"),
        BlockEffective = Number(c, "block_effective"),
        DmgDirect = Number(c, "dmg_direct"),
        DmgAttributed = Number(c, "dmg_attributed"),
        DmgModifier = Number(c, "dmg_modifier"),
        BlkModifier = Number(c, "blk_modifier"),
        MitigateDebuff = Number(c, "mitigate_debuff"),
        MitigateBuff = Number(c, "mitigate_buff"),
        MitigateStr = Number(c, "mitigate_str"),
        SelfDamage = Number(c, "self_damage"),
        Forge = Number(c, "forge")
    }).ToArray();
    private static ChartRow[] ReadRows(JsonNode array) => array.AsArray().Select(row => new ChartRow(
        new StatRow { Id = Text(row, "id"), Kind = (int)Number(row, "kind"), Player = (int)Number(row, "player"), Plays = (uint)Number(row, "plays") },
        Enum.Parse<ChartSection>(Text(row, "section")), (int)Number(row, "flags"), Text(row, "id"), Number(row, "value"), (int)Number(row, "share_x10"),
        row["seg_milli"].AsArray().Select(value => value.GetValue<int>()).ToArray())).ToArray();
    private static ChartMeta ReadMeta(JsonNode meta) => new((uint)Number(meta, "turns"), (uint)Number(meta, "plays"), (uint)Number(meta, "combats"), Number(meta, "total_damage"), Number(meta, "damage_taken"), (int)Number(meta, "dps_x10"), Text(meta, "encounter"));
    private static AvatarFact[] Avatars(JsonNode array) => array.AsArray().Select(avatar => new AvatarFact((int)Number(avatar, "slot"), Flag(avatar, "loaded"), Text(avatar, "path"))).ToArray();
    private static string Text(JsonNode node, string field) => node[field].GetValue<string>();
    private static long Number(JsonNode node, string field) => node[field].GetValue<long>();
    private static float Float(JsonNode node, string field) => node[field].GetValue<float>();
    private static bool Flag(JsonNode node, string field) => node[field].GetValue<bool>();
    private static int? OptionalInt(JsonNode node, string field) => node[field]?.GetValue<int>();
    private static JsonNode Node<T>(T value) => JsonSerializer.SerializeToNode(value);
    private static void Equal(JsonNode expected, JsonNode actual, string path)
    {
        if (expected == null || actual == null)
        {
            if (expected != null || actual != null) throw new InvalidOperationException(path + ": null mismatch");
            return;
        }
        if (expected is JsonObject objectExpected)
        {
            if (actual is not JsonObject objectActual || objectExpected.Count != objectActual.Count)
                throw new InvalidOperationException(path + ": object fields differ");
            foreach (var (name, value) in objectExpected)
            {
                if (!objectActual.ContainsKey(name)) throw new InvalidOperationException(path + ": missing " + name);
                Equal(value, objectActual[name], path + "." + name);
            }
        }
        else if (expected is JsonArray arrayExpected)
        {
            if (actual is not JsonArray arrayActual || arrayExpected.Count != arrayActual.Count)
                throw new InvalidOperationException(path + $": array lengths differ ({arrayExpected.Count} expected, {(actual as JsonArray)?.Count} actual)");
            for (int i = 0; i < arrayExpected.Count; i++) Equal(arrayExpected[i], arrayActual[i], path + "[" + i + "]");
        }
        else if (expected.GetValueKind() == JsonValueKind.Number && actual.GetValueKind() == JsonValueKind.Number)
        {
            decimal wanted = decimal.Parse(expected.ToJsonString(), System.Globalization.CultureInfo.InvariantCulture);
            decimal got = decimal.Parse(actual.ToJsonString(), System.Globalization.CultureInfo.InvariantCulture);
            bool integer = !expected.ToJsonString().Contains('.', StringComparison.Ordinal);
            if (integer ? wanted != got : Math.Abs(wanted - got) > 0.0001m)
                throw new InvalidOperationException(path + $": expected {wanted}, actual {got}");
        }
        else if (!JsonNode.DeepEquals(expected, actual)) throw new InvalidOperationException(path + ": expected " + expected + ", actual " + actual);
    }
}
