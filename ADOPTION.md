# Codex adoption handoff

Temporary handoff for branch `llm-dev-hygiene`, based on `67bc66e`.
This file, the repository, and Git status suffice to continue; no earlier chat, Codex checkout, or saved `tmp/` fixtures are required.
Delete this file and its branch-only pointer in [AGENTS.md](AGENTS.md) before the final human merge to `main`.

## Start here

1. Read [AGENTS.md](AGENTS.md) and this file; run `git status --short --branch` and reconcile local changes before editing.
2. Complete priority fixes before extraction or cosmetic work, one bounded logical change through implementation, independent review, gates, and a human commit.
3. Replace the checkpoint at handoff; prune completed material instead of appending a session log.

`[ ]` means pending. Missing prerequisites leave affected verification pending while independent work continues.
A sufficient continuation prompt is: “Continue ADOPTION.md from the next pending item, using its implementation/review workflow.”

## Current checkpoint

Uncommitted checkpoint: fresh, isolated fixtures, based on clean HEAD `f81aaed39b9f94f028b343e398543e52b39f5943` on 2026-09-09. That HEAD is the documentation-only handoff audit; the last committed implementation remains `2c6555c` (release packaging hardening).
Implementation, independent review (SHIP), and required checks are complete. Stop for the human commit before the next correction, trustworthy headless verdict.

Completed work:

- Smoke covers ABI, private rustdoc with warnings denied, comment density, and documentation hygiene. Inner Cargo gate/build calls use `--locked`; the outer alias gap remains below. Nextest uses `--no-fail-fast`; production printing/unwrap lints and the em-dash ratchet are installed. Canonical documentation ownership, trims, soft module-doc budgets, and the shared line scanner landed.
- Corrective fixes preserve context-scope balance/attribution, exact run identity across suspension, checked ID exhaustion, write-dependent persistence success/cache invalidation, and finite scroll input with hide/dismiss/reopen expiry. Private rustdoc fixed four broken links/rendering cases. Simulation seed errors fail loudly, failures identify seed/scenario, and model comparisons use deterministic order.
- Release requires clean committed input, smoke, exact bundle/stamp validation, explicit archive members, and checksums after successful replacements. Tests pin archive entries, extracted bytes, checksum rows, and failures. Replacement is atomic per file, not across the archive set; target input staging trees are gone.
- [chart_layout.rs](profiler-core/src/ui/chart_layout.rs) tests moved into [tests.rs](profiler-core/src/ui/chart_layout/tests.rs): production file 2,037 to 946 lines, 31 tests preserved, two snapshots renamed content-identically with unchanged module paths.
- [test_util.rs](profiler-core/src/test_util.rs) reserves fresh empty directories with atomic `create_dir`, retries only occupied candidates, and reports other setup errors with paths. `wiped_dir` and its destructive setup are removed; profiler callers retain isolated fixtures under `tmp/unique/`. [discover.rs](xtask/src/discover.rs) `FakeTree` owns `xshell::TempDir`. Four new tests cover occupied directory/file sentinels, same-label contention, parent/candidate errors, and independent teardown. Existing behavior tests and snapshots are preserved; the 15-file source diff adds 249 and deletes 140 lines, chiefly helper replacement and caller renames.

Verification:

- `cargo xtask smoke` passed 360/360 tests, workspace Clippy, private rustdoc, formatting, citations, 38 ABI bindings, and em-dash checks. Rust comment density is 10.5% (3,080 / 29,361); em-dash pins remain 39 files / 130 dashes.
- Targeted checks passed: `cargo test --package profiler_core --locked test_util::tests` (3), `cargo test --package profiler_core --locked same_seed_replays_never_collide` (1), and `cargo test --package xtask --locked discover::tests` (10). `cargo fmt --all`, `cargo xtask fmt-md`, and `git diff --check` passed.
- After `cargo build --package profiler_core --features test-support --locked`, a standalone Rust probe linked that library and started four overlapping child processes with the same fixture label. Each reserved a directory, wrote its PID sentinel, and waited on stdin until all four reservations completed. All paths differed and every sentinel survived, including after process exit.
- Before source or handoff edits, `ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-cache" cargo xtask release` passed from clean committed HEAD `f81aaed39b9f94f028b343e398543e52b39f5943`: smoke (356 tests), all four native targets, the pinned v0.111.0 Windows game discovery, C# shim (zero warnings/errors), and five ZIPs under `dist/`. `sha256sum --check SHA256SUMS` passed there; separate archive inspection verified exact members, matching bundle content hashes, and stamp `v0.111.0-f81aaed3`. Native cross-compilation emitted macOS SDK-discovery and deprecated-linker-setting warnings but completed successfully. These archives precede the uncommitted fixture change.
- GNU tar and BSD tar previously passed gzip/xz long-option extraction probes; dirty release rejection remains covered by smoke.
- Headless at `d6f75b2` passed on Windows game v0.111.0 through WSL: 194 patches, draw/persistence markers, exit 0, no unexpected profiler errors. This covers combat-panel boot/draw, not interactive scrolling or the lazy history panel. Subsequent packaging and test-fixture changes affect no engine, registration, shim, or ABI behavior, so require no later headless run.
- Fixture/error execution is verified on Linux/WSL only. macOS runtime and Windows/macOS filesystem failure execution remain unverified.

Follow [build.md](docs/build.md), [verify.md](docs/verify.md), and [game.md](docs/game.md). Record missing tools or mismatch with [game_version.rs](xtask/src/game_version.rs); never silently bump `PIN` or weaken gates.

## Priority corrections

### 1. Trustworthy headless verdict

- [ ] In [headless.rs](xtask/src/headless.rs), require successful termination as well as markers/error checks. `run_game_captured` accepts nonzero/signal completion, `headless_test` prints the code, and `assemble_verdict` checks only output. Pin marker-complete failed-exit and successful-exit cases; preserve watchdog kill/wait failure handling.
- [ ] Scope `check_patch_count` to `[SpireProfiler] harmony patches applied; patched methods: ` and test unrelated matching text. Keep `>= MIN_PATCHES`: the shim uses `Harmony.GetAllPatchedMethods`, including other mods. Correct [AGENTS.md](AGENTS.md)'s “exact” count wording to [verify.md](docs/verify.md)'s canonical minimum semantics in this change.

Keep this one verdict concern, without a process-runner redesign. Run smoke and headless; report unavailable runtime validation.

### 2. Lock the outer Cargo invocation

- [ ] Add `--locked` to the `run --package xtask --` alias in [.cargo/config.toml](.cargo/config.toml). Cargo can resolve before inner locked gates run. Verify stale-lock refusal and no rewrite in an isolated disposable fixture, then smoke. Never modify the user's lockfile for the probe; this audit did not experimentally reproduce mutation.

### 3. Reconcile ownership and reproducibility policy

Keep documentation concerns separate from behavior fixes.

- [ ] Narrow duplicated claims in [AGENTS.md](AGENTS.md), [lib.rs](profiler-core/src/lib.rs), and [data.rs](profiler-core/src/data.rs): `State` owns live combat/run data. Panel instances/TLS input, history cache/selection, the log sink, [panel_body.rs](profiler-core/src/ui/panel_body.rs)'s owner/child handoff, and GDExtension initialization retain their lifetime owners. [gdext.rs](profiler-core/src/engine/gdext.rs) uses production `EngineClass::name_ptr: AtomicUsize` and `API`/`STRING_DTOR`/`CLASSES` `OnceLock`s; tests also use atomics. Limit no-lock/no-atomic policy to gameplay-state coordination; do not centralize independent state.
- [ ] Specify one active state mutation guard: supplied-state helpers must not reborrow `STATE`; release guards before reentering callbacks/writers. [events.rs](profiler-core/src/data/events.rs) `init` and run/combat lifecycle legitimately borrow sequentially; counting borrows across an entire event is wrong. Preserve the independent log sink's no-`STATE`-reentry contract and behavioral test. Owner details stay in their modules.
- [ ] Narrow the `tmp/` rule to prohibit shipped-profiler dependence on repository scratch while permitting explicitly owned test/xtask scratch. Fixed headless/decompile scratch remains unsafe for concurrent invocations; do not blanket relocate it.
- [ ] Define `SIM_SEED` as replaying events and behavioral assertions under equivalent isolated fixtures, not all persisted bytes. [combat.rs](profiler-core/src/data/events/combat.rs) `combat_started` and [run.rs](profiler-core/src/data/events/run.rs) `take_ended_run` use wall time for combat `started_at` and run `ended_at`; original run-start identity is fixed. Correct [sim.rs](profiler-core/tests/sim.rs)'s “one nondeterminism” header too. Keep reviewed byte snapshots separately; add no clock abstraction to rescue prose. Run fmt-md and smoke for policy edits.

## Bounded test improvements

The source review is settled; these are concrete dispositions, not another open audit.
Preserve [run_panel.rs](profiler-core/src/ui/run_panel.rs) `run_manual_visible_cycles` and `dismiss_run_manual_lands_on_hidden`: distinct hide/dismiss/hidden-input/reopen regressions.
Existing context ABI tests observe null/empty/valid non-UTF-8 attribution; add no duplicate context matrix.
Keep deterministic simulation, the naive block model, allocation tests, persistence snapshots, and failure coverage.

- [ ] Isolate [check_catalog.rs](xtask/src/check_catalog.rs) fixtures in `hook_universe_accepts_non_task_return_types`, `class_files_reject_namespace_drift`, and `class_files_reject_bodyless_declarations` with owned `Shell::create_temp_dir` directories. Their fixed system-temp paths and teardown still let concurrent processes contaminate/delete each other's fixtures; avoid overlapping full xtask or smoke suites until fixed. Preserve the semantic assertions and leave production headless/decompile scratch separate.
- [ ] Replace `u64_from_hash_sign_extends` in [ledger/tests.rs](profiler-core/src/data/ledger/tests.rs) only after strengthening [power.rs](profiler-core/src/data/events/tests/power.rs) `doom_kills_attribute_enemy_hp_to_the_doom_appliers` with `-1` and `i32::MIN` credit cases. Signed shim doom hashes must match unsigned power hashes; an intervening `u32` cast must fail. Existing event coverage uses positive 42/43.
- [ ] Replace `from_u8_round_trips_all_kinds` only with focused stored-record decoding coverage for 3/4/255 (Potion/Osty/unknown to Osty). `From<u8>` differs from `from_c`; [combat_doc.rs](profiler-core/src/data/persistence/combat_doc.rs) `all_zero_card_rows_round_trip_as_identity_only` covers Potion only.
- [ ] Keep handwritten valid/invalid wire decoder cases: const discriminants do not exercise decoding. Drop only the membership-only `-64..=64` source-kind sweep. Preserve modifier attribution coverage; correct its stale `0=Power` comment to `2=Power`/`1=Relic`. In the same bounded decoder concern, fix `SourceKind::from_c` documentation/diagnostic claiming invalid negatives clamp to Power: nearest-clamping yields Card for negatives, Power above 2. Preserve behavior.
- [ ] In [state.rs](profiler-core/src/data/state.rs), remove the duplicate Victory serialization assertion from `outcome_serde_round_trips_lowercase_and_reads_unknowns_as_defeat`; the records schema snapshot pins it. Retain unknown-string-to-Defeat behavior and rename accurately or replace with record-level coverage.
- [ ] Strengthen [abi.rs](profiler-core/src/abi.rs) `null_string_arguments_are_treated_as_empty`, now survival-only with potion calls before combat. Assert metadata and active potion behavior: `potion_used("")` increments `potions_used` while preserving fallback; `potion_context_begin("")` preserves fallback. Malformed UTF-8 tests need valid NUL-terminated storage, never invalid pointers.
- [ ] In [sim.rs](profiler-core/tests/sim.rs), delete fixed JSON absence/type assertions duplicated by writer snapshots and the identity-only fixture; do not relocate them. `check_absent_equals_zero` compares parsed JSON to itself, not independent live expectations. Preserve randomized ledger/model/lifecycle/file-count checks. Remove `check_run_and_store_files`' duplicate read of `runs/1/1.json` and ID substring check, already covered by `check_written_files`; retain run outcome assertions. No giant merger/framework.
- [ ] In [runs.rs](profiler-core/src/data/persistence/runs.rs) `rebuild_run_accumulator_matches_live_merge_field_by_field`, replace `assert_card_stat_eq` and length/zip comparison with whole-vector equality. `CardStat` already derives `Debug`/`PartialEq`/`Eq`; no production change/dependency is needed.
- [ ] Replace [git.rs](xtask/src/git.rs)'s checkout-dependent hash-shape test after its Git fixture asserts the dev abbreviation matches known full HEAD (at least eight characters; Git may lengthen it). Preserve dirty index/worktree, untracked, missing, unborn, and fallback behavior.

Run targeted tests and smoke for each bounded change; preserve each changed test's distinct failure mode.

## Mechanical extraction

- [ ] Move [run_layout.rs](profiler-core/src/ui/run_layout.rs) tests to a sibling module: 949 total lines, about 404 after extraction.
- [ ] Move [tooltip.rs](profiler-core/src/ui/tooltip.rs) tests: 884/about 427.
- [ ] Move [snapshot.rs](profiler-core/src/ui/snapshot.rs) tests: 901/about 503.

These checkpoint measurements explain local production navigation benefits; they are not targets.
Each concern is move-only, preserving names, fixtures, module paths, and snapshots. `run_layout` has four snapshots and test-only `TextAlign`/`panel_common` imports to retain/move.
Move snapshots into the sibling's `snapshots/` directory as content-identical renames (R100).
Drop `runs` extraction: its 123-line production prefix in a 535-line file is already navigable. Drop whole-crate inline-share targets; moving text does not remove code.

## API and ABI

- [ ] Trial `unreachable_pub` with actual visibility reductions, starting with `registration`; an all-public module tree defeats the lint's value alone. Keep integration-test paths through `data::{events,ledger,records,state}`, `ui::{snapshot,ui_model}`, and feature-gated `test_util`. Add no facade/test-support expansion just to silence it. Drop separate `private_interfaces`/`private_bounds` adoption: both already warn and smoke denies warnings.
- [ ] Make scalar/pointer-free exports safe `extern "C"` where caller obligations allow. Pointer-reading exports and GDExtension remain unsafe based on obligations, not whether the outer body contains an unsafe block. Preserve containment and ABI signatures. In the **same change**, update [check_abi.rs](xtask/src/check_abi.rs) `scan_rust_exports`, which matches exact `unsafe extern "C" fn` text, plus a mixed safe/unsafe fixture retaining missing/signature failure cases. Run smoke, check-abi, and headless; state unavailable runtime limits.

## xtask simplification and optional CI

Proceed in this order, one bounded concern each:

- [ ] In [main.rs](xtask/src/main.rs) `ensure_cargo_tool` and main/[cross.rs](xtask/src/cross.rs) callers, pass package/version arguments directly; remove formatted `install_spec`, `split_whitespace`, `install_tool_argv`, and its parser-only test. Preserve stable override, `--locked`, exact version, and postinstall checks.
- [ ] In [zig.rs](xtask/src/zig.rs), make `resolve_zig` delegate missing/wrong-version handling to `ensure_bootstrap_in` once. Preserve checksum/version/cache-refresh behavior and `CARGO_ZIGBUILD_ZIG_PATH`; keep separate caches/installers without a shared bootstrap framework.
- [ ] In [build.rs](xtask/src/build.rs), test explicitly passing generated `SpireProfiler.csproj` to `dotnet build` with the pinned SDK before removing `refresh_gen_dir`'s ambiguity-repair scan. This is a source-backed hypothesis, not validated tool behavior. Preserve `write_if_changed`, `DOTNET_ROOT`, and build options.

Preserve release bundle/stamp/archive-byte/checksum/failure tests; add no transactional archive-set machinery.
Long-option preference is already in AGENTS and release commands.
Optional CI, only if useful: one Linux smoke job, no matrix, bootstrap-all, game prerequisites, retries, or publishing. Its absence never blocks branch completion.

## Implementation, review, and human handoff

Every bounded item uses an implementer/reviewer subagent pair. The orchestrator owns specification, integration, gates, and checkpoint; agents never create commits or PRs.

1. Give the implementer motivation, standard, hard scope, gates, and deliverable.
2. Start the reviewer **after implementation finishes**, with fresh context, original material, final diff, and standard, without implementer reasoning.
3. Review lost behavior/contracts and remaining bloat, plus links, em dashes, and diff scope. Verdicts: SHIP, SHIP WITH FIXES, REJECT. Return required fixes; substantial fixes need re-review, REJECT needs a fresh implementer. The orchestrator may fix nonblocking nits.
4. Run gates, replace the checkpoint, present the result and suggested commit title, then stop for the human commit before the next item.

Keep boundary/complex changes near 300 changed lines and ordinary changes near 500; move-only extraction and authorized broad compaction may exceed this guidance.
Keep mechanical moves, behavior changes, documentation, and punctuation cleanup separate. Lower [em-dash pins](xtask/src/check_emdash.rs) with count reductions; ceilings only move down, and cleanup below a pin is allowed.

Run `cargo xtask smoke` after source/docs changes; `cargo xtask check-abi` after ABI/shim changes; headless after engine/registration/shim behavior changes or ABI refactors. Follow [verify.md](docs/verify.md) and xtask for semantics; report unavailable validation.
Review containment/pointer lifetimes, game drift, persistence schema/atomicity/snapshots, independent test value, canonical ownership/context size, and release/platform impact. Never set `panic = "abort"`; containment needs unwinding.
Report motivation, net behavior/safety impact, distinct test failures, actual commands, validation limits, and context added/deleted/moved. Omit abandoned attempts and repeated gate histories.

Final cleanup: pass relevant gates, state platform/runtime limits, remove temporary adoption policy and the AGENTS pointer, and delete this file in the final human-reviewed change. The human rebases/merges this work branch into linear `main`, then deletes the work branch.

## Comparison rationale and retained decisions

Codex comparison commit `6750f5bd13` supplies evidence, not recovered author intent; relevant inspected rules remain in `2cbbf0c9b542a36a1c3284b5e804917635b6f666`.
Codex's `AGENTS.md` favors whole-object assertions and behavioral tests, and forbids moving existing inline tests merely to follow its convention. Its runtime context caps imply no documentation quota here.
Its `codex-rs/core-api/src/lib.rs` applies visibility lints to a public facade, not its workspace. The owned temporary fixtures in `codex-rs/core/tests/common/test_codex_exec.rs` support adopting fresh reservation/ownership without a builder framework.
Separate `Session` (`codex-rs/core/src/session/session.rs`) and `ChatWidget` lifetimes support local ownership, not copying async/locks. Upstream has seeded tooltip tests and daily-seeded textarea fuzz; it is not uniformly deterministic.
Codex's `justfile` supports Nextest `--no-fail-fast`; its CI/retries/shards solve larger-suite needs. Density, em-dash, locking, printing, and this workflow are local choices. Keep justified `expect` rather than copying `expect_used`.

- Keep profiler-core/xtask; do not copy Codex's crate graph, async/locks/channels, Bazel, protocol generation, cargo-deny/cargo-shear, or branch/PR automation. Add no dependency for imitation; consider `pretty_assertions` only for recurring whole-object diff problems.
- Keep explicit Cargo locking; smoke must work on dirty developer trees. Persistence stays disposable WIP data without migrations; schema/matching owners define exact run identity.
- Keep Rust's 15% comment gate; no shim density ceiling (its measured 30.6% mostly documents marshaling, engine traps, and pixel math). No broad Unicode ban: preserve runtime typography and the renderer glyph allowlist.
- Delete prose already taught by code, `--help`, or general knowledge; do not relocate it. No hard byte ceilings. Soft module-doc guides remain about 50 lines, 90 for canonical schema/state/safety owners.
- Use relative links for tracked files and backticks for commands, symbols, and untracked paths. Canonical facts have one owner:

| Fact | Owner |
|---|---|
| Agent policy; crate/attribution contracts | [AGENTS.md](AGENTS.md); [lib.rs](profiler-core/src/lib.rs); [data.rs](profiler-core/src/data.rs) |
| Player slots; JSON/write protocol | [state.rs](profiler-core/src/data/state.rs); [persistence.rs](profiler-core/src/data/persistence.rs) |
| Unsafe engine contracts; empirical findings | [gdext.rs](profiler-core/src/engine/gdext.rs); [gdextension.md](docs/gdextension.md) |
| Build environment; gates; game/update drift | [build.md](docs/build.md); [verify.md](docs/verify.md) and xtask; [game.md](docs/game.md) |
| ABI; Harmony catalog | [abi.rs](profiler-core/src/abi.rs) and [check_abi.rs](xtask/src/check_abi.rs); [catalog.rs](xtask/src/catalog.rs) and generated shim |

## Maintain this handoff autonomously

Compaction needs no separate permission. Replace/prune checkpoints, completed narrative, repeated rules, and stale tasks; Git preserves history, so create no archive or relocated audit log.
Preserve unresolved source-anchored findings/reproductions, constraints, active decisions, remaining work, necessary measurements, validation limits, pair/human-commit workflow, and this policy. Recheck source for stale tasks; keep review items only where evidence is unresolved.
Compaction authorizes no automatic test deletion, feature change, or scope expansion. Routine checkpoint updates must not reproduce the whole audit.
Use 200-300 lines as a soft growth guide, retaining critical context and readable prose over numeric targets. Ordinary updates ride with the task; broad compaction is a separate doc-only change under the same workflow.
Run `cargo xtask fmt-md` after doc edits; [md.rs](xtask/src/md.rs) pins the formatted set and excludes this temporary file. Never hand-reflow pinned docs.
