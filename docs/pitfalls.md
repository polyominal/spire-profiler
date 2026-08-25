# Pitfalls & environment notes

Durable lessons from debugging sessions, promoted out of SESSION.md (the
gitignored, session-scoped scratchpad — record new *problems* there and
promote them here once confirmed). Read this before touching the build,
GDExtension interop, or shim patches. Each item cost real debugging time.

## Build and verify

Prerequisites: a Steam install of Slay the Spire 2 and the pinned Rust nightly
(`rust-toolchain.toml` auto-fetches it). No pre-installed .NET SDK, zig, or
cross-target stds are needed — the build auto-installs the matrix's rust stds
via rustup and bootstraps the toolchains (see "Hermetic .NET SDK bootstrap" and
"Cross-compilation"; `install-tool` provisions all of them eagerly). Windows
hosts cannot run `build` (the hermetic .NET bootstrap is a Unix script).

```sh
cargo xtask install-tool   # optional: provision host tools + bootstraps up front
cargo xtask build          # cross-platform mod bundle (runs check-abi inside)
cargo xtask release        # build + package the mod as release zips under dist/
cargo xtask install-mod    # copy the bundle into the game's mods directory
cargo xtask smoke          # commit gate: fmt, clippy, citations, tests
cargo xtask check-abi      # standalone shim<->Rust ABI check (38 bindings)
cargo xtask check-citations # fail on file:line citations in comments/docs
cargo xtask headless-test  # install, then boot the game headless (~30s)
cargo xtask decompile      # recover the game's Godot source (see below)
```

The gate set, stated once: `smoke` green; `check-abi` green (38 bindings);
`headless-test` PASS — patch count ≥ 199 (the static count is 203), no
unexpected `[SpireProfiler]` ERROR lines, and the combat panel's `draw` virtual
fires under the headless dummy renderer (draw dispatch is covered, visual output
is not). Real-play validation is pending for the user — the pipeline cannot
play the game.

Game discovery: per-OS Steam default path, or `STS2_GAME_DIR` (macOS: the
directory containing `SlayTheSpire2.app`; Windows/Linux: the directory
containing the executable); `STS2_USER_DATA_DIR` forces the log dir.

`cargo xtask release` packages the bundle as five zips under `dist/`
(gitignored): one `-universal` archive carrying every platform's library, plus
one per target carrying only its own; a `SHA256SUMS` file sits next to the zips.
Every archive's root is the `spire-profiler/` folder, so extracting into the
mods directory is the whole install.

## Rust toolchain (pinned nightly)

- The toolchain is pinned in `rust-toolchain.toml` (`nightly-2026-08-12`).
  rustup selects it automatically inside the workspace, so no toolchain install
  step is needed.
- `install-tool` builds the host tools (nextest, insta, zigbuild) with `cargo
  +stable install ... --locked`, never the pinned nightly: the rustup proxy
  exports `RUSTUP_TOOLCHAIN` to every process cargo spawns, so a bare `cargo
  install` run inside the workspace compiles the tool with the pinned nightly.
  Current nightlies reject the `rustc_*` attributes cargo-insta's locked rustix
  dependency uses, so the tool build dies inside rustix. Stable leaves that cfg
  inert. The same hazard applies to any `cargo install` copied from these docs
  inside the repo — hence the `+stable` in every install one-liner.
- `cargo xtask smoke` is the pre-commit gate: `cargo fmt --all -- --check`, the
  markdown wrap check (`fmt-md --check`), the citation check
  (`check-citations`), `cargo clippy --workspace --all-targets --all-features --
  --deny warnings`, `cargo nextest run --workspace`. It must pass on every
  commit; each step propagates its exit code so the gate cannot silently pass.
- Unit tests run under `cargo nextest` (not plain `cargo test`). Install via
  `cargo +stable install cargo-nextest --locked` or let `cargo xtask
  install-tool` do it.
- Adding a crate needs a reason recorded in the commit message.
- Do not update the pinned nightly casually: bump the date in
  `rust-toolchain.toml` only when a new nightly is needed, and re-run the full
  gate set after.

## StS2 headless testing

- Headless boot requires `--headless --force-steam off` (without Steam running,
  the game stalls at the Steam-error popup otherwise). `--quit-after N` exits
  after N frames (~10s to main menu).

- With `--force-steam off`, the game reads settings from `<user data
  dir>/default/1/settings.save` (NOT the steam/ account-scoped one); on macOS
  that is `~/Library/Application Support/SlayTheSpire2/default/1/settings.save`.
  Mod loading requires `mod_settings.mods_enabled: true` there; the one-time
  enable (macOS):
  
  ```sh
  python3 -c "import json,os; p=os.path.expanduser('~/Library/Application Support/SlayTheSpire2/default/1/settings.save'); d=json.load(open(p)); d.setdefault('mod_settings',{})['mods_enabled']=True; json.dump(d,open(p,'w'))"
  ```

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

- The verdict's patch-count gate accepts \>= 199 patched methods (the static
  count is 203); below that means a large chunk of the catalog silently failed
  to patch.

- `headless-test` boots the game under a 10-minute watchdog (first boot after an
  install recompiles game shaders and can take 30-60s); a hung boot is killed
  and reported instead of hanging the terminal.

- Engine exit noise to ignore in headless logs: RID leaks of dummy renderer
  types, "ObjectDB instances leaked at exit", "Parameter t is null".

- Useful for source spelunking: godot 4.5.1 + gdextension-api sparse clones in
  `tmp/` (gitignored), and a downloaded `extension_api_4.5.1.json`.

## GDExtension (Godot 4.5.1, hand-rolled FFI)

The panel runs on a hand-rolled minimal GDExtension binding in
`profiler-core/src/engine/gdext.rs`: the surface is two Control subclasses, one
`_draw` virtual, one `refresh` method, and a handful of engine calls — far
below the cost of a full binding crate. Lessons from wiring it:

- **Engine API is resolved by name at runtime.** `gdextension_entry` gets the
  engine's `get_proc_address` and resolves each interface function by its C name
  (`classdb_register_extension_class5`, `get_variant_from_type_constructor`,
  `variant_call`, ...). A missing symbol fails the entry loudly (it names the
  symbol); there is no compile-time-typed API to catch drift, so the vendored
  `vendor/gdextension_interface.h` is the authoritative signature source.
- **Every engine method call is `variant_call`.** The ptrcall route needs
  `classdb_get_method_bind` signature hashes that are only published in the
  engine's `extension_api.json`; `variant_call` needs no such hashes, so it is
  the smaller, self-contained route. Values are built with
  `get_variant_from_type_constructor`, read back with
  `variant_get_ptr_internal_getter` (type-tag checked first — the internal
  getter is undefined behavior on a type mismatch), and every temporary Variant
  is `variant_destroy`ed on drop.
- **`_draw` dispatch is `get_virtual2`.** The class's `get_virtual_func` slot
  compares the incoming StringName against the cached virtual names by their
  interned data pointer (StringName is a single interned pointer in 4.5, so that
  is exact equality). Methods registered via
  `classdb_register_extension_class_method` never participate in virtual
  dispatch — `refresh` is a plain method, `_draw` is the only virtual.
- **macOS trackpad scrolling never sets wheel-button state.** Godot 4 delivers
  two-finger trackpad scrolls as `InputEventPanGesture` events; only a physical
  wheel produces `MOUSE_BUTTON_WHEEL_UP/DOWN`, and even then the button state
  lasts a single frame — a per-frame `Input.is_mouse_button_pressed` poll
  makes trackpads unscrollable and physical wheels timing-fragile. Scroll input
  therefore arrives from the C\# shim: it connects to each panel's `GuiInput`
  signal (same targeting as the `_gui_input` virtual, per stock 4.5.1
  `Control::_call_gui_input`) and forwards the raw event fields through the
  `spire_profiler_scroll_input` C export; the core translates and queues the
  pixels, and the per-frame `refresh` applies them.
- **The extension must NEVER touch an engine `InputEvent` object — every call
  shape tried froze the whole game.** Three builds, three deterministic freezes
  on the MegaDot fork (Mega Crit's custom 4.5.1 with embedded CoreCLR), each
  pinned by macOS `sample` plus disassembly: `Object.is_class` via
  `variant_call` inside the `_gui_input` virtual → null-jump in the fork's
  dispatch; the same call deferred to the per-frame refresh → same freeze,
  proving the break is the call itself; the pure C `object_get_class_name` on
  the retained event → the main thread parks inside the embedded CoreCLR's GC
  (stock 4.5.1 implements it with no managed code — the fork routes this entry
  into the .NET runtime and never returns). The panels' own engine calls
  (draw\_rect/draw\_string/get\_position/queue\_redraw/Input singleton/...) are
  fine: they target objects the extension created or global singletons. The
  toxic surface is calls ABOUT engine-created input-event objects. When in
  doubt, read the event in C\# — the game's own UI
  (`NScrollableContainer._GuiInput`) does exactly that — and forward plain
  scalars across the ABI.
- **StringName is interned; String is refcounted — different ownership
  rules.** The fixed set of StringNames is created once at Scene init and leaked
  by design: StringName is a single interned pointer in 4.5, so the engine owns
  the deduped storage for the extension's lifetime. A String is a refcounted
  copy-on-write value, not an interned handle: the temporary String built for a
  Variant must be destroyed once the Variant has copied from it, or the
  engine-side buffer leaks.
- **Theme font lifetime is the critical hazard.** Fetch
  `get_theme_default_font()` lazily on the first draw and store the result
  Variant in the panel state, NEVER destroying it — the object Ref inside
  keeps the Font alive. A dropped font ref renders no text; a failed fetch
  disables text but not bars, with a one-shot warning.
- **Verify method names against the extension API before wiring a call.**
  Real-play found panel clicks dead while hover worked: the code called
  `Viewport.get_mouse_button_state`, which does not exist in the 4.5.1 API; the
  failing call read as "never pressed", silently killing tab clicks. `xtask
  check-abi` verifies shim↔Rust exports, but nothing verifies Rust→engine
  method names — grep `extension_api_4.5.1.json` first. Mouse button state
  comes from the **`Input` singleton** (`global_get_singleton("Input")` +
  `is_mouse_button_pressed(1)`).
- **Panel parent class choice**: a `PanelContainer` parent paints its stylebox
  in `NOTIFICATION_DRAW`, which runs AFTER virtual `_draw` and would cover
  custom drawing — the panel is a plain `Control` and draws its own
  background/border.
- **Headless + registration**: both panel classes register at Scene init
  (`minimum_initialization_level = Scene`); the `panel class registered` line is
  a headless gate marker. The `_draw` virtual fires under the headless dummy
  renderer too (the boot logs `chart _draw active: N cmds`), so headless covers
  draw dispatch but not visual output — fonts/colors still need a real-play
  check.
- **Keep `#![deny(unsafe_code)]` intact**: the raw FFI (engine function
  pointers, raw `*mut c_void` reads, `extern "C"` callbacks) is quarantined in
  `engine/gdext.rs`; engine.rs scopes that module with `#[allow(unsafe_code)]`
  (the third such relaxation, after `abi` and `registration`). Every C callback
  routes through a panic-containment helper (mirroring `abi::contain`) so a Rust
  panic can never unwind into the engine.
- **Engine-free testing**: the interaction decision logic is factored into pure
  functions (`press_zone`, `dismiss_on_outside_press`, `content_signature`, the
  scrollbar state machine) that take the local `math::Vector2`/`Rect2` (plain
  `f32` structs — no engine calls), so they are unit-tested under nextest
  without booting Godot.

## Platform layout (Windows/Linux assumptions)

The macOS layout is inspected ground truth; Windows/Linux follow Godot export
conventions and the v0.111.0 decompiled source, but have NOT been verified on a
real machine. Before trusting them, check:

- `ModManager.cs` (decompiled) derives the mods dir from
  `OS.GetExecutablePath()` on **every** OS — so mods always sit in `<exe
  dir>/mods/`, and the exe dir differs per platform (the `.app`'s
  `Contents/MacOS` on macOS, the game root on Windows/Linux).
- `project.godot` sets `use_custom_user_dir` + `custom_user_dir_name =
  "SlayTheSpire2"`, which is why the user-data dir is `<data
  dir>/SlayTheSpire2/` on all three OSes: `~/Library/Application Support/` on
  macOS (confirmed), `%APPDATA%\` on Windows, `~/.local/share/` (or
  `$XDG_DATA_HOME`) on Linux. The `SaveManager.cs` doc comment mentioning
  `Godot\app_userdata\sts2` on Windows is stale (pre-dates the custom user dir).
- The Linux data dir is `data_sts2_linuxbsd_x86_64` (Godot 4 exports the Linux
  build under its internal `linuxbsd` platform name) and the exe is
  `SlayTheSpire2` (not `Slay the Spire 2.x86_64`). Every derived path is
  existence-checked before use, and `STS2_GAME_DIR` overrides the whole game
  tree (`STS2_USER_DATA_DIR` overrides the user-data/log tree), so a wrong
  assumption is an explicit error naming the expected path, never silent
  misbehavior.
- The **Windows layout has never been verified on a real machine**:
  `data_sts2_windows_x86_64`, `Slay the Spire 2.exe`, `%APPDATA%\SlayTheSpire2\`
  are convention-derived only — the same kind of guesses that were wrong on
  Linux. Treat any Windows failure against these paths as a layout assumption
  first; `STS2_GAME_DIR` overrides.
- Steam/Steam Deck users normally run the **Windows** build under Proton. Under
  Proton the game process is a Windows process, so the `windows.x86_64`
  gdextension key and the shim's `OperatingSystem.IsWindows()` branch fire —
  the Linux layout is for the native Linux build. The game's own GDExtension
  addons (fmod, sentry, spine) use `linux.*` feature tags, which is why the
  shipped gdextension keys `linux.x86_64`, not `linuxbsd.x86_64`.
- Linux Steam library roots differ by client/installer, so the xtask does not
  hardcode one: without `STS2_GAME_DIR` it reads every `libraryfolders.vdf` the
  client layout can carry (the modern `~/.local/share/Steam` AND the older
  `~/.steam/steam`; each manifest's `"path"` entries name every library root,
  secondary libraries included), then falls back to the `~/.local/share/Steam`
  default when no manifest exists. Flatpak Steam keeps its data elsewhere and
  remains one `STS2_GAME_DIR` export away.
- `release_info.json` (the game's own `{"commit", "version"}` stamp, beside the
  `.pck`) is the version pin: it lives at
  `SlayTheSpire2.app/Contents/Resources/release_info.json` on macOS (verified)
  and at `<game root>/release_info.json` on Windows/Linux. Its `version` field
  is pinned in `xtask/src/game_version.rs` (`PIN = "v0.111.0"`): `build`,
  `install-mod`, `headless-test`, and `decompile` all fail fast when the
  installed game's version differs, naming both versions and the
  `release_info.json` path. A Steam game update therefore breaks every command
  until `PIN` is bumped deliberately and the mod is re-verified (the Harmony
  patch catalog above all) against the new game.

## Cross-compilation (cargo-zigbuild as the cross linker)

- `cargo xtask build` ALWAYS produces the complete cross-platform bundle (there
  is no host-only mode). The native matrix (`xtask/src/cross.rs`) is a four-row
  table, all built with one `zigbuild` invocation (`cargo zigbuild --target
  <triple> ...`, which runs rustc with zig as the linker). Needs:
  `cargo-zigbuild` (`cargo +stable install cargo-zigbuild --version 0.23.0
  --locked`, installed pinned by `install-tool`), zig 0.16.0 (bootstrapped into
  `zig-sdk/` — eagerly by `install-tool`, or lazily by the first build — and
  passed to cargo-zigbuild via `CARGO_ZIGBUILD_ZIG_PATH`; the build ALWAYS uses
  that bootstrap, PATH is never consulted, and every resolve version-checks the
  bootstrap binary against the pin — a mismatch wipes the dir and
  re-bootstraps the pinned tarball), and the rust stds of all four rows
  (auto-installed via `rustup target add` for the pinned nightly when missing
  — zig supplies only the linker and C libraries, so rustc still needs each
  target's std). The cross tooling is preflighted once, up front, before any
  compile: every missing piece fails the build with the exact command in the
  error. macOS and Linux hosts both produce the full four-library bundle;
  Windows hosts fail the build up front (the hermetic .NET bootstrap is a Unix
  script). The `.gdextension`'s `[libraries]` section is rendered from the same
  matrix, so the keys and the file names cannot drift.
- **Windows-gnu cross-compiles via cargo-zigbuild.** Plain `zig cc` rejects
  rust's `x86_64-pc-windows-gnu` std (it links `-lmsvcrt -lmingwex -lmingw32`
  and passes a rustc-generated `-Wl,<file>.def` export list), but cargo-zigbuild
  rewrites the linker invocation, filters the `.def` argument, and supplies the
  mingw CRT pieces itself. Verified: cargo-zigbuild 0.23.0 + zig 0.16.0 produce
  a PE32+ `profiler_core.dll` exporting all 41 profiler ABI symbols. Note the
  DLL links UCRT api-sets (`api-ms-win-crt-*`), i.e. Windows 10+ — fine for a
  game whose own floor is Windows 10, but not loadable on Win7/8.
- The Linux `.so` is built with the target triple
  `x86_64-unknown-linux-gnu.2.17`: cargo-zigbuild passes the glibc floor to zig
  cc, and `llvm-objdump -T` confirms no symbol requires more than `GLIBC_2.17`
  (the oldest distro Steam still supports; bump deliberately if Steam raises
  it). cargo strips the `.2.17` suffix from the output directory, keeping the
  plain `target/x86_64-unknown-linux-gnu/` layout. Every row is zig-built, so
  the floor holds on every host.
- **macOS thin dylibs.** The bundle ships TWO thin dylibs, one per macOS arch,
  each keyed to its `macos.*` entry, and the shim's selector has a
  `RuntimeInformation.ProcessArchitecture` branch for macOS. Verified on an
  arm64 macOS host: `cargo zigbuild --target aarch64-apple-darwin` (and x86\_64)
  link the cdylibs from zig's bundled libSystem stubs with NO Apple SDK
  (`SDKROOT` unset), producing ad-hoc linker-signed Mach-O thin dylibs that link
  only libSystem; the aarch64 one additionally dlopen-passes on that host. No
  lipo step.
- The cross builds compile only the serde family (~71k lines/target) plus
  profiler-core — the FFI in `gdext.rs` is plain `extern "C"` against the
  vendored header (no generated bindings), so there is no per-target codegen
  blowup.

## Hermetic .NET SDK bootstrap

The xtask owns its .NET SDK (no pre-installed dotnet required): the vendored
`tools/dotnet-install.sh` (official Microsoft installer, MIT, provenance banner
at the top) installs the pinned channel 9.0 into the gitignored `dotnet-sdk/`,
and the build shells out to that binary. Lessons from wiring it:

- **Why channel 9.0 and why it is offline**: the generated csproj targets
  net9.0. SDK 9.0.x tarballs bundle the net9.0 targeting pack
  (`dotnet-sdk/packs/Microsoft.NETCore.App.Ref/9.0.x`), so the shim builds with
  NO NuGet restore. Before the bootstrap, the build ran on the system
  SDK 10.0.302, which bundles only the 10.0.10 pack — the net9.0 build
  silently fetched `microsoft.netcore.app.ref` from nuget.org on first build. Do
  not "upgrade" the pin to LTS: `dotnet-install.sh`'s upstream default channel
  is `LTS`, which since .NET 10's release would install an SDK 10.x and
  reintroduce the NuGet dependency (the vendored copy's default is overridden
  to 9.0 for this reason — see its banner).
- **Channel feed lifetime**: .NET 9 is an STS release (support ended 2026-05),
  but the CDN still serves `--channel 9.0` (resolving to 9.0.317). If Microsoft
  ever retires the 9.0 latest.version feed, pin the exact version with
  `--version 9.0.317` in the vendored script instead (the payload URLs remain).
- **Worktrees**: gitignored files are NOT shared across worktrees — every
  worktree bootstraps its own ~500 MB `dotnet-sdk/` (automatically, on its first
  build). There is no location override.
- **Windows**: the vendored installer is bash (macOS/Linux). On Windows, the
  equivalent is `dotnet-install.ps1` (same `-Channel 9.0` flag, from
  `https://dot.net/v1/dotnet-install.ps1`); `install-tool`'s bootstrap fails
  there with exactly that instruction.
- **DOTNET\_ROOT hijack**: a relocated SDK's `dotnet` muxer consults the
  `DOTNET_ROOT` env var for its root, and a pre-existing value pointing at
  another SDK install confuses it. The xtask pins `DOTNET_ROOT` to the bootstrap
  dir whenever it runs the bootstrapped binary; if you run `dotnet-sdk/dotnet`
  by hand and it fails to find SDKs, check that env var first.
- **Reruns are cheap**: `ensure_bootstrap` skips the installer entirely when
  `dotnet-sdk/dotnet` exists, so `install-tool` on a provisioned machine is fast
  and version-stable (the channel resolves to the latest patch, which the cached
  install keeps until the dir is deleted).

## StS2 modding environment

- The game's mods directory is `<exe dir>/mods/` — on macOS
  `SlayTheSpire2.app/Contents/MacOS/mods/` (NOT a `mods/` folder next to the
  .app — on macOS that one is unused).
- The mod scanner reads every `*.json` under `mods/` recursively as a manifest.
  Mod data therefore lives at the sibling `<exe dir>/mod_data/spire-profiler/`
  — deliberately OUTSIDE the sweep: an early placement under `mods/mod_data/`
  made every boot log ERRORs as the scanner tried to parse the data files as
  manifests (harmless, but it hid real manifest errors).
- `settings.save` JSON keys are snake\_case; mod consent lives at
  `mod_settings.mods_enabled` (verified against a live settings.save).
- A single unparseable entry (a corrupted card id in an old combat record) makes
  the whole-file parse fail at every boot; fresh entries parse fine. Keep
  whole-file parse failures recoverable (quarantine/rotate the bad entry) rather
  than letting one corrupt record poison aggregation forever.

## Game-version drift (decompiled snapshot vs the v0.111.0 pin)

Always verify patch targets against the decompiled source. The current snapshot
lives at `tmp/sts2-decompiled/` (gitignored; regenerate with `cargo xtask
decompile`) and matches the v0.111.0 game pin. The drift findings below were
verified against the earlier v0.110.1 snapshot and must be re-checked against
the current one before acting — hook names/types drift across game versions.

- `CombatRoom.Resume(AbstractRoom, IRunState?)` exists but its body is `throw
  new NotImplementedException()` — Harmony postfixes on it never fire
  (exceptions skip postfixes). Older-build mods used it as a refresh site; in
  v0.110.1 it is a dead site (kept patched, harmless).
- `CombatManager.SetUpCombat` takes `CombatState` in v0.110.1; older hooks
  declare `CombatStateType`. Harmony injects by name, so our postfix takes
  `ICombatState` — do not copy older parameter types blindly.
- Turn hooks became side-based (e.g. `BeforeTurnEnd` → `BeforeSideTurnEnd`).

## Potion-applied power attribution

- `CombatHistory.PotionUsed` fires only AFTER `PotionModel.OnUse` completes
  (inside `PotionModel.OnUseWrapper`, v0.111.0) — a postfix there records the
  potion fallback too late for powers the potion applies during OnUse
  (FlexPotion's +5 Strength). The shim therefore prefixes
  `PotionModel.OnUseWrapper` (`spire_profiler_potion_context_begin`) so the
  fallback exists BEFORE the effects run; the PotionUsed postfix bumps the
  potions\_used counter and records a fresh hash == 0 entry that re-points the
  potion fallback.
- A player's Strength DECREASE is a temporary-power expiry (FlexPotionPower
  applies −5 at side turn end); the core consumes the same amount from the
  recorded Strength appliers FIFO and removes exhausted entries. Deliberate:
  leaving expired Strength credited forever would silently over-credit — the
  FIFO must reflect the live amount.
- `potion_used`/`potion_context_begin` push into the shared `orb_sources` table
  (hash 0 entries, potion kind); both paths are bounded by `caps::ORB_SOURCES`
  with fail-logs on overflow.

## Decompiling the game source (`cargo xtask decompile`)

`cargo xtask decompile` recovers Slay the Spire 2's Godot project source
(scenes, C\# scripts, resources) from the game's `.pck` file, so the game's
architecture and implementation can be studied. It is a **thin wrapper** around
GDRE Tools (Godot RE Tools) — the decompiler does all the work; the subcommand
only locates the game, provisions the pinned tool, runs it, and verifies the
result. No decompiler logic lives in this repo.

### Educational use only

Intended for educational purposes and personal study of the game's architecture
and implementation, as permitted by the developers. See the [Reddit
discussion](https://www.reddit.com/r/godot/comments/1rm7ueb/comment/o8zqpit/)
where the developer stated:

> "It'd make me extremely happy to find out that other game developers learned
> something from reading through our code and our scenes :)"

**Do not redistribute**: decompiled code is for personal study only. Do not
share the decompiled files or use them for commercial purposes. The output is
gitignored (`tmp/`), never committed.

### Prerequisites

- A purchased copy of Slay the Spire 2, installed via Steam.
- macOS or Linux (Windows is out of scope: the verified tool only documents the
  macOS/Linux Steam PCK layouts, so the wrapper rejects other hosts).
- `curl` and `unzip` on PATH (present by default on macOS and Linux; used to
  download and extract GDRE Tools).
- Network access on first run to download GDRE Tools v2.5.0-beta.5 (~100 MB,
  SHA-256 verified, cached under `tmp/gdre-tools/` for later runs).
- ~3 GB free disk space (the GDRE Tools download plus the extracted source).
- 1–2 minutes of runtime per decompilation.

### Usage

```sh
cargo xtask decompile [output_dir] [--yes|-y] [--help|-h]
```

- `output_dir` — where the recovered project is written. Defaults to
  `tmp/sts2-decompiled/`. Must be omitted to use the default.
- `-y`, `--yes` — delete an existing output directory without the interactive
  `y/N` confirmation (default is to prompt).

The game is located automatically: `STS2_GAME_DIR` (the directory containing
`SlayTheSpire2.app` on macOS, or the game executable on Linux) wins when set;
otherwise every Steam library in `libraryfolders.vdf` is walked and the `.pck`
found under `steamapps/common/Slay the Spire 2/`. A failure names every path
that was searched.

After a successful run the output directory contains the recovered Godot project
(`project.godot`, `src/`, `scenes/`, ...) plus a `.provenance.json` recording
when it was produced, the host platform, the source `.pck` path, the GDRE Tools
version, and whether `gdre_export.log` was written.

### Troubleshooting

**`cargo xtask decompile` crashes GDRE but `./target/debug/xtask decompile`
works.** GDRE segfaults in its native decompiler worker threads only when
launched through `cargo`, never when the identical `xtask` binary runs directly:
cargo leaks POSIX signal state (SIGUSR1 keeps `SA_SIGINFO` through `exec`), and
GDRE's .NET Native AOT runtime uses SIGUSR1 internally — with the leftover
flag its worker threads dispatch through an uninitialized function pointer. The
subcommand resets every standard signal to `SIG_DFL` with cleared flags in a
`pre_exec` hook before exec'ing GDRE — no action needed.

**GDRE exits with signal 11 right after "Loading GDScript cache"**, and the
output shows `Failed to open 'user://logs/...'`: the Godot user-data directory
is unwritable (restricted/sandboxed or read-only-HOME environments). Point
`HOME` at a writable, gitignored directory inside the repo:

```sh
HOME="$PWD/tmp/gdre-home" cargo xtask decompile
```
