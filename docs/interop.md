# Managed and native integration

The game loads the managed mod assembly. Its runtime selector loads the matching
native library by absolute path; ship the assembly and native library from the
same build. [check-abi](../xtask/src/check_abi.rs) verifies the exported C
signatures against managed delegates. The ownership and failure contracts live
in the [native boundary](../profiler-core/src/abi.rs) and [crate
overview](../profiler-core/src/lib.rs).

## Managed entry points

For managed-side changes, start with the relevant owner:

- [CaptureRuntime](../shim/attribution/CaptureRuntime.cs): capture-thread and
  combat admission; [SourceSnapshot](../shim/attribution/SourceSnapshot.cs):
  managed ownership of native source handles.
- [FlowCapture](../shim/attribution/FlowCapture.cs) and
  [DamageCapture](../shim/attribution/DamageCapture.cs): producer context and
  completed damage observations.
- [ProfilerSession](../shim/session/ProfilerSession.cs): run and combat
  transitions, record submission, and published views.
- [StatisticsStore](../shim/session/StatisticsStore.cs): managed record
  transport and optional trace files; [storage inspection](#storage-inspection)
  covers persistence and diagnosis.
- [ProfilerPanels](../shim/ui/ProfilerPanels.cs) and
  [ChartProjection](../shim/ui/ChartProjection.cs): panel lifecycle and summary
  presentation.

Lifecycle changes span two owners: `CaptureRuntime.InvalidateEpoch` rejects
further observations from a combat, while `ProfilerSession` owns discarding its
native state. Invalidating capture alone does not discard the native ledger.

## Following a direct damage result

Consider an ordinary Strike with no modifier credit: the game reports one
outgoing result with 4 unblocked damage and 2 blocked damage. The profiler
records 6 total damage. This is the damage observation in
`ProfilerSession.SelfTest`, which also exercises block and forge accounting.

1. **Capture the source and result.** In
   [DamageCapture](../shim/attribution/DamageCapture.cs), `Prefix` gives an
   explicit card source precedence over an ambient producer. `ModifyDamage`
   opens a target-local calculation. `ReportResultGroup` reads finalized game
   results, appends their packets, and commits the complete group. The total
   comes from unblocked plus blocked damage, not the requested amount.
2. **Assign credit.**
   [NativeAttributionBackend](../shim/native/NativeAttributionBackend.cs)
   forwards the calculation through
   [ProfilerNative](../shim/native/ProfilerNative.cs) and the Rust ABI. [Damage
   allocation](../profiler-core/src/data/source/allocation.rs) assigns all 6
   points to the card when no modifiers contribute. [The
   ledger](../profiler-core/src/data/source.rs) records `damage_dealt = 6`,
   `dmg_direct = 6`, and `damage_blocked = 2`; the attributed and modifier
   segments remain zero. Blocked damage is included in the total, so unblocked
   damage is `damage_dealt - damage_blocked`.
3. **Publish a live view.**
   [State::snapshot](../profiler-core/src/data/summary.rs) serializes the ledger
   and coverage. `ProfilerSession.Refresh` reads changed native revisions,
   parses the snapshot through
   [StatisticsJson](../shim/session/StatisticsJson.cs), checks combat identity,
   and publishes `CurrentCombat`.
4. **Persist a completed combat.** `ProfilerSession.Finish` wraps the snapshot
   with run, game, and mod identity and submits it through `StatisticsStore` to
   Rust. Completed records feed the run aggregate; selecting history reads
   persisted records into `SelectedHistory`.
5. **Display the result.** `ProfilerPanels` supplies those views to the panels.
   `ChartProjection.Rows` makes a damage row with value 6 and a direct segment;
   `ChartProjection.Detail` shows 6 damage with 4 unblocked. Filtering and
   rendering operate on these views without modifying attribution.

### What verifies this example

The existing checks cover different portions of that path:

- The Rust scenario
  `ordinary_hit_on_poisoned_target_does_not_consume_poison_suppliers` in
  [scenarios.rs](../profiler-core/src/data/source/scenarios.rs) asserts the
  4-unblocked, 2-blocked hit's exact direct credit and checks that it does not
  consume poison suppliers.
- `DamageGroups` in [managed capture fixtures](../shim/tests/Fixtures.cs) checks
  the patched damage bridge, card-source selection, and complete-group timing
  against a fake backend. Its redirected result group is a different fixture
  from the single hit above.
- [SessionFixtures](../shim/tests/SessionFixtures.cs) invokes
  `ProfilerSession.SelfTest` against the real native library. It injects native
  observations and checks damage totals, replay equality, and stored history.
  This does not exercise capture from actual game damage.
- [Panel fixtures](../shim/tests/PanelFixtures.cs) check presentation rules on
  constructed summaries. They do not establish rendered appearance.

Run the native scenario through `cargo nextest` and the managed fixtures through
`cargo xtask managed-test`. No single existing check follows this exact hit from
a real game hook through SQLite to rendered pixels. See the [verification
guide](verify.md) for gate selection, game prerequisites, and manual validation.

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
record observations under
`statistics-v3/traces-v2/<run-id>/<combat-id>.trace.json`. Keep the matching
statistics database, game version, and mod build when reporting a failure.
Observation recordings and audit journals remain optional file artifacts,
separate from the statistics database.

The [observation module](../profiler-core/src/data/observation.rs) defines trace
compatibility and size limits. A truncated recording is explicitly rejected for
replay. `ProfilerNative.Replay` creates an isolated reducer, verifies recorded
operation results, returns its summary, and destroys that reducer. Modifier
credits enter the trace as integers: replay checks their accounting, while
managed fixtures check their decimal derivation and source selection. The
production self-test and managed session fixtures compare replayed and original
summaries without replacing the live engine. Version-1 traces under `traces/`
require their matching older build; replay rejects them without changing state.

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
console-started encounters. Keep the journal, statistics under `statistics-v3`,
and the matching build together.

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

[The Rust store](../profiler-store/src/lib.rs) owns SQLite and its persistence
contract. The managed host supplies finalized records and aggregates returned
records into views. Storage has a separate native handle from attribution;
database failure leaves live capture available with partial coverage.

Use a separate `SPIRE_PROFILER_DATA_DIR` for experiments. New databases live at
`statistics-v3/statistics.sqlite3`. On first use, legacy JSON records are read
once and imported atomically; their original files are preserved. Unknown
database versions are rejected without replacement. Close the game before
copying the database and any adjacent journal for diagnosis.

Import and history transfer records incrementally. Individual record and
transport limits are documented in the Rust store's persistence contract.

The history lookup uses the game's exact profile, seed, and start time. The
first matching finalized header supplies the outcome and roster; interrupted
combats can provide an unfinished history view without a header. Ambiguous run
identities remain unselectable. The importer accepts unversioned records,
numeric/GUID records in `statistics-v1`, and schema-2 records in
`statistics-v2`; missing legacy metadata stays unknown. An import cannot recover
records already absent from the source archive.
