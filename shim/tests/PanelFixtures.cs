using System;
using System.Linq;

namespace SpireProfiler;

internal static class PanelFixtures
{
    internal static void Run()
    {
        var cards = new[]
        {
            new StatRow { Id = "SHARED", Player = 0, DamageDealt = 30, DmgDirect = 10, DmgAttributed = 12, DmgModifier = 8, BlockEffective = 5, BlkModifier = 3, SelfDamage = 20, Forge = 4 },
            new StatRow { Id = "SHARED", Player = 1, Kind = 1, DamageDealt = 15, DmgDirect = 15, BlockEffective = 7, MitigateDebuff = 2, MitigateBuff = 1, MitigateStr = 3 },
            new StatRow { Id = "COST", Player = 0, SelfDamage = 50 },
        };
        var rows = ChartProjection.Rows(cards);
        var damage = rows.Where(row => row.Section == ChartSection.Damage).ToArray();
        if (damage.Length != 2 || damage[0].ShareX10 != 666 || damage[1].ShareX10 != 333)
            throw new InvalidOperationException("Damage shares must truncate to tenths of a percent");
        var own = rows.Single(row => row.Section == ChartSection.Defense && row.Source.Player == 0 && row.Name == "SHARED" && !row.SelfDamage);
        if (own.Value != 8 || own.SegMilli[(int)ChartSegment.Direct] != 100 || own.SegMilli[(int)ChartSegment.Modifier] != 60)
            throw new InvalidOperationException("Defense segments must share a scale that includes displayed self-damage");
        var filtered = ChartProjection.Rows(cards, 1);
        if (filtered.Count != 2 || filtered.Any(row => row.Source.Player != 1) || filtered[0].ShareX10 != 1000)
            throw new InvalidOperationException("Player selection must retain source ownership and independent shares");
        var summary = new SummaryView
        {
            Cards = new[]
            {
                new StatRow { Id = "A", Player = 0, Plays = 4, DamageDealt = 9, DmgDirect = 9 },
                new StatRow { Id = "B", Player = 1, Plays = 2, DamageDealt = 6, DmgDirect = 6 },
            },
            Turns = 4,
            Plays = 7,
            Combats = 2,
            DamageReceived = 3,
        };
        var combatMeta = ChartProjection.Meta(summary, UiTab.Combat);
        var runMeta = ChartProjection.Meta(summary, UiTab.Run);
        var playerMeta = ChartProjection.Meta(summary with { Cards = new[] { summary.Cards[1] } }, UiTab.Run);
        if (combatMeta.Plays != 7 || combatMeta.TotalDamage != 15 || combatMeta.DpsX10 != 37
            || runMeta.Plays != 6 || runMeta.TotalDamage != 15 || runMeta.DpsX10 != 37
            || playerMeta.Plays != 2 || playerMeta.TotalDamage != 6 || playerMeta.DpsX10 != 15
            || playerMeta.Turns != 4 || playerMeta.Combats != 2 || playerMeta.DamageTaken != 3
            || ChartProjection.Meta(summary with { Turns = 0 }, UiTab.Combat).DpsX10 != 0)
            throw new InvalidOperationException("Headlines must distinguish observed combat plays, summed source plays, and selected history totals without filtering run-wide counters");
        int hanging = rows.Select((row, index) => (row, index)).Single(item => item.row.SelfDamage && !item.row.SoloSelf).index;
        var terse = ChartProjection.Detail(rows, hanging, cards);
        if (terse.Stats.Count != 1 || terse.Stats[0].Label != "self dmg" || terse.Stats[0].Value != "20")
            throw new InvalidOperationException("Hanging self-damage detail must stay terse");
        var detail = ChartProjection.Detail(rows, 0, cards);
        if (!detail.Stats.Any(stat => stat.Label == "forge" && stat.Value == "4") || detail.Title != "SHARED x0")
            throw new InvalidOperationException("Raw source IDs and non-chart statistics must remain in tooltips");
        if (PanelGeometry.DragState(false, true, true, true) || !PanelGeometry.DragState(true, true, true, false))
            throw new InvalidOperationException("A held cursor must not start a drag, and active drags must survive leaving the track");
        var scrolling = PanelLayout.Chart(UiTab.Combat, ChartProjection.Rows(Enumerable.Range(0, 12)
            .Select(index => new StatRow { Id = "CARD_" + index, DamageDealt = 1, DmgDirect = 1 }).ToArray()), new(), "");
        var panel = new UiRect(100, 50, scrolling.Width, 400);
        var band = PanelGeometry.BodyBand(panel.H, false, scrolling.HeaderBottom);
        var hit = scrolling.RowHits[5];
        float scroll = hit.Y0 - band.Top;
        if (PanelGeometry.Hover(scrolling.RowHits, panel, new(panel.X + 1, panel.Y + band.Top + 1), scroll, band) != hit.FlatIndex
            || PanelGeometry.Hover(scrolling.RowHits, panel, new(panel.X + 1, panel.Y + band.Top - 1), scroll, band) != null
            || PanelGeometry.Hover(scrolling.RowHits, panel, new(panel.X + 1, panel.Y + band.Bottom), scroll, band) != null
            || PanelGeometry.Hover(scrolling.RowHits, panel, new(panel.X - 1, panel.Y + band.Top + 1), scroll, band) != null)
            throw new InvalidOperationException("Scrolled row hit-testing must transform screen coordinates and exclude the fixed header, footer, and outside panel");
        float maximumScroll = scrolling.Height - panel.H;
        var scrollbar = PanelGeometry.Scrollbar(new(panel.W, panel.H), false, band, scrolling.Height, maximumScroll);
        if (scrollbar == null || scrollbar.Track.Y != band.Top || scrollbar.Track.Y + scrollbar.Track.H != band.Bottom
            || scrollbar.Grabber.Y + scrollbar.Grabber.H != band.Bottom
            || PanelGeometry.ApplyScroll(0, -60, panel.H, scrolling.Height) != 0
            || PanelGeometry.ApplyScroll(maximumScroll, 60, panel.H, scrolling.Height) != maximumScroll)
            throw new InvalidOperationException("Scrolling must stay within content bounds and keep the track below the fixed header");
        var portraits = new[] { new AvatarFact(2, false, "missing"), new AvatarFact(0, true, "loaded") };
        string[] portraitPaths = { "missing", "loaded" };
        foreach (var layout in new[]
        {
            PanelLayout.Chart(UiTab.Combat, rows, new(), "", avatars: portraits),
            PanelLayout.History(summary, rows, new(), portraits),
        })
            if (!layout.PortraitPaths.SequenceEqual(portraitPaths) || layout.AvatarHits.Single().Slot != 0
                || layout.Header.OfType<TextureCommand>().Single(texture => texture.Icon == IconId.Character).Slot != 1)
                throw new InvalidOperationException("A failed portrait must not redirect another player's texture or hit target");
        var animation = new AvatarAnimation();
        int[] slots = { 0, 1 };
        animation.SetTargets(null, slots);
        animation.SetTargets(1, slots);
        animation.AdvanceFrame(10);
        if (animation.Values[1] != 1) throw new InvalidOperationException("Avatar transitions must start at zero elapsed time");
        animation.AdvanceFrame(10.025);
        if (Math.Abs(animation.Values[1] - 1.05f) > .0001f) throw new InvalidOperationException("Avatar transitions must advance on monotonic elapsed time");
        animation.AdvanceFrame(10.1);
        animation.AdvanceFrame(10.2);
        animation.SetTargets(null, slots);
        animation.AdvanceFrame(20);
        if (animation.Values[1] != 1.1f) throw new InvalidOperationException("An idle period must not advance the next avatar transition");
        (StatRow Card, int[] Segments, int? Child, long Value)[] defenseCases =
        {
            (new() { Id = "POSITIVE_NET", BlockEffective = 30, BlkModifier = 20, SelfDamage = 49 }, new[] { 600, 0, 400, 0, 0, 0, 0 }, 980, 50),
            (new() { Id = "ZERO_NET", BlockEffective = 30, BlkModifier = 20, SelfDamage = 50 }, new[] { 600, 0, 400, 0, 0, 0, 0 }, 1000, 50),
            (new() { Id = "NEGATIVE_NET", BlockEffective = 30, BlkModifier = 20, SelfDamage = 70 }, new[] { 428, 0, 285, 0, 0, 0, 0 }, 1000, 50),
            (new() { Id = "NEGATIVE_MODIFIER", BlockEffective = 50, BlkModifier = -20, SelfDamage = 10 }, new[] { 1000, 0, 0, 0, 0, 0, 0 }, 200, 30),
            (new() { Id = "CANCELED_DEFENSE", BlockEffective = 50, BlkModifier = -50, SelfDamage = 10 }, new[] { 833, 0, 0, 0, 0, 0, 166 }, null, -10),
            (new() { Id = "WIDE_DRAWABLE_SUM", BlockEffective = long.MaxValue, BlkModifier = -long.MaxValue, MitigateBuff = long.MaxValue, SelfDamage = 1 }, new[] { 500, 0, 0, 0, 500, 0, 0 }, 0, long.MaxValue),
        };
        foreach (var fixture in defenseCases)
        {
            var projected = ChartProjection.Rows(new[] { fixture.Card });
            if (projected.Count != (fixture.Child.HasValue ? 2 : 1) || projected[0].Value != fixture.Value
                || projected[0].ShareX10 != (fixture.Child.HasValue ? 1000 : 0) || !projected[0].SegMilli.SequenceEqual(fixture.Segments))
                throw new InvalidOperationException("Defense scale changed numeric accounting or drawable proportions: " + fixture.Card.Id);
            if (fixture.Child is { } child && (projected[1].Value != -fixture.Card.SelfDamage
                || projected[1].SegMilli[(int)ChartSegment.SelfDamage] != child || projected[1].ShareX10 != 0))
                throw new InvalidOperationException("Self-damage must retain its own magnitude on the shared defense scale: " + fixture.Card.Id);
            var layout = PanelLayout.Chart(UiTab.Combat, projected, new(), "", skipChrome: true);
            var rectangles = layout.Body.OfType<RectCommand>().ToArray();
            foreach (var track in rectangles.Where(rectangle => rectangle.Color == UiPalette.Track))
                foreach (var fill in rectangles.Where(rectangle => rectangle.Color != UiPalette.Track && rectangle.Y == track.Y && rectangle.H == track.H))
                    if (fill.X < track.X || fill.X + fill.W > track.X + track.W)
                        throw new InvalidOperationException("A defense segment escaped its track: " + fixture.Card.Id);
        }
        var ranked = ChartProjection.Rows(new[]
        {
            new StatRow { Id = "MIXED", BlockEffective = 30, BlkModifier = 20, SelfDamage = 49 },
            new StatRow { Id = "OTHER", BlockEffective = 20 }
        });
        string[] expectedNames = { "OTHER", "MIXED", "MIXED" };
        if (!ranked.Select(row => row.Name).SequenceEqual(expectedNames)
            || ranked[0].Value != 20 || ranked[1].Value != 50 || ranked[2].Value != -49)
            throw new InvalidOperationException("Defense scaling must retain the existing net-value ranking and separate row labels");
        var healthy = new SummaryView { Coverage = CoverageSummary.Healthy, Cards = cards };
        foreach (var coverage in new[] { healthy.Coverage.WithFailure("snapshot-read-failed"), CoverageSummary.Unknown })
        {
            var uncertain = healthy with { Coverage = coverage };
            string warning = coverage.Quality == CaptureQuality.Partial
                ? "Incomplete statistics: totals may omit activity." : "Capture quality unknown: totals are unverified.";
            foreach (var tab in new[] { UiTab.Combat, UiTab.Run })
            {
                var completeLayout = PanelLayout.Chart(tab, rows, ChartProjection.Meta(healthy, tab), "");
                var uncertainLayout = PanelLayout.Chart(tab, rows, ChartProjection.Meta(uncertain, tab), "");
                CheckWarning(completeLayout, uncertainLayout, warning);
            }
            CheckWarning(
                PanelLayout.History(healthy, rows, ChartProjection.Meta(healthy, UiTab.Run), Array.Empty<AvatarFact>()),
                PanelLayout.History(uncertain, rows, ChartProjection.Meta(uncertain, UiTab.Run), Array.Empty<AvatarFact>()), warning);
        }
        var empty = healthy with { Cards = Array.Empty<StatRow>(), Coverage = healthy.Coverage.WithFailure("snapshot-read-failed") };
        var emptyLayout = PanelLayout.Chart(UiTab.Combat, Array.Empty<ChartRow>(), ChartProjection.Meta(empty, UiTab.Combat), "");
        if (!emptyLayout.Header.OfType<TextCommand>().Any(text => text.Text.StartsWith("Incomplete statistics:", StringComparison.Ordinal)))
            throw new InvalidOperationException("A rejected empty snapshot must not look like a trustworthy zero-activity combat");
        Console.WriteLine("MANAGED PANEL PROJECTION FIXTURES PASS");
    }

    private static void CheckWarning(PanelLayout complete, PanelLayout partial, string expected)
    {
        var warning = partial.Header.OfType<TextCommand>().Single(text => text.Text == expected);
        if (complete.Header.OfType<TextCommand>().Any(text => text.Text == warning.Text)
            || warning.Y >= partial.HeaderBottom || partial.HeaderBottom <= complete.HeaderBottom
            || partial.Body.OfType<RectCommand>().First().Y < partial.HeaderBottom
            || partial.Height - complete.Height != partial.HeaderBottom - complete.HeaderBottom)
            throw new InvalidOperationException("Incomplete capture must remain visible in the fixed header without overlapping or hiding chart rows");
    }
}
