# Original UI behavior reference

The reference is commit `c928c477852e75ffcc35f8cd16c7ed03caad7c7f`.
`ui_oracle.rs` executes that commit's Rust implementation in an isolated source
export. It never imports the managed port. `ui_reference.json` records its
outputs. `UiParityFixtures.cs` feeds the same inputs to the production managed
projection and layout methods and compares their complete outputs.

The comparison includes ordered drawing commands, strings, colors, typography,
alignment, textures, row and avatar hitboxes, metadata, tooltip lines, and
geometry. Integer values and text compare exactly; floating point geometry and
colors allow an absolute difference of 0.0001. The `render_contract` and each
case's `render` describe host composition for a separate graphical comparison.
Command parity alone does not establish input or rendered-pixel parity.

## Generating the reference

Export the pinned commit into an ignored scratch directory. Copy
`ui_oracle.rs` to its `profiler-core/src/parity_oracle.rs`, then append
`#[cfg(test)] mod parity_oracle;` to its `profiler-core/src/lib.rs`. Run:

```sh
PARITY_OUTPUT=/absolute/path/to/ui_reference.json cargo test \
  --manifest-path /absolute/path/to/export/Cargo.toml \
  --package profiler_core --lib parity_oracle::export_reference --locked --offline
```

The original UI test suite also runs without Godot:

```sh
cargo test --manifest-path /absolute/path/to/export/Cargo.toml \
  --package profiler_core --lib ui:: --locked --offline
```

The managed adapter requires the production statistics DTOs, `ChartProjection`,
`PanelLayout`, `PanelGeometry`, and `TooltipLayout`. Its entry point is
`UiParityFixtures.Run(referencePath)`. It has no Godot dependency. A failed
comparison reports the fixture name and the first differing JSON path; expected
values must only change through a reviewed baseline regeneration.

## Data and presentation contracts

- Each section admits the first 128 qualifying candidates before sorting. Player
  filtering precedes admission. Ties preserve input order. Both sections share
  the 256-row output limit; a hanging self-damage row consumes a slot.
- Damage ranks by its positive contribution sum. Defense ranks by positive
  contributions minus self-damage. Positive defense and self-damage are drawn
  separately, including zero or negative net defense. Standalone self-damage
  follows positive defense. Segment normalization uses the maximum absolute net
  value before splitting; each segment clamps independently to 1000, so combined
  segments can extend past the track.
- Share percentages use all selected positive contributions, including candidates
  excluded by the admission cap. Tenths of a percent truncate. Segment widths
  truncate after per-mille normalization and again to whole pixels.
- Row identifiers retain at most 64 UTF-8 bytes. Visible names retain 16 Unicode
  characters before the ellipsis. Relic, power, and Osty prefixes use separate
  colored text runs. Hanging self-damage says `+ self damage` and hides plays;
  standalone self-damage retains its identifier and plays. Zero plays are hidden.
- Detail resolution selects the first card with the same player and identifier,
  ignoring source kind. Truncated identifiers may not resolve back to the full
  source. These surprising baseline behaviors are intentional parity fixtures.
- Live Combat and Run headline totals remain team-wide when a player is selected.
  History totals use the selected saved player rollup. An unavailable history
  player rollup falls back to the aggregate rollup. DPS truncates to one decimal;
  zero turns produce the original dash. Footer text is compared byte for byte.
- The main panel is 780 pixels wide. Body text is 24 pixels and its title is 32.
  Textured tabs are 256 by 90 pixels in a 102-pixel strip above the plate.
  Portraits are 64 pixels with a 70-pixel pitch. The header remains pinned while
  body rows scroll. Section bands, rules, alternating shading, and hover shading
  are separate ordered commands.
- Textured content insets are asymmetric: left 22, top 16, right 37, bottom 28.
  Flat fallback uses 12-pixel insets. The modal fits its content up to viewport
  height minus 96, with a 96-pixel minimum cap. Position clamps at zero when the
  viewport is smaller than the panel; panel width does not shrink.
- Failed portrait loads remove their occupied position but remain represented in
  the portrait-path array. Character texture indices address this array, not
  player slots. Missing textures and flat chrome have separate fixtures.
- Tooltips are 360 pixels wide. Titles, labels, values, and overflow text wrap at
  their distinct original budgets. Hard breaks use UTF-8 byte boundaries, while
  the fit test counts Unicode characters. The fixtures cover both forms of
  truncation and wrapping. Zero-valued lines remain when their group is visible;
  hanging self-damage details contain only the self-damage cost.
- The nine-entry legend stays beside the plate. Side panels flip when the plate's
  right edge exceeds 75 percent of the viewport, then clamp horizontally. A
  tooltip fits below the legend with a five-pixel gap when possible; otherwise
  it follows the hovered row within the plate's vertical limits.
- The original font selection applies to the entire panel: one uncovered glyph
  in its header, rows, or active detail switches both title and body to the
  fallback font. The renderer must not choose fallback independently per string.

## Input and lifecycle contracts requiring host verification

- F8 reacts on a press edge. Outside a run, the live toggle is ignored without
  destroying stored visibility. History screen state routes the same hotkey to
  the history panel. Closing and reopening a history screen preserves the
  original visibility rules.
- Tabs and avatars react to a fresh mouse press, never during a scrollbar drag.
  Clicking the selected avatar returns to All. The live filter is shared across
  tabs; history has its own filter and resets a stale slot when the roster
  changes. A tab or avatar change retains the scroll position, then clamps it
  to the new content height.
- Outside dismissal also uses a fresh press. Moving a held scrollbar drag
  outside does not dismiss. The legend and tooltip are mouse-transparent and
  count as outside the main plate and tab strip for dismissal.
- Wheel press deltas are exactly 60 pixels; releases contribute zero. Pan events
  use their raw vertical delta. Non-finite queued input is ignored; finite queue
  overflow clamps. Hidden panels drain pending input. The scrollbar has a
  20-pixel track and a fixed 30-pixel grabber; track clicks map proportionally,
  and dragging remains captured until release.
- All avatars start at scale 1. Selecting a player animates it to 1.1 and others
  to 0.95 over 0.05 seconds; unselected modulation is 0.55. New or reordered slots
  initialize at their new target, rather than inheriting another slot's motion.
- The original backdrop is black with alpha 0.8. Tooltip and legend geometry
  expands the drawing control without changing the main plate's input bounds.
- The run-history Contribution button follows the live Share button rectangle:
  same size, a 16-pixel horizontal gap, and 22-pixel Kreon Bold text. Disabled
  appearance retains the plate with dim modulation. Its anchor must follow
  actual layout changes rather than a hard-coded screen corner.

History fixture selection explicitly feeds the selected original rollup into the
pure layout. The production store/session-to-panel routing, input dispatch,
texture loading, text metrics, theme replay, and persistence lifecycle require
their own integration checks; the adapter does not claim to test them.
