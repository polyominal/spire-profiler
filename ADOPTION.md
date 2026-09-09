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

Uncommitted checkpoint: wire-decoder test and diagnostic cleanup, based on clean HEAD `f02ff872508d78dff10b18e7b92d9129f232742e` on 2026-09-09. Initial Git status was clean; the stored-record source-kind checkpoint is committed there.
Implementation, independent review (SHIP), and required checks are complete. Stop for the human commit before the next item: remove the duplicate Victory serialization assertion in `state.rs`, retaining unknown-string-to-Defeat coverage with an accurate test name.

Completed work:

- Smoke covers ABI, private rustdoc with warnings denied, comment density, and documentation hygiene. The outer alias in [.cargo/config.toml](.cargo/config.toml) and inner Cargo gate/build calls use `--locked`. Nextest uses `--no-fail-fast`; production printing/unwrap lints and the em-dash ratchet are installed. Canonical documentation ownership, trims, soft module-doc budgets, and the shared line scanner landed.
- Corrective fixes preserve context-scope balance/attribution, exact run identity across suspension, checked ID exhaustion, write-dependent persistence success/cache invalidation, and finite scroll input with hide/dismiss/reopen expiry. Private rustdoc fixed four broken links/rendering cases. Simulation seed errors fail loudly, failures identify seed/scenario, and model comparisons use deterministic order.
- Release requires clean committed input, smoke, exact bundle/stamp validation, explicit archive members, and checksums after successful replacements. Tests pin archive entries, extracted bytes, checksum rows, and failures. Replacement is atomic per file, not across the archive set; target input staging trees are gone.
- [chart_layout.rs](profiler-core/src/ui/chart_layout.rs) tests moved into [tests.rs](profiler-core/src/ui/chart_layout/tests.rs): production file 2,037 to 946 lines, 31 tests preserved, two snapshots renamed content-identically with unchanged module paths.
- [test_util.rs](profiler-core/src/test_util.rs) reserves fresh empty directories with atomic `create_dir`, retries only occupied candidates, and reports other setup errors with paths. `wiped_dir` and its destructive setup are removed; profiler callers retain isolated fixtures under `tmp/unique/`. [discover.rs](xtask/src/discover.rs) `FakeTree` owns `xshell::TempDir`. Tests cover occupied directory/file sentinels, same-label contention, parent/candidate errors, and independent teardown. A four-process probe also verified separate same-label directories and sentinel retention after all reservations and process exits.
- [headless.rs](xtask/src/headless.rs) carries the child's full `ExitStatus` into the verdict and rejects unsuccessful termination alongside existing marker/error gates. Patch counts require the full profiler marker, retaining maximum-count deduplication and the minimum threshold. [AGENTS.md](AGENTS.md) and [verify.md](docs/verify.md) agree with that contract. Behavioral tests cover unsuccessful exit/signal statuses on supported Unix hosts, missing markers/errors, unrelated count text, thresholds, duplicates, malformed counts, and overflow.
- Ownership policy points to [state.rs](profiler-core/src/data/state.rs) for live combat/run data and borrowing, and to the [log sink](profiler-core/src/data/persistence/log.rs) for no-`STATE`-reentry. Independent owners retain their lifetimes; gameplay-state coordination alone excludes locks/atomics. Owned test/xtask scratch is permitted, with fixed headless/decompile paths requiring serialized invocations. [sim.rs](profiler-core/tests/sim.rs) distinguishes seeded behavioral replay from wall-clock timestamps; [AGENTS.md](AGENTS.md) retains the separate reviewed byte-snapshot policy. Duplicated ownership prose was deleted.
- The three filesystem-backed tests in [check_catalog.rs](xtask/src/check_catalog.rs) hold owned `xshell::TempDir` fixtures through their assertions. Fixed shared paths and manual teardown are removed; fixture bytes, test names, hook discovery, namespace-drift rejection, and bodyless-declaration rejection are preserved. Headless/decompile scratch is unchanged.
- Creature hashes use `int`/`i32` across the [shim](shim/shim.cs.template), [ABI](profiler-core/src/abi.rs), core events/attribution tables, and simulation inputs. Power apply/decrease, damage receiver/dealer, and weak-mitigation parameters change together on both ABI sides; doom capture keeps its signed type. Widening casts and `u64_from_hash` are deleted. The pinned v0.111.0 `Creature`, power-owner, Doom, and damage-hook audit supports retaining `GetHashCode` identity without substituting `CombatId`. Zero/null handling, containment, fallback, unrelated IDs/counters/RNG state, and the saved-record schema are preserved.
- [Power-event tests](profiler-core/src/data/events/tests/power.rs) preserve doom applier/fallback credits for 42, -1, and `i32::MIN`, replacing the cast-only ledger test. Negative poison keys make decremented duration observable in subsequent damage credit; weak keys exercise head replacement and unknown-target exclusion; strength keys retain LIFO restoration/mitigation checks. [Combat tests](profiler-core/src/data/events/tests/combat.rs) cover equal and distinct negative hashes, dealer-zero card attribution, and both-zero/no-source damage. [sim.rs](profiler-core/tests/sim.rs) narrows creature inputs to `i32` while preserving seeded RNG state and draw order.
- [zig.rs](xtask/src/zig.rs) `resolve_zig` delegates provisioning/version handling once to `ensure_bootstrap_in`, then selects the local binary and logs it. One line replaces 15 lines of duplicate branching, queries, and obsolete comments. Installer internals, pin/checksum/extracted-version checks, cache refresh, and `CARGO_ZIGBUILD_ZIG_PATH` integration are unchanged. No helpers, dependencies, or permanent tests were added.
- [Record tests](profiler-core/src/data/records/tests.rs) replace the ledger's `from_u8_round_trips_all_kinds` with literal JSON parsed through `parse_combat_doc`: 3 reads as Potion, 4 as Osty, and 255 falls back to Osty. This catches incorrect stored-kind mappings, accidental context-kind clamping, and unknown-kind rejection or wrong fallback. The identity-only Potion fixture and surrounding tests remain intact; production behavior, ABI, schema, and snapshots are unchanged.
- [State tests](profiler-core/src/data/state.rs) drop only the membership-only `-64..=64` source-kind sweep, retaining explicit valid codes, negative/Card and upper/Power clamps, and modifier decoder cases. [SourceKind::from_c](profiler-core/src/source_kind.rs) documents both clamp directions and reports the actual clamped code through the unchanged log-once path. The [power-event modifier test](profiler-core/src/data/events/tests/power.rs) comment now states `2 = Power, 1 = Relic`; its persisted kind/credit assertions remain intact. Gameplay behavior, ABI, schema, and snapshots are unchanged. Rust context is 5 lines added, 12 deleted, with no moves, helpers, or new tests.

Verification:

- `cargo nextest run --package profiler_core --locked --no-fail-fast --filter-expr 'test(data::state::tests::) | test(data::events::tests::power::modifier_kind_codes_map_power_and_relic_and_clamp_unknowns) | test(data::records::tests::parse_combat_doc_preserves_potion_and_osty_and_defaults_unknown_kinds_to_osty)'` passed all 7 targeted tests. They retain failures for context mapping/clamps, modifier fallback and persisted credit, stored-kind decoding/fallback, run-outcome decoding/defaults, and context-exhaustion recovery.
- `cargo nextest run --package profiler_core --locked --no-fail-fast --no-capture --filter-expr 'test(data::state::tests::from_c_clamps_every_input_to_a_catalogued_kind)'` passed and emitted exactly one diagnostic: `invalid context kind -2147483648; clamping to 0`. Upper-clamp assertions passed; that direction's diagnostic was not separately captured because the test logs its first invalid input once.
- `cargo xtask smoke` passed 363/363 tests, workspace Clippy, private rustdoc, formatting, citations, 38 ABI bindings, and em-dash checks. Rust comment density is 10.4% (3,071 / 29,462); em-dash pins remain 39 files / 130 dashes. `cargo fmt --all -- --check`, `cargo xtask fmt-md`, and `git diff --check` passed. This test/comment/diagnostic change requires no new build or headless run; prior runtime/platform limits below still apply.
- The committed Zig resolver change passed `zig-sdk/zig version` at the pinned 0.16.0 and `ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-cache" cargo xtask build`, exercising the simplified resolver with the cached SDK and building all four native targets and the C# shim; C# had zero warnings/errors. The existing macOS SDK-discovery and deprecated-linker-setting warnings were nonfatal. Missing/wrong-version refresh and download-failure paths were reviewed in the unchanged installer, not executed against the real SDK.
- The committed catalog-fixture change passed all 10 catalog tests and four concurrent processes running the three affected tests against one isolated shared `TMPDIR`: all 12 executions passed. Former fixed-path sentinels and occupied xshell directory/file candidates retained their bytes; all newly owned fixture directories were removed.
- The committed outer-locking change passed an isolated offline stale-lock probe: `cargo xtask probe` refused before compilation/execution and preserved lockfile bytes and metadata. The prior unlocked alias updated the same stale lock and ran; the locked alias ran with the repaired lock, forwarded arguments, and preserved it. The repository lockfile remained unchanged.
- Headless verification remains at the creature-hash implementation committed in `9471d34`: `ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-cache" cargo xtask headless-test` installed the matched bundle and passed against Windows game v0.111.0 through WSL: game exit status 0 after 39.7 seconds, 194 patches (minimum 194), all draw/persistence markers, no unexpected profiler errors, command exit 0. WSL interop and installation into the game directory required sandbox escalation.
- Clean release verification remains at `f81aaed39b9f94f028b343e398543e52b39f5943`: `ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-cache" cargo xtask release` passed smoke, all four native targets, the C# shim, and five ZIPs under `dist/`. `sha256sum --check SHA256SUMS` passed there; separate archive inspection verified exact members, matching bundle content hashes, and stamp `v0.111.0-f81aaed3`. Those archives precede the fixture, headless-verdict, and creature-hash changes.
- GNU tar and BSD tar previously passed gzip/xz long-option extraction probes; dirty release rejection remains covered by smoke.
- Live headless covers successful Windows termination, mod loading, and combat-panel boot/draw. Negative-hash attribution is verified by Rust behavioral tests, not live gameplay through the changed C# delegates. Interactive scrolling, the lazy history panel, native Linux/macOS game runtime, and Windows/macOS filesystem failure execution remain unverified. Failed-exit/signal verdicts use constructed OS statuses in Linux/WSL tests.

Follow [build.md](docs/build.md), [verify.md](docs/verify.md), and [game.md](docs/game.md). Record missing tools or mismatch with [game_version.rs](xtask/src/game_version.rs); never silently bump `PIN` or weaken gates.

## Bounded test improvements

The source review is settled; these are concrete dispositions, not another open audit.
Preserve [run_panel.rs](profiler-core/src/ui/run_panel.rs) `run_manual_visible_cycles` and `dismiss_run_manual_lands_on_hidden`: distinct hide/dismiss/hidden-input/reopen regressions.
Existing context ABI tests observe null/empty/valid non-UTF-8 attribution; add no duplicate context matrix.
Keep deterministic simulation, the naive block model, allocation tests, persistence snapshots, and failure coverage.

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
| Live-state ownership/borrowing and player slots; JSON/write protocol | [state.rs](profiler-core/src/data/state.rs); [persistence.rs](profiler-core/src/data/persistence.rs) |
| Unsafe engine contracts; empirical findings | [gdext.rs](profiler-core/src/engine/gdext.rs); [gdextension.md](docs/gdextension.md) |
| Build environment; gates; game/update drift | [build.md](docs/build.md); [verify.md](docs/verify.md) and xtask; [game.md](docs/game.md) |
| ABI; Harmony catalog | [abi.rs](profiler-core/src/abi.rs) and [check_abi.rs](xtask/src/check_abi.rs); [catalog.rs](xtask/src/catalog.rs) and generated shim |

## Maintain this handoff autonomously

Compaction needs no separate permission. Replace/prune checkpoints, completed narrative, repeated rules, and stale tasks; Git preserves history, so create no archive or relocated audit log.
Preserve unresolved source-anchored findings/reproductions, constraints, active decisions, remaining work, necessary measurements, validation limits, pair/human-commit workflow, and this policy. Recheck source for stale tasks; keep review items only where evidence is unresolved.
Compaction authorizes no automatic test deletion, feature change, or scope expansion. Routine checkpoint updates must not reproduce the whole audit.
Use 200-300 lines as a soft growth guide, retaining critical context and readable prose over numeric targets. Ordinary updates ride with the task; broad compaction is a separate doc-only change under the same workflow.
Run `cargo xtask fmt-md` after doc edits; [md.rs](xtask/src/md.rs) pins the formatted set and excludes this temporary file. Never hand-reflow pinned docs.
