# Verifying the mod

The gate set every commit must pass, and the headless game boot that exercises
the real game. Build and toolchain setup live in `build.md`; the game's on-disk
layout and discovery live in `game.md`.

## The gate set, stated once

- `cargo xtask smoke` green — the pre-commit gate: `cargo fmt --all --
  --check`, the markdown wrap check (`fmt-md --check`), the citation check
  (`check-citations`), `cargo clippy --workspace --all-targets --all-features --
  --deny warnings`, `cargo nextest run --workspace`. Each step propagates its
  exit code, so the gate cannot silently pass.
- `cargo xtask check-abi` green — the shim's `GetExport` bindings verified
  against the Rust exports. The binding count is the command's own output;
  nothing else pins it, so no count is stated here.
- `cargo xtask headless-test` PASS — at least the shim's expected number of
  patched Harmony methods (`MIN_PATCHES`, derived from the attribution catalog
  plus the fixed class-level and orb groups), no unexpected `[SpireProfiler]`
  ERROR lines (including skipped dynamic catalog/orb patches), and the combat
  panel's parent, rows-child, and overlay-child `draw` virtuals fire under the
  headless dummy renderer (draw dispatch is covered, visual output is not).
- Real-play validation is manual: the pipeline cannot play the game.

A game update adds one machine-local gate: `cargo xtask check-catalog` reads the
decompiled tree (`tmp/sts2-decompiled`), so it stays out of smoke; what it
verifies lives in `game.md`.

## StS2 headless testing

- Headless boot requires `--headless --force-steam off` (without Steam running,
  the game stalls at the Steam-error popup otherwise). `--quit-after N` exits
  after N frames (~10s to main menu).

- With `--force-steam off`, the game reads settings from `<user data
  dir>/default/1/settings.save` (NOT the steam/ account-scoped one); on macOS
  that is `~/Library/Application Support/SlayTheSpire2/default/1/settings.save`.
  Mod loading requires `mod_settings.mods_enabled: true` there (the consent
  model is described in `game.md`); the one-time enable (macOS):
  
  ```sh
  python3 -c "import json,os; p=os.path.expanduser('~/Library/Application Support/SlayTheSpire2/default/1/settings.save'); d=json.load(open(p)); d.setdefault('mod_settings',{})['mods_enabled']=True; json.dump(d,open(p,'w'))"
  ```

- The first `--force-steam off` boot creates that settings file with mods
  disabled, so the first headless run FAILs on missing markers — set the flag
  once and re-run. The enable does not cover normal Steam play (the steam/
  account-scoped settings file is a separate consent).

- lldb cannot attach to the game (hardened runtime); debug via the core's
  fail-safe stderr diagnostics (which `headless-test` captures) and the godot
  log files (`<user data dir>/logs/`).

- Marker placement differs by origin, and `headless-test` greps both sources for
  its verdict: the C\# side's `Log.Info` markers (harmony patch count, `native
  core loaded`, `GDExtension load result`, `profiler panel attached`) land in
  `godot*.log`, while the core's stderr `INFO` markers (`core initialized`,
  `combat N summary written`, `run N recorded`, `chart self-test (...):`, `panel
  class registered`) only appear in game process output — never in the log
  files. Do not look for core markers in `godot*.log`.

- The shim's data dir is pointed at `tmp/headless-data` (wiped per boot) via
  `SPIRE_PROFILER_DATA_DIR`, so self-test records never mix into real play data.

- Any `[SpireProfiler]`-tagged ERROR line fails the headless verdict; the
  panel-attach failures (`GDExtension load result: Failed`, `panel attach
  failed`, `panel instantiation failed`) are deliberate failures, not
  allowlisted.

- `headless-test` boots the game under a watchdog (the timeout constant lives in
  `xtask/src/headless.rs`; a first boot after an install recompiles game shaders
  and can take 30-60s); a hung boot is killed and reported instead of hanging
  the terminal.

- Engine exit noise to ignore in headless logs: RID leaks of dummy renderer
  types, "ObjectDB instances leaked at exit", "Parameter t is null".

- Useful for source spelunking: godot 4.5.1 + gdextension-api sparse clones in
  `tmp/` (gitignored), and a downloaded `extension_api_4.5.1.json`.
