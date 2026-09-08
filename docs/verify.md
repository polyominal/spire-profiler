# Verifying the mod

## The gate set

- `cargo xtask smoke`: `cargo fmt --all -- --check`, `fmt-md --check`,
  `check-citations`, `check-emdash`, `check-abi` (shim `GetExport` bindings
  against the Rust exports), `cargo clippy --workspace --all-targets
  --all-features --locked -- --deny warnings`, `check-docs` (warning-free `cargo
  doc --document-private-items` and the comment-density budget), `cargo nextest
  run --workspace --locked --no-fail-fast`.
- `cargo xtask headless-test` PASS: the game exits successfully, its
  `[SpireProfiler] harmony patches applied; patched methods: N` marker reports
  at least `MIN_PATCHES` patched Harmony methods (derived from the attribution
  catalog plus the fixed class-level and orb groups; other mods can increase the
  count), no unexpected `[SpireProfiler]` ERROR lines (skipped dynamic
  catalog/orb patches included; panel-attach failures are deliberate failures,
  never allowlisted), and the combat panel's parent, rows-child, and
  overlay-child `draw` virtuals fire under the headless dummy renderer (draw
  dispatch is covered, visual output is not).
- Real-play validation is manual: the pipeline cannot play the game.
- A game update adds one machine-local gate: `cargo xtask check-catalog` reads
  the decompiled tree (`tmp/sts2-decompiled`), so it stays out of smoke; what it
  verifies lives in [game.md](game.md).

## StS2 headless testing

- Headless boot requires `--headless --force-steam off` (without Steam running,
  the game stalls at the Steam-error popup otherwise). `--quit-after N` exits
  after N frames (~10s to main menu).

- With `--force-steam off`, the game reads settings from `<user data
  dir>/default/1/settings.save` (NOT the steam/ account-scoped one); on macOS
  that is `~/Library/Application Support/SlayTheSpire2/default/1/settings.save`.
  Mod loading requires `mod_settings.mods_enabled: true` there (the consent
  model is described in [game.md](game.md)); the one-time enable (macOS):
  
  ```sh
  python3 -c "import json,os; p=os.path.expanduser('~/Library/Application Support/SlayTheSpire2/default/1/settings.save'); d=json.load(open(p)); d.setdefault('mod_settings',{})['mods_enabled']=True; json.dump(d,open(p,'w'))"
  ```

- The first `--force-steam off` boot creates that settings file with mods
  disabled, so the first headless run FAILs on missing markers: set the flag
  once and re-run. The enable does not cover normal Steam play (the steam/
  account-scoped settings file is a separate consent).

- Marker placement differs by origin, and `headless-test` greps both sources for
  its verdict: the C\# side's `Log.Info` markers land in `godot*.log`, while the
  core's stderr `INFO` markers appear only in game process output, never in the
  log files. Do not look for core markers in `godot*.log`.

- lldb cannot attach to the game (hardened runtime); debug via the core's
  fail-safe stderr diagnostics (which `headless-test` captures) and the godot
  log files (`<user data dir>/logs/`).

- `SPIRE_PROFILER_DATA_DIR` points the shim's data dir at `tmp/headless-data`
  (wiped per boot), so self-test records never mix into real play data.

- `headless-test` boots the game under a watchdog (the timeout constant lives in
  [headless.rs](../xtask/src/headless.rs)); a hung boot is killed and reported
  instead of hanging the terminal. A first boot after an install recompiles game
  shaders and can take 30-60s.

- Engine exit noise to ignore in headless logs: RID leaks of dummy renderer
  types, "ObjectDB instances leaked at exit", "Parameter t is null".
