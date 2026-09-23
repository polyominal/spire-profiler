# Verifying the mod

## The gate set

- `cargo xtask smoke`: `fmt --check` (Rust, handwritten C\# including fixtures,
  and Markdown; requires the pinned SDK but no game), `check-citations`,
  `check-emdash`, `check-abi` (`GetExport` bindings across the production shim
  sources against the Rust exports), `cargo clippy --workspace --all-targets
  --all-features --locked -- --deny warnings`, `check-docs` (warning-free `cargo
  doc --document-private-items` and the comment-density budget), `cargo nextest
  run --workspace --locked --no-fail-fast`.
- `cargo xtask managed-test`: build the host Rust reducer, then compile the
  shared production managed sources and deterministic fixtures against the
  installed, version-checked game and Harmony assemblies. Compiler and
  recommended .NET 9 analyzer warnings fail the build. Each invocation retains
  an isolated project and source, assembly, and native-library hashes under
  `tmp/managed-tests/`. The fixtures check capture and async scope restoration,
  exact patch bridges, chart projection, parser boundaries, atomic persistence,
  and native session/replay behavior. They do not launch Godot.
- `cargo xtask headless-test` first runs `managed-test`, then requires
  successful game exit, this mod's `OWN PATCHES` minimum, and the `CAPTURE
  VERIFIED` marker with exact producer and damage/temporal/modifier bridge
  inventories. The managed session fixture must prove native accounting,
  deterministic observation replay, and persisted run history. The managed panel
  fixture must attach, draw, scroll, filter, hide, free, and recreate both panel
  variants. Unexpected `[SpireProfiler]` ERROR lines fail the gate. Harmony
  verification checks owner `dev.spireprofiler` and exact patch methods; other
  mods cannot satisfy it. Headless drawing proves dispatch and lifecycle, not
  visual output.
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

- `headless-test` combines the fresh `godot*.log` with captured process output.
  Managed self-test and panel markers use the game's logger; contained native
  panic diagnostics use stderr. Both streams matter when investigating a failed
  gate.

- lldb cannot attach to the hardened game runtime. Inspect `<user data
  dir>/logs/`, captured process output, and the record's coverage reasons. For
  reproducible attribution failures, enable observation recording as described
  in [interop.md](interop.md).

- `SPIRE_PROFILER_DATA_DIR` points the shim's data dir at `tmp/headless-data`
  (wiped per boot), so self-test records never mix into real play data.

- `headless-test` boots the game under a watchdog (the timeout constant lives in
  [headless.rs](../xtask/src/headless.rs)); a hung boot is killed and reported
  instead of hanging the terminal. A first boot after an install recompiles game
  shaders and can take 30-60s.

- Engine exit noise to ignore in headless logs: RID leaks of dummy renderer
  types, "ObjectDB instances leaked at exit", "Parameter t is null".
