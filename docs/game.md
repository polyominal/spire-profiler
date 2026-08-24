# The Slay the Spire 2 environment

Where the game and its data live on every supported platform, how the xtask
finds them, the modding environment, game-version drift, and the decompiler
workflow. Building and verification live in `build.md` and `verify.md`.

## Game discovery

Per-OS Steam default path, or `STS2_GAME_DIR` (macOS: the directory containing
`SlayTheSpire2.app`; Windows/Linux: the directory containing the executable); a
foreign-platform tree — the Windows install mounted into WSL2 — is detected
from its contents. `STS2_USER_DATA_DIR` forces the log dir. (WSL2 specifics:
`build.md`.)

## Platform layout (native Linux still unverified)

The macOS and Windows layouts are verified ground truth: macOS by inspection,
Windows against a real install of the pinned game version driven from a WSL2
host (see `build.md`). The native-Linux layout follows Godot export conventions
and the decompiled source but has NOT been verified on a real machine;
`data_sts2_linuxbsd_x86_64` and the bare `SlayTheSpire2` binary name are
convention-derived only. Treat any native-Linux failure against these paths as a
layout assumption first; `STS2_GAME_DIR` overrides.

- `ModManager.cs` (decompiled) derives the mods dir from
  `OS.GetExecutablePath()` on **every** OS — so mods always sit in `<exe
  dir>/mods/`, and the exe dir differs per platform (the `.app`'s
  `Contents/MacOS` on macOS, the game root on Windows/Linux).
- `project.godot` sets `use_custom_user_dir` + `custom_user_dir_name =
  "SlayTheSpire2"`, which is why the user-data dir is `<data
  dir>/SlayTheSpire2/` on all three OSes: `~/Library/Application Support/` on
  macOS (confirmed), `%APPDATA%\` on Windows (confirmed), `~/.local/share/` (or
  `$XDG_DATA_HOME`) on Linux. The `SaveManager.cs` doc comment mentioning
  `Godot\app_userdata\sts2` on Windows is stale (pre-dates the custom user dir).
- Every derived path is existence-checked before use, so a wrong assumption is
  an explicit error naming the expected path, never silent misbehavior.
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
  and at `<game root>/release_info.json` on Windows (verified) and Linux. Its
  `version` field is pinned in `xtask/src/game_version.rs`: `build`,
  `install-mod`, `headless-test`, and `decompile` all fail fast when the
  installed game's version differs, naming both versions and the
  `release_info.json` path. A Steam game update therefore breaks every command
  until the pin is bumped deliberately; that bump starts re-verification of the
  mod (the Harmony patch catalog above all) against the new game.

## StS2 modding environment

- The game's mods directory is `<exe dir>/mods/` — on macOS
  `SlayTheSpire2.app/Contents/MacOS/mods/` (NOT a `mods/` folder next to the
  .app — on macOS that one is unused). The game creates it on the first
  mods-enabled boot, so discovery does not require it to exist; install-mod's
  copy does the mkdir.
- The mod scanner reads every `*.json` under `mods/` recursively as a manifest.
  Mod data therefore lives at the sibling `<exe dir>/mod_data/spire-profiler/`
  — deliberately OUTSIDE the sweep: data under `mods/mod_data/` makes every
  boot log ERRORs as the scanner tries to parse the data files as manifests
  (harmless, but it hides real manifest errors).
- `settings.save` JSON keys are snake\_case; mod consent lives at
  `mod_settings.mods_enabled` (verified against a live settings.save), and
  consent is scoped per settings file.

## Game-version drift (decompiled snapshot vs the pinned version)

Always verify patch targets against the decompiled source. The current snapshot
lives at `tmp/sts2-decompiled/` (gitignored; regenerate with `cargo xtask
decompile`) and its provenance must name the pin in `xtask/src/game_version.rs`.
The update runbook lives in AGENTS.md.

`cargo xtask check-catalog` compares the hand-curated attribution catalog and
its reviewed exclusions with the decompiled relic and power classes. It fails on
renamed or overloaded methods, namespace drift, entries that no longer show a
tracked effect, new effect-producing hooks, and stale reviewed exclusions. The
check is syntax-based: a new or changed report still requires reading the hook
bodies before changing the catalog.

The findings below record traps check-catalog cannot see (dead hook bodies,
renamed parameter types). Re-check them against the new snapshot on every game
update and re-date this note; hook names and types drift across versions.
Verified against the v0.111.0 snapshot.

- `CombatRoom.Resume(AbstractRoom, IRunState?)` exists but its body is `throw
  new NotImplementedException()` — Harmony postfixes on it never fire
  (exceptions skip postfixes). Older-build mods used it as a refresh site; it is
  kept patched, harmless.
- `CombatManager.SetUpCombat` takes `CombatState`; older hooks declare
  `CombatStateType`. Harmony injects by name, so our postfix takes
  `ICombatState` — do not copy older parameter types blindly.
- Turn hooks are side-based: older mods' `BeforeTurnEnd` is now
  `BeforeSideTurnEnd`.

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
- macOS, Linux, or a WSL2 host against the Windows install (native Windows hosts
  are rejected at `Platform::detect`; the Windows install's `.pck` is found
  through layout detection — see `build.md`).
- `curl` and `unzip` on PATH (present by default on macOS and Linux; used to
  download and extract GDRE Tools).
- Network access on first run to download the pinned GDRE Tools release (~100
  MB, SHA-256 verified, cached under `tmp/gdre-tools/` for later runs; the pin
  lives in `xtask/src/decompile.rs`).
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
version, the game version, and whether `gdre_export.log` was written.

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
