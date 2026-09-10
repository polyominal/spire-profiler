# Static allocation branch handoff

## Checkpoint and next action

Development is paused after implementation item 3. Items 1 through 3 are human
committed; items 4 through 8 have not started. This handoff is a separate
documentation change and does not advance the implementation plan. Resume item 4
only when the user explicitly resumes development. Confirmation of the handoff
commit alone is not an instruction to continue.

This temporary, tracked scratchpad contains the continuation plan and handoff
evidence. A fresh checkout needs no earlier conversation, local research
checkout, or ignored `tmp/` artifact. Read [AGENTS.md](AGENTS.md) first. Durable
contracts remain in Rust module docs; source prevails if a checkpoint fact has
drifted. The temporary pointer in AGENTS.md, this file, and its formatter entry
must be removed together before the final merge, as described below.

- Plan base: `3c2406a6b66c4cf4bb8c10fcbb4a06728664b82b`, the attribution
  implementation before this plan.
- Item 1: `a1cfca4ad5fe8f4b0e4ccba81354150a099fb203`. Recorded validation: 328
  core tests, eight release allocation tests, 404 smoke tests, and 39 baseline
  rows agreeing in debug/release.
- Item 2: `aeb74ad9796411c4790a209b61dd4dd74c27c4aa`. Recorded validation: 341
  core tests, 19 release allocation/lifecycle/simulation tests, and 417 smoke
  tests.
- Item 3: `f3c00805772d2da9d75616f6adcbcb24da13e020`. Recorded validation: 361
  core tests in both debug and release, and 437 smoke tests with zero skipped.

Each item completed implementation, independent review, corrections, and its
required checks before the human commit. Final Astra reviews found no actionable
issues. Item 3 had two independent reviewers, covering storage and behavior.
These are historical results from macOS arm64 on `nightly-2026-08-12`, rustc
commit `3d6c19bb9ab4798ecfb2ee943df01a811720fc27`. They are not results from the
next computer or a qualification of every execution path. Git contains the
changes and reproducible tests; local raw logs are optional corroboration.

Handoff validation on the same host passes `cargo xtask smoke` (437 tests, zero
skipped), including formatting, citations, em-dash counts, ABI, Clippy, and
documentation gates. The primary checked relative links, all eight plan items,
source/commit references, and cleanup discoverability. This documentation change
has no separate subagent review and changes no gameplay code.

## Scope and non-negotiable boundaries

The accepted target is allocation-free native computation with bounded retained
storage after explicit initialization. It covers attribution, gameplay state
mutation and logical reset, snapshots, pure history matching and rollups,
layouts, and tooltip computation. Read the allocation contract in
[lib.rs](profiler-core/src/lib.rs) and the adapter boundaries in
[persistence.rs](profiler-core/src/data/persistence.rs) and
[engine.rs](profiler-core/src/engine.rs).

- Initialization may reserve heap storage fallibly. New combats/runs and
  repeated `init` do not reopen gameplay initialization. Failed initialization
  leaves the owner disabled without retrying. Final owner teardown may free
  storage. Panel construction has its own explicit lifetime-owner setup.
- Zero means no `alloc`, `alloc_zeroed`, `realloc`, or `dealloc` in the measured
  computation. Admission, expected failures, release, and populated resets are
  part of that computation. Keep large buffers on the initialization heap;
  static allocation does not mean giant stack arrays.
- Filesystem/codec work, host engine calls, panel creation/resource loading,
  foreign allocators, and unexpected panic recovery are declared exceptions. Run
  merging inside a writer and other pure computation inside an adapter wrapper
  stay in scope. Moving allocations into the shim or disabling the counter
  around a mixed event does not satisfy the target.
- Preserve panic containment at the ABI. Expected exhaustion and malformed input
  use bounded failure values and existing diagnostics. Never enforce the target
  with a production allocator that panics, aborts, or returns null to punish a
  forbidden allocation.
- Preserve transactions, source ownership, deterministic order, Unknown
  attribution, and epoch/serial rejection. Keep mutable gameplay data under
  State's borrowing contract. No new gameplay locks, atomics, global pools, or
  unsafe code outside the existing quarantine.

The registered test allocator observes the calling thread's forwarded Rust
allocator operations. It does not observe spawned threads, foreign allocators,
or runtime paths that call `System` directly, including some TLS setup. It
passes operations through and reports outside allocator callbacks. Counts are
evidence for exercised scopes, not whole-process memory or latency bounds.
Unexpected panic recovery remains outside the guarantee, even though containment
must work. A whole-core or host-inclusive lifetime allocation ban would be a
separate scope expansion requiring an explicit decision.

## Starting on a fresh computer

1. Check out the branch containing the item 3 commit and this handoff. Inspect
   `git status --short` and `git log --max-count=6 --oneline`; do not overwrite
   unrelated work. If history was rebased, identify the items by their titles
   and diffs and update the checkpoint commit mapping. Do not replay completed
   items just because an old hash is absent.
2. Follow [docs/build.md](docs/build.md), [docs/verify.md](docs/verify.md), and
   `cargo xtask --help`. The checked-in Rust toolchain, lockfile, xtask pins,
   and game-version pin are authoritative. Supported build hosts are macOS,
   Linux, and WSL2. Smoke needs the bootstrapped .NET SDK but no installed game.
   Missing local dependencies or network access are environment prerequisites,
   not reasons to bypass a gate.
3. Recreate project-local `tmp/` as needed. Baseline commands below need no
   machine-local assessment document. Save new logs under a new item directory.
   Do not assume old decompilation, test data, logs, or tools survived.
4. Before editing item 4, verify the baseline and inspect the in-scope
   allocation paths below. Record actual commands, target, outcomes, and any
   blocker. The game pin at this checkpoint is `v0.111.0`; game-touching work
   must obey the update/re-verification procedure in AGENTS.md.

<!-- end list -->

```sh
cargo test --package profiler_core --locked
cargo test --package profiler_core --locked --release
cargo test --package profiler_core --locked --test allocation_baseline -- --nocapture
cargo xtask smoke
```

The allocation baseline is an observation matrix, not a test requiring every row
to become zero immediately. Fixture and filesystem preparation stay outside
computation measurements. Use the existing test-support entry points to measure
real production computation; do not replace it with a test-only implementation.

## Per-item implementation and review protocol

Every numbered item is a separate implementation and review cycle. The user
requires fresh subagents using explicit `gpt-6-astra` with `medium` reasoning.
Use standalone task briefs without inherited implementation conversations
(`fork_turns: "none"` where supported). If that model or delegation capability
is unavailable, report the specific missing prerequisite before substituting
another process.

1. Capture the starting commit, status, and complete item diff under
   `tmp/static-allocation-item-N/`. Start one or more new implementers for the
   item; do not reuse prior-item implementers or have the primary replace this
   phase. Give each a bounded task, files, constraints, invariants, expected
   deliverable, and required checks. Assign clear file ownership. The primary
   owns integration and decisions spanning subsystems.
2. Parallelize independent work while the primary makes useful progress. Avoid
   overlapping writes, and never run concurrent headless or decompile commands.
   Follow-ups may finish an implementer's work within the same item. Treat
   reports as proposals: verify source evidence, arithmetic, diffs, and tests.
3. Integrate and run focused behavior/allocation checks and required repository
   gates. Freeze a complete diff, including new files, and a file-hash manifest.
   Spawn new independent reviewers who did not implement the item. Supply the
   requirements, complete diff, relevant source, and raw validation evidence;
   omit implementation conversation history. Reviewers report actionable
   findings and coverage gaps without editing the reviewed change.
4. Evaluate findings and send necessary corrections to the item's implementers.
   Re-run affected checks and use new reviewers for subsequent review passes.
   Finish only when actionable findings are resolved, the final diff is
   reviewed, and required checks pass. Do not weaken behavioral expectations or
   snapshots to make a storage refactor pass.
5. Update this handoff with the final disposition and limits, then stop for the
   human commit. Agents create no commits or PRs. If proposing a message, use
   `<scope>: <subject>` and end its body with `AI-Assisted: gpt-6-astra`. On
   human commit confirmation, verify the actual commit and working state before
   advancing, subject to any explicit pause from the user.

Keep raw execution records and comparison artifacts in ignored `tmp/`, and
unrelated non-blocking problems in session-scoped `tmp/ISSUES.md`. Essential
continuation facts belong here before stopping; an ignored report must never be
the only explanation of an unresolved decision.

## Numbered implementation plan

### 1\. `alloc: define computation boundaries and add allocation accounting`

Completed. Define the exact contract in module docs and enumerate allocating
persistence/engine adapters. Add isolated pass-through accounting for all four
allocator operations, calibrated for nesting, return-value destruction, and
unwind. Measure existing behavior before enabling zero checks as paths convert.
Keep computation visible inside mixed lifecycle events.

The harness is
[tests/support/allocation.rs](profiler-core/tests/support/allocation.rs); the
observation matrix is
[allocation\_baseline.rs](profiler-core/tests/allocation_baseline.rs). Dedicated
sink tests cover warmed diagnostic/event-log paths. First-use diagnostics and
unusual host paths still need treatment when extending that coverage.

### 2\. `state: retain bounded identity and lifecycle storage`

Completed. Introduce shared bounded text and retain combat, run, roster,
finish-staging, and production self-test storage. Reserve fallibly during
initialization, separate occupancy from storage, and reuse owners on
replacement, discard, and finish handoff. Metadata admission precedes lifecycle
mutation.

The implementation uses [Text](profiler-core/src/data/text.rs), aliases and caps
in [state.rs](profiler-core/src/data/state.rs), and occupancy-bearing
`Retained<T>` owners. Byte caps are profiler-supported input policy, not hard
bounds on arbitrary game mods or custom seeds: model IDs 128, seeds 256, labels
32, decimal network IDs 20, and run character text `4 * 128 + 3 = 515`. Reject
oversized/NUL identities; never truncate them into collisions. Serde checks
decoded UTF-8 bytes. Invalid source identity preserves the credited slot's
Unknown behavior, and unused wire text cannot replace a trusted capture.

Preserve the named bounds of 512 combat rows, 1,024 run rows, 128 destinations
per source, and five creditor slots, including Unknown capacity.

Nine lifecycle buffers retain their backing storage. `FinishedCombat` and
`FinishedRun` leases return staging after State guards are released.
Initialization publishes Disabled before reservation and Ready only after
success; ordinary repeat calls do not retry. The exported test reset is an
explicit test teardown, not a production allocation privilege. Repeated
production self-tests pin all nine owners. Valid persisted JSON snapshots are
unchanged.

Serialization envelopes in
[persistence.rs](profiler-core/src/data/persistence.rs) are decomposed const
arithmetic for fields, syntax, numeric widths, and worst case escaping: 665,354
combat bytes and 8,209 run bytes before the JSONL newline. Tests compare these
against actual serialization. Preserve the derivation; do not replace it with
unexplained totals when the schema changes.

### 3\. `source: reuse transfer and provenance storage`

Completed. Convert normalization, Unknown fills, snapshots/prefixes, grants,
play/generated/orb/reduction tracking, calculation captures, and defensive pools
to retained bounded owners. Preserve logical reset, ordering, copied facts, and
stale-token rejection.

- [Slots](profiler-core/src/data/source/storage.rs) owns a preconstructed Vec
  and an occupied prefix. Stable removal rotates an owner into inactive storage.
  Logical clear preserves nested buffers. Active iteration avoids scanning every
  reserved slot. Snapshot and prefix `clone_from` retain storage; ordinary owned
  clones used by staging still allocate.
- Transfers retain upload and sealed buffers independently of their
  Upload/Sealed/Invalid status. Checked u128 normalization preserves first-seen
  destinations and validates before publication. `snapshot_into` fills an
  initialized owner. Prefix candidate cursors commit only after validation;
  `credit_iter` borrows deltas, while the owned `credit` adapter still
  allocates.
- The flat grant table has 512 owners, with at most 64 per power and at most
  `512 - 256` extra grants beyond each occupied power's first grant. This keeps
  space for one Unknown grant per power. Power compaction adjusts owner indices.
  Suspended captures own facts rather than indices into mutable grants. Preserve
  FIFO grants, LIFO reductions, cached trust, Weak-head lookup, and the Poison
  empty-source exception.
- The play cap is `32 * 5 = 160`. Identified play/generated admission rejects
  full tables before ledger publication. Missing wire identities still count
  observed events while creating no retained tracking identity, including at
  full occupancy. Invalid regeneration/rechannel invalidates old ancestry.
- Each of 64 calculations retains its producer and Weak snapshots, 64 modifier
  captures, 64 copied Strength facts, and two results. Each of five pools has 64
  blocks, four modifiers per block, 16 pending modifiers, and 32 Osty owners.
  Commit/abort and pool reset retain storage. Live pool publication uses custom
  fieldwise `clone_from`, including pending tuple snapshots; generic tuple
  replacement previously failed the pointer-retention check.

Six new allocation guards in the calculation, snapshot, tracking, and transfer
integration suites check first/repeated use and bounded failure/reuse/reset.
Their fixture setup is outside measurement and failure diagnostics stay inside.
The source limits test pins 10,160 calculation/pool source pointers through
capture, publication, and reset. Independent simulation models remain intact.

Item 3 does not make successful play/generation transactions, owned attribution
outputs, deep staging, or Doom capture allocation-free. A populated Doom reset
still frees nested vectors. These are item 4 work, not acceptable exceptions in
the final gameplay computation contract.

### 4\. `source: replace allocating attribution and transaction staging`

Not started; next implementation item after an explicit resume.

Make proportional allocation and grouped results fill caller-owned buffers.
Coalesce Doom kickoff credits by destination, preserving captured amounts and
first-seen order. The conservative destination universe is 512 rows plus five
Unknown slots. Bound simultaneous batches and targets using the named caps;
completion must not reread mutable grants.

Replace `LedgerStage` and `CardStat::merge_rows` deep clones with reusable
staging. Keep immutable identity/source payloads out of copied numerical state
where practical. Begin with bounded staging and publish-after-validation;
introduce an undo log only with a demonstrated need. Preserve all-or-nothing
rows, counters, pool cursors, multiple targets, grants, and Unknown fallback on
late arithmetic or capacity failure. Reuse staging after both success and
failure; prevent scratch aliasing under State's borrowing contract.

Start inspection at these actual remaining paths:

- [source.rs](profiler-core/src/data/source.rs): `LedgerStage::new` clones the
  combat and pools; stage publication retains live owners but frees the
  temporary stage. `DoomBatch` and `DoomCapture` own nested vectors.
- [snapshot.rs](profiler-core/src/data/source/snapshot.rs) and
  [allocation.rs](profiler-core/src/data/source/allocation.rs): `RootBudgets`,
  proportional/take results, grouped damage outputs, and owned prefix credits.
- [pools.rs](profiler-core/src/data/source/pools.rs),
  [damage.rs](profiler-core/src/data/source/damage.rs), and
  [play.rs](profiler-core/src/data/source/play.rs): consuming block/modifier
  outputs, successful ledger transactions, and calculation commit.
- [power.rs](profiler-core/src/data/source/power.rs): Doom capture/completion
  and its outputs.
  [persistence/runs.rs](profiler-core/src/data/persistence/runs.rs):
  transactional run merges clone the row vector.

Enable zero checks across every gameplay computation and repeated populated
logical resets, including Doom. Preserve existing late-failure scenarios and the
seeded per-unit/per-seat reference models. Extend first-use, maximum occupancy,
cap-plus-one, release/reuse, and alternating small/large cases; do not declare a
whole mixed I/O event allocation-free.

### 5\. `ui: reuse snapshots layouts and tooltip text`

Not started. Filter borrowed card slices, use fixed ranking scratch, and keep
stable ties through an explicit original-position key or bounded stable sort.
Refill panel-owned row, command, hit, detail, tooltip, avatar, and text buffers.
Use one command vector with header/body ranges and bounded text addressed by
offsets. Build the run chart directly into its destination, removing header
collection and duplicate command storage. Bound bytes as well as counts and
check room before formatting. Display truncation must respect UTF-8 boundaries.
Hiding/clearing retains buffers with their panel lifetime owner.

Inspect [ui/snapshot.rs](profiler-core/src/ui/snapshot.rs),
[chart\_layout.rs](profiler-core/src/ui/chart_layout.rs),
[run\_layout.rs](profiler-core/src/ui/run_layout.rs),
[tooltip.rs](profiler-core/src/ui/tooltip.rs), and the panel modules. Enable
zero checks for dirty rebuilds, hover, tabs, filters, scroll, and resize,
including first use after panel setup. Preserve command snapshots and explicit
tie order.

### 6\. `history: retain selected aggregates and stream record processing`

Not started. Replace `RunSummaryView.combats`/`CombatView` storage with checked
combat count, turns, and damage-taken aggregates. Remove the all-record cache
and deep `selected_view` cloning. Adjust fingerprints and their tests while
preserving `ended_at` fallback. Parse one bounded record at a time and refill
one selected view; codec and disk allocations remain at the declared adapter.

Preserve exact profile/seed/StartTime matching, ambiguous-identity rejection,
sorted first-seen order, continuation, and combats-only fallback. The 64 MiB
file limit covers individual combat files and `runs.jsonl`, not the whole
combat-file store. Retain it unless deliberately changing its documented
contract. Directory enumeration is not deterministic sorted order. Compact ID
indexes at the allocating I/O boundary are allowed; do not scan every possible
u32 ID or silently return partial history after an incomplete read.

Stream existing JSONL into a sibling temp file, append the new record there, and
atomically rename. Preserve malformed-line and failed-read behavior and failed
temporary-write/rename recovery. An unchecked append is not equivalent. Inspect
[run\_history.rs](profiler-core/src/data/run_history.rs),
[records.rs](profiler-core/src/data/records.rs), and the persistence modules.
Deletion of unused retained history data should precede a new abstraction.

### 7\. `engine: remove transient Rust allocations from dispatch`

Not started. Remove formatted callback labels and per-call CString construction
in [gdext.rs](profiler-core/src/engine/gdext.rs) and
[registration.rs](profiler-core/src/registration.rs). Consider bounded
caller-owned Variant storage for synchronous calls only after a focused unsafe
proof/review closes. Preserve stable initialized addresses, return slots,
failure cleanup, destructors, and retained-resource ownership. Apply the unsafe
Rust review skill when available; document and verify those obligations rather
than mechanically removing Box.

Host allocation, panel creation, and resource loading remain explicit boundary
exceptions. Keep panel buffers with their actual lifetime owner. A zero native
counter or warmed glyph cache proves nothing about foreign engine allocation.

### 8\. `alloc: enforce computation budgets across lifecycle scenarios`

Not started. Complete the matrix and remove transitional allocating computation
paths. Update canonical module docs to say exactly what is established. Exercise
many combats/runs in one initialized process, maximum occupancy, alternating
small/large inputs, release/reuse, expected failure, history changes, and panel
recreation. Keep counters active on overflow and expected failure. Report
unsupported runtime paths and incomplete gates explicitly.

Final acceptance requires:

- Preserve seeded independent models, per-event ledger invariants, delayed and
  nested operations, stale tokens, and transaction rollback. Separate setup,
  expected-value generation, and filesystem preparation from measurements. First
  use after initialization must pass without arbitrary warmup.
- Check exact-cap and cap-plus-one bytes, UTF-8/NUL, maximum rows and source
  fanout, full nested calculations, and recovery. Verify all four allocator
  deltas and retained capacity/pointers; either alone misses failure modes.
- Preserve JSON byte snapshots and resume/identity semantics for missing,
  malformed, oversized, unreadable, conflicting, and failed-write records.
- Quantify maximum-load CPU, stack use, and retained bytes, including staging
  and simultaneous ownership. No hard memory or frame-latency budget has been
  selected. Avoid trading allocator traffic for unexamined full-capacity copies.
- Run the gate set in [docs/verify.md](docs/verify.md), including smoke, managed
  tests, and headless tests when integration changes land. `cargo xtask build`
  builds the four native artifacts: macOS arm64/x86\_64, Linux x86\_64, and
  Windows x86\_64. Cross-compilation does not establish runtime allocation
  claims on Windows/Linux; run the relevant checks there.
- Manually validate both panels, fonts, localization, hover/scroll, history
  reopen/recreation, and combat/run suspend/resume. Headless draw dispatch does
  not establish visual correctness. Respect the game pin and fixed scratch
  paths.
- Complete the temporary handoff cleanup below. Any proposed human-authored PR
  title/description must describe allocation-free computation with bounded
  retained storage and its actual limits. Agents do not publish it.

## Memory evidence and unresolved choices

Item 3's retained source payload arithmetic on the inspected macOS arm64 layout
uses 24-byte weighted destinations, 32-byte upload/mixture pairs, 128
destinations per source, and two u64 cursor arrays per prefix:

```text
Transfers and normalization:
  16 * 128 * (24 + 32) + 128 * 32 = 118,784 bytes
Tracking and scratch:
  (256 + 512 + 64 + 32 + 160 + 64 + 1) * 128 * 24 + 128 * 32
  = 3,349,504 bytes
Calculation captures:
  64 * (2 + 64 + 64) * 128 * 24 = 25,559,040 bytes
Defensive pools:
  5 * (64 * (1 + 4) + 32) * 128 * (24 + 8 + 8) + 5 * 16 * 128 * 24
  = 9,256,960 bytes
Listed total:
  118,784 + 3,349,504 + 25,559,040 + 9,256,960 = 38,284,288 bytes
```

This is about 36.5 MiB, excluding owner headers, other numeric fields, results,
allocator overhead, lifecycle storage, and transient stages/Doom outputs. The
source-pointer inventory is `64 * (2 + 64 + 64) + 5 * (64 * (1 + 4) + 16 + 32)
= 10,160`. Recompute layout-dependent evidence on other targets and after
changing owners. Keep derivations in source for implemented budgets; avoid
unexplained totals.

The remaining choices can be resolved within their items: staging layout and
simultaneous scratch ownership (4), text budgets/ranking scratch (5), selected
history ownership and deterministic disk ordering (6), and Variant lifetime
proofs (7). Begin with retained typed buffers and compact numerical staging; a
reference-counted source arena adds ownership/reuse obligations and is not the
default design. No total-process MiB cap, maximum-load CPU/stack bound, or game
frame-latency benefit has been established.

Initialization failure has structural review and the existing deterministic
partial-failure test, not failure injection at every reservation point. Item 3
reviews did not qualify every allocation-failure path. Cross-platform runtime,
full integration/manual validation, non-UTF-8 diagnostic paths, and first-use or
failed-sink coverage remain to be assessed. Distinguish missing evidence from a
demonstrated defect. There is no unresolved actionable item 3 review finding.

## Modification protocol for this scratchpad

The primary agent owns updates; subagents report facts and proposed changes to
the primary. Keep this a concise current checkpoint rather than a chronological
transcript. Update it at each completed review/check cycle, before every human
commit boundary, and whenever scope, blockers, or decisions change.

- Maintain completed versus pending items, the next permitted action, actual
  commit IDs once known, and explicit user pauses. A document cannot contain its
  own eventual commit hash; record that hash at a later checkpoint only if
  useful. Never invent it or label an uncommitted item committed.
- Record commands, target/toolchain, outcomes, reviewer coverage, actionable
  findings and dispositions, and remaining limitations. If work stops mid-item,
  identify unfinished files, the starting commit, failing checks, and the exact
  next action so a new agent can continue without a live subagent session.
- Keep all needed decisions and reproduction instructions in tracked text. Local
  raw logs/diff hashes may supplement this but cannot be prerequisites. Use
  repository-relative links, method names, and pinned versions; no machine
  paths, secrets, game binaries, or brittle source line citations.
- Update the applicable Rust module docs when a durable behavior or allocation
  contract changes, then summarize the continuation impact here. Avoid copying
  the full schema or player-slot spec into this file. Retire stale proposed
  designs when implementation settles them.
- Run `cargo xtask fmt-md` after edits. This file has a temporary entry in
  [xtask/src/md.rs](xtask/src/md.rs), so `cargo xtask fmt-md --check` and smoke
  cover it. Run applicable documentation gates and `git diff --check`, inspect
  links, and record the result. Do not bypass formatting by wrapping manually.

## Cleanup before the final merge

After item 8's conclusions are captured in durable module docs, tests, and any
remaining issues, remove these three temporary additions in one reviewed change:

1. Delete `STATIC_ALLOC_HANDOFF.md`.
2. Remove only the marked temporary handoff block near the top of `AGENTS.md`.
3. Remove only `"STATIC_ALLOC_HANDOFF.md"` from `xtask/src/md.rs`'s `DOCS` list.

Search the tracked source/docs for the filename and pointer markers, remove
dangling references, run the formatters and required gates, and stop for the
human commit. Never delete AGENTS.md itself or its standing project rules. Keep
ignored scratch/log artifacts uncommitted. Only a human creates commits or PRs
and performs the final merge.
