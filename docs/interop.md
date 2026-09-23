# Managed and native integration

The game loads the managed mod assembly. Its runtime selector loads the matching
attribution library by absolute path; ship the assembly and native library from
the same build. [check-abi](../xtask/src/check_abi.rs) verifies the exported C
signatures against managed delegates. The ownership and failure contracts live
in the [native boundary](../profiler-core/src/abi.rs) and [crate
overview](../profiler-core/src/lib.rs).

## Godot resources and panels

Panels use the installed game's GodotSharp assembly and ordinary managed Godot
nodes. The generated project needs no script subclasses or Godot source
generator. Input stays on the managed engine path; no Godot objects or input
events cross into Rust.

The game ships no global UI theme. [PanelTheme](../shim/ui/PanelTheme.cs) loads
fonts and chrome from game resources, with per-resource engine defaults when
assets are missing. The scene tree owns each panel's child nodes, and the panel
manager disconnects its frame driver when the root exits. Test node destruction
and recreation in the actual game process: a standalone .NET fixture cannot
validate Godot object lifetimes.

## Reproducing attribution failures

Set `SPIRE_PROFILER_RECORD=1` in the game process environment before startup to
record observations alongside completed combat records. Each `.trace.json` file
belongs to the adjacent combat ordinal. Keep the matching summary, game version,
and mod build when reporting a failure.

The [observation module](../profiler-core/src/data/observation.rs) defines trace
compatibility and size limits. A truncated recording is explicitly rejected for
replay. `ProfilerNative.Replay` creates an isolated reducer, verifies recorded
operation results, returns its summary, and destroys that reducer. The
production self-test and managed session fixtures compare replayed and original
summaries without replacing the live engine.

Coverage reasons distinguish failed observation or accounting from unknown
source ownership. Unknown credit alone does not prove an observation was lost.
Attribution policy identity lives in the [summary
module](../profiler-core/src/data/summary.rs); changing the implementation
language does not establish metric comparability.

## Storage inspection

[StatisticsStore](../shim/session/StatisticsStore.cs) owns file locations and
atomic writes; [StatisticsJson](../shim/session/StatisticsJson.cs) validates
stored and native records before publishing immutable views. Use a separate
`SPIRE_PROFILER_DATA_DIR` for experiments. Headless tests set this
automatically, so their synthetic runs do not enter normal play history.

The history lookup uses the game's exact profile, seed, and start time. The
first matching finalized header supplies the outcome and roster; interrupted
combats can provide an unfinished history view without a header. Ambiguous run
identities remain unselectable. New records live under `statistics-v1`. Legacy
files and prior GUID recordings remain read-only, and missing legacy metadata
stays unknown.
