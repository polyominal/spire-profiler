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

## Auditing poison attribution

Set `SPIRE_PROFILER_AUDIT=1` in the game process environment before startup.
This also enables observation recording. Combat journals appear under
`audit-v2/run-<id>/combat-<epoch>-<attempt>.audit.jsonl` in the profiler data
directory. The attempt identifier distinguishes retries, continued runs, and
console-started encounters. Keep the journal, adjacent statistics under
`statistics-v2`, and the matching build together.

Generate a readable report from one journal or a directory:

```sh
cargo xtask audit-report /path/to/audit-v2 --output tmp/poison-audit.md
```

The journal records card origin, accepted Poison and Envenom mutations, command
completion, target HP/block/Artifact, physical damage results, and living
opponents' Accelerant stacks. Envenom callbacks identify their triggering hit.
Source frames preserve the raw cause at the moment attribution captures it,
before later hooks can change the power. Separate native checkpoints expose the
reducer's ordered grants and credited totals as claims to check.

For supported ordinary-card and Envenom causes, the report reconstructs FIFO
suppliers from raw mutations, allocates actual outgoing poison damage
independently, and compares the resulting ownership and per-tick credits with
native claims. Missing or ambiguous causal evidence makes the affected
reconstruction unverifiable; native suppliers never fill those gaps.
Generated-card ancestry and other unsupported causes remain explicit
limitations. An eligible Envenom callback without a captured canonical power
command is unresolved, since the game can refuse an application before that
command is reached.

The report retains independent poison subtotals. A row's full indirect-damage
total can include other effects, so it is not automatically equal to that
subtotal. Version 1 journals remain readable as consistency checks without
independent supplier verification. Neither mode formally verifies every game
mechanic or proves that all game activity was observed.

The current poison allocation rule is documented in the [provenance
module](../profiler-core/src/data/source/power.rs). Audit mode does not change
it. Damage requested before modifiers, actual damage, blocked damage, and
overkill are separate values; comparing requested damage directly with credited
totals can give a false discrepancy.

Each complete JSONL line is flushed immediately. A missing footer, sequence gap,
diagnostic, interrupted combat, or explicit cutoff makes the evidence
incomplete. Capture stops at 20,000 events or 32 MiB per combat, with a 1 MiB
event limit. Audit mode performs extra serialization and file I/O on the game
thread; leave it disabled for ordinary play. Journals are retained until
manually removed, and they cannot recover numerical evidence missing from
earlier runs.

## Storage inspection

[StatisticsStore](../shim/session/StatisticsStore.cs) owns file locations and
atomic writes; [StatisticsJson](../shim/session/StatisticsJson.cs) validates
stored and native records before publishing immutable views. Use a separate
`SPIRE_PROFILER_DATA_DIR` for experiments. Headless tests set this
automatically, so their synthetic runs do not enter normal play history.

The history lookup uses the game's exact profile, seed, and start time. The
first matching finalized header supplies the outcome and roster; interrupted
combats can provide an unfinished history view without a header. Ambiguous run
identities remain unselectable. New records live under `statistics-v2`. Legacy
files and numeric/GUID records in `statistics-v1` remain read-only, and missing
legacy metadata stays unknown.
