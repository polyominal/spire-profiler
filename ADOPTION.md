# Codex adoption scratchpad

Status: active branch scratchpad

This document tracks a multi-session adoption of practices learned from the
local Codex checkout. It is mutable operational state, not permanent project
documentation. Delete it in the final change that merges this branch to main.

## Branch facts

- Branch: `llm-dev-hygiene`
- Worktree: `../spire-profiler-llm-dev-hygiene`
- Base commit: `67bc66e`
- Source comparison: local Codex commit `6750f5bd13`
- Human rule: agents do not create commits or pull requests.

## New-session orientation

This worktree is the only workspace for this branch. The original checkout stays
on `main`; do not make adoption changes there. Conversation history from earlier
agent sessions is unavailable. This file, the repository files, and Git status
are the authoritative state.

A new session should:

1. Set the working directory to this worktree.
2. Read this file and the root `AGENTS.md`; both apply.
3. Run `git status --short --branch` and reconcile uncommitted files with the
   checklists below before editing.
4. Inspect the owning source or doc before changing a checklist item.
5. Update the checklist and append a session note in the same working session.

Checklist semantics:

- Mark `[x]` only after implementation and its relevant gate pass.
- Mark `[-]` only with a decision-log reason.
- Use `[~]` while work is intentionally left incomplete.
- Do not mark an item complete merely because code was written.

## Baseline at branch creation

At base commit `67bc66e`, before adoption changes:

- `cargo xtask smoke` passed.
- `cargo nextest run --workspace` ran 325 tests, all passing.
- `cargo xtask check-docs` passed and reported 11.5% comment density.
- `cargo xtask check-abi` verified 38 shim bindings.
- The tracked tree was clean.
- In-house Rust was about 29,700 physical lines.
- Test code was approximately 10,400 lines, about 35% of audited Rust.
- `AGENTS.md` was 12,133 bytes.
- Tracked Markdown totaled 36,477 bytes.
- There were 272 em dashes across Markdown, Rust, and the C# shim: 37 in
  Markdown, 173 in Rust, and 62 in the shim template.
- `shim/shim.cs.template` had about 2,100 lines and 620 comment-bearing lines.
- Several UI files were more than half inline tests: `run_layout.rs`,
  `chart_layout.rs`, and `tooltip.rs`.
- The largest combined source/test files were `ui/chart_layout.rs`,
  `engine/gdext.rs`, `ui/panel.rs`, and `tests/sim.rs`.

The Codex comparison came from local commit `6750f5bd13`; it was inspected
read-only and is not a dependency of this branch.

## Fact ownership

| Fact | Canonical owner |
|---|---|
| Cross-cutting agent policy | `AGENTS.md` |
| Crate layers and standing contracts | `profiler-core/src/lib.rs` |
| Attribution model | `profiler-core/src/data.rs` |
| Player-slot and team model | `profiler-core/src/data/state.rs` |
| JSON schema and write protocol | `profiler-core/src/data/persistence.rs` |
| GDExtension unsafe contracts | `profiler-core/src/engine/gdext.rs` |
| Empirical GDExtension findings | `docs/gdextension.md` |
| Build environment | `docs/build.md` |
| Verification gates | `docs/verify.md` and xtask implementation |
| Game layout and update drift | `docs/game.md` |
| C ABI surface | Rust exports plus `xtask/src/check_abi.rs` |
| Harmony hook catalog | `xtask/src/catalog.rs` and generated shim |

A change may mention another owner briefly, but it must not duplicate that
owner's canonical explanation.

## Non-goals

- Do not split profiler-core into a Codex-scale crate graph.
- Do not add async state ownership, locks, or channels.
- Do not add Bazel, generated protocol schemas, cargo-deny, cargo-shear, or a
  platform CI matrix.
- Do not adopt agent-created commits, branches beyond this one, or automated PR
  workflows.
- Do not turn this scratchpad into permanent documentation.

## Operating rules for this branch

1. Prefer deleting text and tests over adding replacements.
2. Mechanical moves, behavior changes, documentation edits, and punctuation
   cleanup do not share one change.
3. A test may remain only when it catches a distinct behavioral failure that no
   type, compile-time assertion, or reviewed snapshot already pins.
4. Canonical facts stay in their owning module docs. `AGENTS.md` keeps only
   cross-cutting agent policy.
5. Preserve unsafe contracts, capacity rationale, persistence identities, and
   deterministic-model documentation unless a replacement is strictly clearer.
6. Run `cargo xtask smoke` after source or documentation changes. Run
   `cargo xtask check-abi` after ABI or shim changes. Run `cargo xtask
   headless-test` after engine, registration, or shim behavior changes.
7. Before deleting an uncertain test, name the implementation mutation it is
   expected to catch. If no such mutation exists, delete the test.

## Review and handoff shape

A nontrivial change should be reviewed along six dimensions:

1. C ABI and GDExtension containment and pointer lifetime.
2. Game-version and Harmony-hook drift.
3. Persistence schema, atomicity, and snapshot impact.
4. Test value and deterministic coverage.
5. Context economy and canonical fact ownership.
6. Release, ABI, bundle, and platform impact.

The completing session must hand off:

- why the change was needed;
- the net change only, not abandoned attempts;
- safety impact;
- each new or changed test and the failure it catches;
- verification commands actually run;
- remaining machine-local or untested risk;
- context added, deleted, or moved.

Additional operating rules:

- Keep complex logic or boundary changes under roughly 300 changed lines and
  ordinary changes under roughly 500. Mechanical moves may exceed that only by
  remaining move-only.
- Never configure `panic = "abort"`; panic containment depends on unwinding.
- A future native engine resource must enter safe Rust through one owner type
  whose `Drop` states the exact release rule.
- Do not add `pretty_assertions` merely to imitate Codex. Consider it only if
  whole-object assertion diffs become a recurring review problem.

## Progress legend

- [ ] Not started
- [~] In progress
- [x] Done
- [-] Dropped; record the reason

## Workstreams

### 0. Resolve contradictory contracts

- [x] Choose persistence policy: disposable WIP data or additive-only records.
- [x] Make the 15% Rust comment-density threshold a hard gate, or document the
      existing behavior as a checkpoint warning.
- [x] Make `SIM_SEED` parsing fail loudly on malformed input.
- [x] Make simulation start times deterministic or weaken the byte-for-byte
      reproducibility claim.
- [x] Include seed and scenario in simulation failure messages.

### 1. Gate wiring

- [x] Add `check-abi` and `check-docs` to `smoke`.
- [x] Add `--locked` to Cargo doc, clippy, nextest, and zigbuild invocations.
- [x] Consider explicit Nextest `--no-fail-fast`.
- [x] Add profiler-core lints for direct printing and production `unwrap`.
- [x] Add a targeted em-dash ratchet for Markdown and Rust comments.

### 2. Context economy

- [ ] Rewrite `AGENTS.md` to under 6,000 bytes without moving detail elsewhere.
- [ ] Apply soft module-doc budgets: about 50 lines for ordinary modules and 90
      lines for canonical schema, state, and safety owners.
- [ ] Deduplicate the unsafe-quarantine policy.
- [ ] Split generic GDExtension mechanics from empirical environment guidance.
- [ ] Remove child-module dictionaries from `lib.rs` and `data.rs`.
- [ ] Remove obvious restatement comments.
- [ ] Delete the README roadmap or move it outside the repository.
- [ ] Report C# shim comment density before deciding whether to gate it.

### 3. Mechanical test extraction

- [ ] Move `ui/chart_layout.rs` tests to a sibling test module.
- [ ] Move `ui/run_layout.rs` tests to a sibling test module.
- [ ] Move `ui/tooltip.rs` tests to a sibling test module.
- [ ] Move `ui/snapshot.rs` tests to a sibling test module.
- [ ] Evaluate `data/persistence/runs.rs` for test extraction.
- [ ] Record inline-test share before and after extraction.

Extraction changes are move-only. Preserve test names, fixtures, and snapshots;
do not rewrite production code in the same change.

### 4. Test-value audit

- [ ] Delete `u64_from_hash_sign_extends`.
- [ ] Delete `from_u8_round_trips_all_kinds`.
- [ ] Delete `run_manual_visible_cycles`.
- [ ] Delete `dismiss_run_manual_lands_on_hidden`.
- [ ] Trim redundant source-kind and run-outcome wire-code sweeps.
- [ ] Rewrite `outcome_serde_round_trips_lowercase_and_reads_unknowns_as_defeat`
      as either complete round-trip coverage or corrupt-record coverage.
- [ ] Move fixed persistence-shape checks out of the randomized simulation.
- [ ] Consolidate `check_run_and_store_files` with `check_written_files`.
- [ ] Replace the field-by-field `CardStat` comparison with whole-object
      equality.
- [ ] Strengthen null-string ABI testing with observable state.
- [ ] Add a valid non-UTF-8 C-string test.
- [ ] Strengthen or delete the commit-hash shape-only test.

Preserve the deterministic simulations, independent block-pool model,
allocation tests, ABI failure-mode tests, persistence tests, and reviewed Insta
snapshots unless a concrete replacement is stronger.

### 5. API and ABI hygiene

- [ ] Trial `unreachable_pub`, `private_interfaces`, and `private_bounds` in
      profiler-core.
- [ ] Narrow top-level module visibility where practical.
- [ ] Evaluate safe `extern "C"` exports when their bodies contain no unsafe
      operation.
- [ ] Recheck ABI and headless behavior after any ABI refactor.

### 6. Release hardening

- [ ] Validate the exact universal bundle layout before zipping.
- [ ] Validate each target staging directory before zipping.
- [ ] Run `smoke` before release packaging.
- [ ] Reject dirty release inputs or encode a dirty marker.
- [ ] Optionally add one minimal CI job if shared-branch verification becomes
      useful.

### 7. Final branch cleanup

- [ ] All relevant gates pass.
- [ ] No temporary policy remains in permanent docs.
- [ ] Remove the branch-only `ADOPTION.md` pointer from `AGENTS.md`.
- [ ] Delete this scratchpad in the final human-reviewed merge change.

## Decision log

- Worktree and branch: accepted. The branch is short-lived, and the final merge
  remains human-made.
- Codex topology: rejected. Keep the profiler-core and xtask split only.
- Broad Unicode ban: rejected for now. Start with an em-dash ratchet and an
  explicit runtime-glyph allowlist.
- Persistence evolution: resolved as disposable WIP data. AGENTS.md already
  mandated no-migrations with old data deleted; the additive-only contract
  in the persistence and records module docs was the contradiction and now
  frames parse leniency as boundary robustness instead.
- `--locked` scope: kept on the workspace gate invocations despite Codex
  limiting the flag to `cargo install`. Codex enforces workspace lockfile
  freshness with a CI clean-worktree gate; this branch has no CI and smoke
  runs on dirty worktrees, so the flag is the local equivalent.
- Em-dash ratchet: loose, not strict. Pins are ceilings that only descend;
  failing on stale pins (check-catalog style) would punish cleanup, the
  behavior the ratchet exists to encourage. The justification is readability
  first (the dash can stand in for comma, colon, semicolon, or parentheses,
  deferring the clause relation to the reader), machine-generated-tell second.
  Scope: the fmt-md markdown set and xtask/src whole-file; profiler-core
  counts comments only, keeping player-visible typography and the renderer
  glyph allowlist exempt.
- Comment budget enforcement: resolved as a hard gate. AGENTS.md states the
  budget as mandatory; check-docs now fails above 15% after printing the
  offender report, and the breach path was verified by temporarily lowering
  the limit (exit 1 with a named error).

## Session log

### 2026-09-08: em-dash ratchet wired into smoke

Stage 1, item 5. New `check-emdash` gate pins per-file em-dash ceilings (46
files, 199 dashes, generated by the checker's own classifier) and fails only
on increases; drops below a pin print a lowerable nudge. The comment/code
classifier moved from check_docs internals to shared visibility, and
md::DOCS is reused rather than duplicated. AGENTS.md records the rule and
verify.md names the new smoke step. Built by a worker subagent to spec; the
above-pin and unpinned failure modes and the lowerable nudge were each
verified empirically and reverted. Gates run: `cargo xtask smoke` (331/331,
density 11.5%, 38 bindings).

### 2026-09-08: print and unwrap lints, Codex layout

Stage 1, item 4. Adopted Codex's two-mechanism split (confirmed by a
read-only subagent against the pinned checkout): `unwrap_used = "deny"`
joined the workspace lints table both crates inherit, and
`#![deny(clippy::print_stdout, clippy::print_stderr)]` went to
profiler-core's crate root — printing policy is per-crate, and xtask is the
CLI that prints deliberately. clippy.toml gained `allow-unwrap-in-tests`,
which excuses every existing test unwrap; no production unwrap existed.
`emit`'s writeln through a StderrLock did not trip print_stderr, so no
`#[expect]` exception was needed. Both denies were verified live by injecting
an eprintln! and a production unwrap into lib.rs: clippy failed naming both
lints; the probe was then removed. `expect_used` deliberately stays allowed —
AGENTS.md sanctions expect over unwrap. Gates run: `cargo xtask smoke`
(325/325, density 11.5%).

### 2026-09-08: nextest runs with --no-fail-fast

Stage 1, item 3. Nextest's default cancels the run on the first failure, so a
red smoke reported only the first failing test; the gate now passes
`--no-fail-fast` (matching every Codex nextest invocation, local and CI), so
one red run names every failure, each with its repro string. Verified with a
temporary failing assertion: the failing test ran 324th and test 325 still
ran, ending "324 passed, 1 failed" instead of a cancelled run; the probe was
then removed and `cargo xtask smoke` passes (325/325).

### 2026-09-08: --locked usage validated against the Codex comparison

Consulted the pinned Codex checkout for its `--locked` practice via a
read-only subagent. Codex passes `--locked` only to `cargo install` of
third-party tools (this repo's install specs already do) and never to
workspace build/test/clippy/nextest; lockfile freshness is instead enforced
by a CI clean-worktree gate that fails when cargo rewrites Cargo.lock. That
mechanism does not transfer: this branch has no CI and smoke must pass on
dirty worktrees, so the flags on the gate invocations are the local
equivalent and stay. No code changed. One corroboration for the next Stage 1
item: Codex's nextest invocations pass `--no-fail-fast`.

### 2026-09-08: cargo invocations run with --locked

Stage 1, item 2. `cargo doc` (check-docs), `cargo clippy` and `cargo nextest`
(smoke), and `cargo zigbuild` (the cross build) now pass `--locked`, so a gate
fails instead of silently resolving against a stale Cargo.lock; verify.md's
smoke step list names the new flags. The zigbuild invocation is
compile-checked but not run end-to-end here (it needs the zig and dotnet
bootstraps); the installed cargo-zigbuild's help lists `--locked`. Gates run:
`cargo xtask smoke` (325/325, density 11.5%, 38 bindings). Also restored the
comrak-canonical two-space blank line before the settings.save fence in
verify.md: something stripped it after the last session, leaving HEAD failing
`fmt-md --check`.

### 2026-09-08: smoke runs the ABI and doc gates

Stage 1, item 1. `smoke` stopped at fmt, fmt-md, check-citations, clippy, and
nextest, so the commit gate could pass with a broken shim ABI or an
over-budget comment density. Both checks are now smoke steps: check-abi joins
the cheap text scans before clippy, and check-docs runs after clippy (its
`cargo doc` is compile-priced) and before nextest. The verify.md gate list
folds the standalone check-abi bullet into the smoke step list so the gate
set is still stated once. Gates run: `cargo xtask smoke` (every step green,
including the two new ones; 325/325 tests, density 11.5%, 38 bindings).

### 2026-09-08: persistence policy resolved

Stage 0, item 1. Chose disposable WIP data per the standing AGENTS.md policy
and rewrote the contradicting additive-only contract in the `persistence`
module doc, the `records` module doc, and one test comment; removed a field
doc that only restated the schema's zero-omission rule. Documentation-only
change, no behavior touched. Gates run: `cargo xtask smoke` (325/325) and
`cargo xtask check-docs` (rustdoc clean, density 11.5%). Remaining risk:
none identified; the decision is reversible by editing the same docs.

### 2026-09-08: comment-density gate made hard

Stage 0, item 2. check-docs warned instead of failing while AGENTS.md states
the 15% budget as mandatory; exceeding it is now an error after the offender
report prints. Gates run: `cargo xtask check-docs` (passes at 11.5%, breach
path verified with a temporarily lowered limit) and `cargo xtask smoke`
(325/325).

### 2026-09-08: SIM_SEED fails loudly

Stage 0, item 3. A malformed SIM_SEED silently fell back to the default seed,
replaying a different walk than the one being debugged; it now panics naming
the bad value. Verified with SIM_SEED=notanumber (test fails with the new
message) and a clean `cargo xtask smoke` (325/325).

### 2026-09-08: sim reproducibility claim made precise

Stage 0, item 4. Chose to weaken the claim rather than add clock machinery:
`combat_started` always wall-clock-stamps the record (the ABI has no combat
start time) and the walk forwards no run StartTime, so persisted bytes were
never byte-for-byte reproducible. Nothing in the walk reads a timestamp, so
the module doc now claims exact replay of the event stream and assertion
failures, naming persisted start times as the one nondeterminism. `cargo
xtask smoke` passes (325/325).

### 2026-09-08: sim failure messages carry the repro

Stage 0, item 5. Every sim assert and failure-path panic now leads with a
repro string ("SIM_SEED=<base> scenario <n>" in the lifecycle walk, the seed
alone in the block-pool test), so a failure message is enough to replay it.
Context-free expects in the sim's checkers became panics carrying the same
prefix. Verified by breaking an invariant under SIM_SEED=42 (message led
with "SIM_SEED=42 scenario 0 step 0") and reverting; `cargo xtask smoke`
passes (325/325). One side effect: the block-pool test crossed clippy's
too-many-lines limit and now carries the file's allow-with-justification
pattern.

### 2026-09-08: block-pool comparisons iterate deterministically

Review found that the independent block-pool credit model used a HashMap, so
when multiple credits disagreed the first failing assertion could vary with map
iteration order. The model now uses a BTreeMap, preserving sorted comparison
order for a given seed. This completes the exact assertion-failure promise in
the simulation module doc.

### 2026-09-08: setup

Created the branch and this scratchpad from `67bc66e`. No source behavior was
changed yet. Baseline `smoke`, nextest, `check-docs`, and `check-abi` passed in
the original workspace before branch creation.
