# Codex adoption scratchpad

Status: active branch scratchpad

This document tracks a multi-session adoption of practices learned from the
local Codex checkout. It is mutable operational state, not permanent project
documentation. Delete it in the final change that merges this branch to main.

## Branch facts

- Branch: `llm-dev-hygiene`
- Worktree: `../spire-profiler-working`
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

## Subagent pairs

Each checklist item (one commit) is executed by an (implementer, reviewer)
subagent pair; this distributes context pressure and buys independent
verification. The orchestrating session writes the specs, integrates the
results, runs the gates, owns this scratchpad, and prompts the human at
commit boundaries.

- The implementer spec carries the motivation, the standard to apply, the
  hard constraints (scope, what not to touch, gate requirements), and the
  deliverable shape.
- Work stops at each commit boundary: the human commits. Present the
  finished change, the gates run, and a suggested commit title, then wait.
- The reviewer is independent: fresh context, never the implementer's
  reasoning. It sees the original material, the changed material, and the
  standard, and audits both directions (anything load-bearing lost, any
  target bloat remaining) plus the mechanics (links, em dashes, diff
  scope). Verdicts: SHIP, SHIP WITH FIXES, REJECT.
- SHIP WITH FIXES sends the findings back to the implementer; substantial
  fixes get a re-review. REJECT starts a fresh implementer round. Reviewer
  nits below the fix bar are applied by the orchestrator directly.
- Em-dash pin lowerings in xtask/src/check_emdash.rs ride along in the
  change that lowers a count.

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

### Corrective pass (before remaining extraction and style work)

Approved 2026-09-09 after the branch review. Each item follows the existing
implementer/reviewer and human-commit workflow. Runtime reproductions live in
the review's machine-local probe harness; permanent regression coverage belongs
with each fix.

- [x] Include private items in the documentation gate, fix the four exposed
      links, and update the gate description.
- [x] Preserve begin/end balance for empty and overflowing contexts; pin the
      surviving outer context through observable ABI tests.
- [x] Persist the full run identity (profile, seed, original start time) and
      rejoin exactly across suspension; test same-seed profiles and replays.
- [x] Use checked combat/run ID allocation; verify debug and release behavior
      and prevent record reuse.
- [ ] Propagate persistence write outcomes so success markers require a
      successful write; exercise filesystem failures.
- [ ] Reject non-finite scroll input, keep accumulation finite, and clear
      pending input while hidden; test recovery and hide/reopen behavior.
- [ ] Harden release packaging with fresh archives, exact staged and archived
      contents, smoke before packaging, and rejection of dirty release inputs.
- [ ] Reconcile state-ownership and reproducibility policy with the code,
      revisit unconditional test deletions using their behavioral coverage,
      and reconcile the remaining workstreams before resuming extraction.

The run-identity change follows the existing disposable-WIP schema policy:
review snapshot changes, add no migrations, and never guess a continuation from
seed alone when reliable identity is missing. The negative-hash and stored-kind
decoder coverage stays until a reviewed replacement or redundancy finding.

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

Markdown trims follow the build.md standard: the doc is a trap register and a
home for its owned canonical facts, not a manual; delete anything `--help`,
the code, or general knowledge already teaches; delete, never relocate.
Tracked files are cited with real relative links (link text names the file);
commands, symbols, and untracked paths stay raw backticks.

- [x] Trim `AGENTS.md` to the build.md standard (judgment-based; no byte
      ceiling).
- [x] Trim `docs/game.md` to the build.md standard (the decompile walkthrough
      shrinks toward traps and `--help`).
- [x] Trim `docs/gdextension.md` to the build.md standard (generic Godot
      mechanics go; the verified empirical findings stay).
- [x] Trim `docs/verify.md` to the build.md standard (light pass).
- [x] Apply soft module-doc budgets: about 50 lines for ordinary modules and 90
      lines for canonical schema, state, and safety owners.
- [-] Deduplicate the unsafe-quarantine policy.
- [-] Split generic GDExtension mechanics from empirical environment guidance.
- [x] Remove child-module dictionaries from `lib.rs` and `data.rs`.
- [x] Remove obvious restatement comments.
- [x] Delete the README roadmap or move it outside the repository.
- [x] Report C# shim comment density before deciding whether to gate it.

### 3. Mechanical test extraction

- [x] Move `ui/chart_layout.rs` tests to a sibling test module.
- [ ] Move `ui/run_layout.rs` tests to a sibling test module.
- [ ] Move `ui/tooltip.rs` tests to a sibling test module.
- [ ] Move `ui/snapshot.rs` tests to a sibling test module.
- [ ] Evaluate `data/persistence/runs.rs` for test extraction.
- [ ] Record inline-test share before and after extraction.

Extraction changes are move-only. Preserve test names, fixtures, and snapshots;
do not rewrite production code in the same change. Insta derives the snapshot
directory from the test's source file, so each extraction git-renames the
file's .snap files into the new sibling's snapshots/ directory (R100, content
untouched; the test module path and snapshot names are unchanged).

Inline-test share at the start of Stage 3 (committed tree, before the first
extraction; the "before" half of the record item): profiler-core/src held
23,553 lines — 4,738 inline-test lines in `#[cfg(test)] mod tests {}` blocks
(20.1%), 4,056 already-extracted sibling-test lines (17.2%), 14,759
production lines. The extraction targets' inline shares: chart_layout.rs
54%, run_layout.rs 58%, tooltip.rs 52%, snapshot.rs 44%; the evaluation
candidate persistence/runs.rs sat at 79%, the highest in the tree.

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
- Markdown trim budgets: no hard byte ceilings, not even the AGENTS.md
  6,000 from the original item. Judgment-based trims to the build.md
  standard, with before/after byte counts in the session log; hard numbers
  would tempt cutting traps to hit a target.
- Markdown file references: real relative links for tracked files (build.md
  already set that style); raw backticks for commands, symbols, and
  untracked paths. Equal rot risk to raw paths, verified at write time.
- Doc rewrites and reviews: delegated to subagents with the standard and
  fact-ownership constraints as context; the orchestrator integrates, runs
  gates, and owns the scratchpad.
- Unsafe-quarantine dedupe and the GDExtension mechanics/empirical split:
  absorbed into the AGENTS.md and gdextension.md trims respectively, which
  are the same edits.
- Shim comment-density gate: not added. The measurement (624 comment /
  1413 code lines, 30.6%) shows ~89% load-bearing content (marshaling
  contracts, engine-fork traps, pixel math) and ~0% restatement; a ceiling
  would punish the project's top-risk documentation, and the file changes
  only with the patch catalog, so a ratchet would guard a non-target.
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

### 2026-09-09: exhausted IDs reject fresh starts safely

Corrective pass, fourth item. Combat and run allocation accept `u32::MAX`,
then fail-log and reject fresh starts instead of overflowing or reusing MAX.
Combat rejection retains the interrupted-combat flush, clears the active
combat and per-combat transients, and preserves open context scopes. Run
rejection closes the previous run normally, clears stale combat, totals, and
player selection, and creates no new run context or success-start log. Exact
continuation of the final run still rebuilds its accumulator; combat recording
outside a run remains available. The persistence module owns the two-line
exhaustion contract. No extra flags or global recording disable were added.

Five new behavioral regressions cover:

- Empty, interrupted, and completed final combats, repeated failed starts,
  transient cleanup, balanced context scopes, and unchanged persisted bytes.
- Initialization from a MAX combat filename with corrupt contents, preventing
  wrapped allocation and overwrites without depending on JSON decoding.
- Exhaustion from either run records or directory names, previous-run closure,
  stale-state cleanup, record preservation, and later out-of-run recording.
- Allocation of the final run ID, failed fresh and mismatched continuation,
  exact continuation after restart, and rebuilding only that run's totals.
- Both run-allocation maxima at MAX-1 and MAX, catching saturation and an
  allocator that ignores either source. Existing allocator expectations now
  reflect its fallible return type.

Mutation checks restored unchecked combat addition and saturating run
allocation temporarily: all four then-new tests failed in both debug and
release. Debug exposed the overflow; release detected combat ID 0; both
detected reused run MAX. The mutations were removed. The combined combat test
was subsequently split into live-transition and initialization cases to meet
the function-length gate; equivalent run-start syntax was shortened without a
lint allowance.

Independent reviewer verdict: SHIP. Gates run: `cargo fmt --all`, workspace
Clippy with warnings denied, targeted Nextest (19/19 after final cleanup),
focused release Nextest (9/9 independently), `cargo xtask smoke` (345/345,
38 ABI bindings, private rustdoc clean, density 10.8%), `cargo xtask fmt-md`,
and `git diff --check`. Em-dash pins remain 40 files / 131 dashes. Scope outside
this scratchpad: 306 changed lines, net +258, including the new regression
module. No schema, snapshot, production unsafe, ABI, dependency, engine,
registration, or shim changes; no live-game or headless run was needed.

Smoke exposed an unrelated reproducibility defect: `unique_dir` combines a PID
and process-local counter but accepts an existing directory. A reused PID read
old run-43 files in `same_seed_replays_never_collide`, yielding max ID 4 instead
of 2. File timestamps confirmed the leftovers preceded the failing run. The
fixtures were preserved in `tmp/unique-before-id-verification`; smoke passed
with a fresh `tmp/unique`. Fix atomic directory reservation and review fixture
isolation in the queued reproducibility item. Persistence write outcomes remain
the next corrective item, after the human commit.

### 2026-09-09: full run identity persists across suspension

Corrective pass, third item. RunSnapshot and each combat's run header now
capture profile, seed, and the original game StartTime. The run record uses
that captured profile, so metadata for a later run cannot relabel the previous
one. The duplicated context timestamp is removed. Missing metadata stays
unknown (-1 profile, 0 time); invalid negative wire values clamp and fail-log.
Initialization explicitly sets the unknown profile instead of accepting the
derived default's 0.

Continuation and history share a unique exact identity match across run
records and combat fragments. Unknown components, run ID 0, and ambiguous IDs
cannot match. Rebuilds and every history rollup exclude foreign identities even
under the same numeric run ID. The combat-time proximity heuristic is deleted,
which also removes the timestamp-distance overflow from the next corrective
item; only checked ID allocation remains there.

Five new tests cover later metadata, unknown live identities, corrupt/missing
stored components, ambiguous IDs, and foreign fragments sharing an ID. Existing
resume coverage now suspends, records same-seed profiles and replays, resets
state, and checks the original run's rebuilt totals and timestamp. Existing
rebuild and fallback tests pin exact membership instead of seed/time guesses.
The self-test and simulation use deterministic original run times; the
self-test's metadata order matches the shim and its scripted sequence has a
local function-length lint allowance. The combat snapshot adds profile/time
and a known fixture seed; run JSON bytes stay unchanged, with only its snapshot
expression metadata updated. No migration or real-data deletion occurred;
older combat headers without full identity remain unmatchable.

Independent reviewer verdict: SHIP, including the final lint allowance and
schema diffs. Gates run: `cargo fmt --all`, `cargo xtask fmt-md`, targeted
Nextest (94/94), `cargo xtask check-abi` (38 bindings), focused release Nextest
(48/48), `cargo xtask smoke` (340/340, private rustdoc clean, density 10.9%),
and `git diff --check`. Smoke first exposed the self-test length lint, then
passed after the local allowance. Em-dash pins are 40 files / 131 dashes.
The 795 changed lines outside this scratchpad are wider than the rough budget;
the reviewer accepts the single logical change because identity must cross
state, both serializers, matching, and aggregates together, with explicit
fixture and regression updates. Net source/test/doc change: +277 lines.
No production unsafe, ABI signature, engine, registration, or executable shim
changes. The local shim confirms all four new/saved setup paths forward the
identity before run start; the v0.111.0 decompiled game tree is absent, so no
new game-body verification or live-game/headless run was performed. Next item:
checked combat/run ID allocation, after the human commit.

### 2026-09-09: rejected context scopes keep their balance

Corrective pass, second item. A private ContextStack retains empty logical
frames and counts rejected scopes above the 32-frame cap, so their ends cannot
pop an accepted outer source. Every consumer resolves the nearest named frame;
valid named descendants of empty scopes still attribute normally. Accepted
named begins retain the existing last_source and fallback behavior. Checked
counter exhaustion freezes unwinding until whole-state reset, preserving outer
sources without a panic or wrap. Turn, combat, and run boundaries keep open
scopes intact.

Four new tests pin distinct failures: observable ABI damage/block attribution
after null, empty, and non-UTF-8 begins; nested overflow across turn/combat
boundaries and subsequent recovery; counter exhaustion and reset; and a
256-event seeded comparison against an independent full-scope model. Distinct
outer IDs prevent last_source from hiding a bad pop. Temporarily restoring the
original empty/overflow rejection behavior made both ABI regressions and the
model fail; the mutation was removed. The direct stack-length check was removed
from the integration simulation with the private stack representation; the
capped push path enforces storage bounds and the model checks attribution at
the cap.

Independent reviewer verdict: SHIP. Gates run: `cargo fmt --all`,
`cargo xtask fmt-md`, targeted Nextest suites (23/23 at the default seed,
35/35 with SIM_SEED=42, then 4/4 restored regressions),
`cargo xtask check-abi` (38 bindings), `cargo xtask smoke` (335/335, density
11.1%), `cargo test --locked -p profiler_core --release context_ --
--test-threads=1` (5/5), and `git diff --check`. Source/test scope is 348 changed
lines, net +262; the modest boundary-change size overage covers separate ABI,
counter-exhaustion, and independent-model regressions. No production unsafe, ABI signature,
schema, snapshot, shim, or engine changes. No live-game/headless run; engine and
shim behavior are unchanged. Next item: full persisted run identity, after the
human commit.

### 2026-09-09: private documentation checked

Corrective pass, first item. The implementer enabled
`--document-private-items` and reproduced the four unresolved links before
fixing them: State and fail_call_failed now resolve to their definitions;
the literal relic and orb prefixes render as code. The gate retains
`--locked` and warnings-as-errors, and verify.md names its expanded scope.
Independent reviewer verdict: SHIP, including inspection of the generated
link targets and prefix rendering. Gates run: `cargo xtask check-docs`
(failed before the fixes, passed after), `cargo fmt --all`,
`cargo xtask fmt-md`, `cargo xtask smoke` (331/331, 38 bindings, density
11.2%), and `git diff --check`. No runtime behavior, ABI, schema, test, or
snapshot changes; no headless validation needed for this gate/doc change.
The approved corrective sequence is recorded above and the worktree path
is corrected. Next item: context begin/end balance, after the human commit.

### 2026-09-08: chart_layout tests extracted

Stage 3, first extraction. Implementer subagent moved the inline test module
to ui/chart_layout/tests.rs (chart_layout.rs 2037 to 946 lines, tests 1092
lines of 2037 = 54%); the two insta snapshots became pure renames into
ui/chart_layout/snapshots/ (insta derives the directory from the test's
source file; content untouched, names stable since the module path is
unchanged). No em-dash pin moves: the tests' dashes are string literals,
not comments. Independent reviewer verdict: SHIP (move byte-fidelity, R100
renames, 31 chart_layout tests before and after, 331/331). Gates run:
`cargo xtask smoke` (331/331, density 11.2%).

### 2026-09-08: shim comment density reported; no gate

Stage 2, shim-density item. Measurer subagent classified all 2121 lines of
shim/shim.cs.template (throwaway scanner in tmp/, nothing repo-added): 624
comment, 1413 code, 84 blank, density 30.6% under the check-docs metric;
zero block comments, zero trailing comments, zero TODOs. Independent
verifier confirmed the totals exactly and corrected the secondary claims:
top clusters are StyleRunButton 43, ModifierDecomposition 37,
AttachRunPanelTo 33 (doc block plus body, consistent definition), and the
composition is ~89% load-bearing, ~10% boilerplate (62 XML doc tag lines),
0.8% banners, ~0% restatement. 181 of 244 members carry no comments; the
density sits on the trap sites (Harmony-patch correctness, ABI contracts,
pixel math). Gate decision: none (decision log); a 15% ceiling would have
to cut mostly load-bearing content.

### 2026-09-08: obvious restatement comments removed

Stage 2, restatement-comments item. Audited every plain `//` comment, the
trailing comments, and the `///`/`//!` docs across the three in-house Rust
roots; the honest yield was six deletions and no trims, matching an already
comment-disciplined tree. Deleted: self_test.rs's "BASH hits an enemy" step
narration (its siblings name the behavior each step pins), power.rs's
predicate re-wording over the debuff-layer branch, events.rs's stale
"conversion happens once, here" (init has taken &Path since publicize),
build.rs's name-carried build_matrix line, headless.rs's "print and
capture" narration over the polling loop, and release.rs's "one zip per
matrix row" loop label. Kept everything carrying an invariant, a why, a
trap, a wire decode, or a SAFETY contract; the gdext.rs section banners
stay because each carries provenance or a constraint. No em dashes removed,
so no pin moves (41 files, 137 dashes). Independent reviewer verdict:
SHIP, all six deletions ruled safe (the closest, power.rs's debuff-layer
label, holds because the enemy decode lives at the boundary function).
Gates run: `cargo xtask smoke` (331/331, density 11.2%).

### 2026-09-08: module-doc budgets applied

Stage 2, module-doc budgets item. The inventory found 76 files with module
docs and only 6 over budget, so one implementer/reviewer pair did the whole
item: data.rs 98 to 88 (treated as canonical at 90: it owns the attribution
model per the fact-ownership table), gdext.rs 101 to 90, persistence.rs 97
to 90, run_history.rs 70 to 50, ledger.rs 59 to 48, theme.rs 59 to 47.
Cuts targeted facts verifiably owned by other module docs; the reviewer
confirmed each against the owner with quoted lines, verified budgets,
rustdoc, and pin actuals (ratchet now 41 files, 137 dashes), and found the
theme.rs trim strictly stronger (its pixel numbers were already carried by
named constants with const asserts). One non-blocking nit left as is
(run_history's settings trio generalized; the invariant survives). Gates
run: `cargo xtask smoke` (331/331, density 11.2%).

### 2026-09-08: subagent-pair practice persisted

The (implementer, reviewer) pair rule is now written into the operating
rules so future sessions inherit it: independent reviewers with fresh
context, SHIP / SHIP WITH FIXES / REJECT verdicts, the orchestrator
integrating and owning the scratchpad.

### 2026-09-08: child-module dictionaries removed

Stage 2, dictionaries item. Implementer subagent deleted lib.rs's `# Layers`
section and data.rs's dictionary paragraph; the unsafe-quarantine sentence
now names `abi`, `registration`, `engine::gdext` directly (more accurate
than the deleted bullet, which credited `engine` broadly). Pins lowered
10 to 5 (lib.rs) and 13 to 7 (data.rs); the ratchet is at 41 files, 152
dashes. Independent reviewer verdict: SHIP (scope, completeness, rustdoc,
and pin actuals all verified). Gates run: `cargo xtask smoke` (331/331,
density 11.4%).

### 2026-09-08: README trimmed, roadmap deleted

Stage 2, README roadmap item plus a light pass. Implementer subagent deleted
the Roadmap section outright (its items appear nowhere else in the tree)
and applied the standard: one restated clause dropped, "Where things live"
compressed, file references linked, the file's one em dash removed. 1,857
to 1,737 bytes (48 to 41 lines). Independent reviewer verdict: SHIP. Gates
run: `cargo xtask smoke` (331/331).

### 2026-09-08: em-dash pins lowered after the doc trims

The deferred pins-hygiene change of the away-mode plan: the four markdown
pins that went to zero (README, game.md, gdextension.md, verify.md) left
the PINS table; the ratchet is at 41 files, 163 dashes. Gates run: `cargo
xtask smoke` (331/331, density 11.5%).

### 2026-09-08: verify.md trimmed to the build.md standard

Stage 2, verify.md trim. Implementer subagent cut 4,284 to 3,394 bytes (78
to 62 lines): intro signposting, glosses duplicating AGENTS.md policy, the
marker enumerations canonical in headless.rs, and machine-local tmp/ trivia
went; the gate list and every headless trap stay, em dashes 4 to 0.
Independent reviewer verdict: SHIP, confirming the smoke step list matches
smoke() step for step and every named trap survives. Gates run: `cargo
xtask smoke` (331/331, density 11.5%).

### 2026-09-08: gdextension.md trimmed to the build.md standard

Stage 2, gdextension.md trim (absorbs the mechanics/empirical split item).
Implementer subagent cut 6,468 to 2,182 bytes (96 to 33 lines): generic
Godot mechanics and everything the gdext.rs and panel.rs module docs own
were deleted; the four empirical findings (InputEvent freeze evidence,
trackpad wheel state, theme-font trap, get_mouse_button_state silent
failure) stay with their version pins. Independent reviewer verdict: SHIP,
confirming every cut against the owning module docs line by line, including
the implementer's least-sure cut (panel draw order, owned by panel.rs).
Gates run: `cargo xtask smoke` (331/331, density 11.5%).

### 2026-09-08: game.md trimmed to the build.md standard

Stage 2, game.md trim. Implementer subagent cut 9,962 to 5,761 bytes (183 to
109 lines): the decompile walkthrough collapsed to its traps (the GDRE
SIGUSR1 and unwritable-HOME signal 11), discovery mechanics compressed to
operator facts, duplication with AGENTS.md removed, em dashes 14 to 0.
Independent reviewer verdict: SHIP WITH FIXES; the implementer restored the
developer quote (the decompile permission's primary evidence) and the two
Linux Steam roots, and compressed a duplicated check-catalog list. Gates
run: `cargo xtask smoke` (331/331, density 11.5%).

### 2026-09-08: AGENTS.md trimmed to the build.md standard

Stage 2, AGENTS.md trim. Implementer subagent cut 12,710 to 10,030 bytes
(272 to 200 lines): merged the overlapping comment-rule lists, replaced both
bad/good Rust example pairs with one-line prose, folded "State of this mod"
into General, converted file references to real relative links, and removed
AGENTS.md's two em dashes (its pin left the table; the ratchet is at 45 pins,
197 dashes). Independent reviewer verdict: SHIP; its three optional findings
were applied directly (dropped tool-output narration and a design-goal-1
restatement, restored the six-word catalog-remediation clause in Game
updates). Gates run: `cargo xtask smoke` (331/331, density 11.5%).

### 2026-09-08: markdown trims planned across the doc set

Extended the Stage 2 AGENTS.md item to the whole markdown set at the user's
direction: build.md is the standard (trap register plus owned canonical
facts, never a manual), AGENTS.md first, then game.md, gdextension.md,
verify.md, README. No hard byte ceilings anywhere. File references become
real relative links for tracked files. Rewriting and review are delegated
to subagents per doc; pin lowering in check_emdash.rs rides along in each
trim commit. No doc edited yet.

### 2026-09-08: line scanner extracted to scan.rs

Follow-up to Stage 1, item 5. check_emdash importing check_docs' internals
was the crate's only gate-to-gate import; the line classifier (LineKind,
LineScanner, count_lines, and the string/char helpers) and its six tests
moved verbatim into a new mechanism module `scan.rs`, matching the
discover/catalog/md pattern of mechanism modules the gates consume.
Move-only: no logic changed, test names preserved, check-emdash and
check-docs output identical. Gates run: `cargo xtask smoke` (331/331,
density 11.5%, 38 bindings, 46 pins).

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
