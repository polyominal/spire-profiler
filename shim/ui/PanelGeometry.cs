using System;
using System.Collections.Generic;
using System.Linq;

namespace SpireProfiler;

internal readonly record struct UiPoint(float X, float Y);
internal readonly record struct UiRect(float X, float Y, float W, float H)
{
    internal bool Contains(float x, float y) => x >= X && x < X + W && y >= Y && y < Y + H;
}
internal sealed record ScrollbarGeometry(UiRect Track, UiRect Body, UiRect CapTop, UiRect CapBottom, UiRect Grabber);

internal static class PanelGeometry
{
    internal static float EventScrollDelta(long button, bool pressed, float panY) => button switch
    {
        4 when pressed => -60, 5 when pressed => 60, _ => panY
    };
    internal static float ApplyScroll(float offset, float delta, float boxHeight, float contentHeight)
        => Math.Clamp(offset + delta, 0, Math.Max(0, contentHeight - boxHeight));
    internal static (float Top, float Bottom) BodyBand(float boxHeight, bool plate, float headerBottom)
        => (Math.Max(0, headerBottom), Math.Max(Math.Max(0, headerBottom), boxHeight - (plate ? 28 : 12)));
    internal static float HeightCap(float? viewportHeight) => viewportHeight.HasValue ? Math.Max(96, viewportHeight.Value - 96) : 600;
    internal static UiPoint Center(UiPoint viewport, UiPoint box) => new(Math.Max(0, (viewport.X - box.X) / 2), Math.Max(0, (viewport.Y - box.Y) / 2));
    internal static int? Hover(IReadOnlyList<RowHit> hits, UiRect rect, UiPoint mouse, float scroll, (float Top, float Bottom) band)
    {
        float localY = mouse.Y - rect.Y;
        if (!rect.Contains(mouse.X, mouse.Y) || localY < band.Top || localY >= band.Bottom) return null;
        foreach (var hit in hits) if (localY + scroll >= hit.Y0 && localY + scroll < hit.Y1) return hit.FlatIndex;
        return null;
    }
    internal static ScrollbarGeometry Scrollbar(UiPoint box, bool plate, (float Top, float Bottom) band, float contentHeight, float scroll)
    {
        float maximum = contentHeight - box.Y, height = band.Bottom - band.Top;
        if (maximum <= 0 || height <= 0) return null;
        var track = new UiRect(box.X - (plate ? 13 : 6) - 20, band.Top, 20, height);
        return new(track, new(track.X, track.Y + 12, 20, Math.Max(0, height - 24)),
            new(track.X, track.Y, 20, 12), new(track.X, track.Y + height - 12, 20, 12),
            new(track.X - 5, band.Top + scroll / maximum * Math.Max(0, height - 30), 30, 30));
    }
    internal static float TrackScroll(UiRect track, float mouseY, float maximum)
        => Math.Clamp((mouseY - track.Y) / track.H, 0, 1) * maximum;
    internal static bool DragState(bool active, bool pressed, bool wasPressed, bool onTrack)
        => pressed && (active || !wasPressed && onTrack);
    internal static (UiPoint Size, UiPoint Origin) LegendPlate(bool plate)
        => plate ? (new(183, 260), new(22, 16)) : (new(140, 240), new(12, 12));
    private static float SideX(UiPoint viewport, UiRect plate, float width)
        => Math.Clamp(plate.X + plate.W > viewport.X * .75f ? plate.X - width : plate.X + plate.W, 0, Math.Max(0, viewport.X - width));
    internal static UiRect PlaceLegend(UiPoint viewport, UiRect plate, UiPoint size)
        => new(SideX(viewport, plate, size.X), plate.Y, size.X, size.Y);
    internal static UiRect PlaceTip(UiPoint viewport, UiRect plate, float rowY, UiPoint size, UiRect? legend)
    {
        float low = plate.Y + 8, high = Math.Max(low, plate.Y + plate.H - size.Y - 8);
        float y = legend is { } key && key.Y + key.H + 5 <= high ? Math.Max(rowY, key.Y + key.H + 5) : rowY;
        return new(SideX(viewport, plate, size.X), Math.Clamp(y, low, high), size.X, size.Y);
    }
    internal static (UiRect Rect, float OriginX) Frame(UiRect plate, UiRect? legend, UiRect? tip)
    {
        float left = plate.X, right = plate.X + plate.W, bottom = plate.Y + plate.H;
        foreach (var rectangle in new[] { legend, tip })
            if (rectangle is { } side)
            {
                left = Math.Min(left, side.X);
                right = Math.Max(right, side.X + side.W);
                bottom = Math.Max(bottom, side.Y + side.H);
            }
        return (new(left, plate.Y, right - left, bottom - plate.Y), plate.X - left);
    }
}

internal sealed class AvatarAnimation
{
    private sealed class Entry
    {
        internal int Slot;
        internal float Value, From, Target, Elapsed;
    }
    private readonly List<Entry> _entries = new();
    private double? _lastTick;
    internal IReadOnlyList<float> Values => _entries.Select(entry => entry.Value).ToArray();
    internal void Clear() { _entries.Clear(); _lastTick = null; }
    internal void SetTargets(int? selected, IReadOnlyList<int> slots)
    {
        int shared = Math.Min(slots.Count, _entries.Count);
        for (int index = 0; index < slots.Count; index++)
        {
            float target = selected == null ? 1 : selected == slots[index] ? 1.1f : .95f;
            if (index >= shared) _entries.Add(new Entry { Slot = slots[index], Value = target, From = target, Target = target });
            else
            {
                var entry = _entries[index];
                if (entry.Slot != slots[index]) { entry.Slot = slots[index]; entry.Value = entry.From = entry.Target = target; entry.Elapsed = 0; }
                else if (entry.Target != target) { entry.From = entry.Value; entry.Target = target; entry.Elapsed = 0; }
            }
        }
        if (_entries.Count > slots.Count) _entries.RemoveRange(slots.Count, _entries.Count - slots.Count);
    }
    internal bool AdvanceFrame(double now)
    {
        if (!_entries.Any(entry => entry.Value != entry.Target)) { _lastTick = null; return false; }
        float delta = _lastTick is { } last ? (float)(now - last) : 0;
        _lastTick = now;
        return Advance(delta);
    }
    internal bool Advance(float delta)
    {
        bool redraw = false;
        foreach (var entry in _entries)
        {
            if (entry.Value == entry.Target) continue;
            entry.Elapsed += delta;
            entry.Value = entry.Elapsed >= .05f ? entry.Target : entry.From + (entry.Target - entry.From) * Math.Clamp(entry.Elapsed / .05f, 0, 1);
            redraw = true;
        }
        return redraw;
    }
}
