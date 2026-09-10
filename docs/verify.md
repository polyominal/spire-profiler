# Verifying the mod

## The gate set

- `cargo xtask smoke`: `cargo fmt --all -- --check`, `fmt-md --check`,
  `check-citations`, `check-emdash`, `check-abi` (`GetExport` bindings across
  the production shim sources against the Rust exports), `cargo clippy
  --workspace --all-targets --all-features --locked -- --deny warnings`,
  `check-docs` (warning-free `cargo doc --document-private-items` and the
  comment-density budget), `cargo nextest run --workspace --locked
  --no-fail-fast`.
- `cargo xtask managed-test`: compile the shared production shim sources and
  deterministic managed fixtures with the pinned .NET SDK against the installed,
  version-checked game and Harmony assemblies. Each invocation retains an
  isolated project and source/assembly hashes under `tmp/managed-tests/`. The
  fixtures check capture, async scope restoration, and exact patch bridges; they
  do not play the game.
- `cargo xtask headless-test` first runs `managed-test`, then requires a
  successful game exit, this mod's `OWN PATCHES` minimum and `CAPTURE VERIFIED`
  marker with the exact producer/bridge inventory, no unexpected
  `[SpireProfiler]` ERROR lines, and the combat panel's parent, rows-child, and
  overlay-child `draw` virtuals under the headless dummy renderer. Capture
  verification checks Harmony's exact patch methods and owner
  `dev.spireprofiler`; other mods cannot supply that coverage. Patch or
  panel-attach failures are never allowlisted. Draw dispatch is covered, visual
  output is not.
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
