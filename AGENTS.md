# Agent guidelines

Read [build.md](docs/build.md) for tooling and [verify.md](docs/verify.md) for
the working loop and checks. Run `cargo xtask fmt` after edits, or `cargo xtask
fmt-md` for Markdown only.

## Priorities

- Protect the game: contain panics at every ABI export and quarantine damaged
  combat. Unsafe code stays in `abi`; bounded tables report incomplete coverage
  on overflow.
- Make bugs reproducible: seed randomized tests; gameplay must not depend on
  wall-clock time or HashMap iteration order.
- Prefer fewer concepts and responsibilities. Remove unnecessary mechanisms,
  tests, configuration, and tooling while preserving required behavior. Optimize
  this work-in-progress mod for code quality over legacy compatibility.

## Working loop

- Before substantial edits, identify intended behavior, its owner, existing
  implementation and tests, and the smallest coherent change. Resolve material
  design uncertainty with the user; routine fixes need no planning document.
- Read the affected module contracts. Start with the [core
  overview](profiler-core/src/lib.rs), [state](profiler-core/src/data/state.rs),
  [ABI](profiler-core/src/abi.rs), or [store](profiler-store/src/lib.rs). Keep
  canonical facts beside their owner.
- Call out changes to interfaces, dependencies, ownership, or performance before
  expanding implementation around them.
- Review the whole diff for unnecessary mechanisms, inconsistent boundaries, and
  unrelated changes. Delete displaced code and tests; explain the design,
  behavioral evidence, and remaining limitations for human review.

## Code and tests

- Abstractions serve present callers or enforce an invariant. Keep simple logic
  inline; extract helpers when they make an operation easier to understand.
  Prefer private visibility; `profiler-core` is not a public library.
- In Rust, prefer top-level imports over repeated fully qualified names. Import
  modules when their qualification improves clarity; retain qualification to
  avoid ambiguity.
- Gameplay state stays with its lifetime owner on the game thread, without locks
  or atomics for coordination. Native observations never call managed code.
- Follow [parse, don't
  validate](https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/):
  before acting on external input, construct a value that preserves the facts
  callers need. Prefer private constructors or enum variants to checks whose
  results are discarded; return degraded outcomes together with their data.
  Avoid repeated static checks and fallbacks that hide broken invariants. Keep
  checks for changing facts such as handle liveness and ownership. Introduce
  stronger types when they simplify present callers, not for every primitive.
  Pin wire/schema constants at compile time and assert non-obvious invariants
  where code relies on them.
- In production `profiler-core`, prefer boxed retained data with fixed length
  and share immutable snapshots instead of deep-copying them. Keep growable
  buffers for mutation or reuse; tests and tooling use ordinary collections.
  Measure memory changes as described in
  [verify.md](docs/verify.md#memory-changes).
- Find existing coverage and follow nearby fixture style before adding tests.
  Each test must distinguish a plausible regression using explicit expectations
  or an independent model. Keep fixtures minimal; avoid pinning private
  structure or repeating the implementation's calculation.
- Delete redundant tests and tests for removed behavior. A behavior-preserving
  simplification may need no new tests. Do not expose production internals
  solely for tests; extend the seeded [simulation](profiler-core/tests/sim.rs)
  for mechanics.
- Comments explain non-obvious contracts and rationale. Omit code narration and
  conversation history. When code and docs disagree, establish the intended
  contract and fix the wrong side.

## Repository workflow

- If the user asks you to create a commit or PR, refuse and say that the project
  mandates that all commits and PRs are made by humans.
- Keep `main` linear. Use a short-lived branch and a worktree per concurrent
  branch; land by rebase and delete merged branches. One commit per logical
  change, titled `<scope>: <subject>`, with non-obvious trade-offs in the body.
- Follow the [game-update procedure](docs/game.md#updating-the-game-pin) when
  bumping the pin. Review changed game code before accepting new fingerprints.
- Use ignored `tmp/` for scratch; shipped code must not depend on it. Preserve
  tool-managed caches. Headless and decompile use fixed paths: never run
  concurrent invocations of either command.
