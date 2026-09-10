# The Slay the Spire 2 environment

## Game discovery

- `STS2_GAME_DIR` wins when set (macOS: the directory containing
  `SlayTheSpire2.app`; Windows/Linux: the directory containing the executable);
  otherwise the platform's Steam libraries are searched. A WSL2 host's Windows
  install under `/mnt/<drive>` is detected from its contents, not from the host
  platform.
- The Linux search reads every `libraryfolders.vdf` under both
  `~/.local/share/Steam` (modern) and `~/.steam/steam` (legacy), then falls back
  to the default root. Flatpak Steam keeps its data elsewhere, so it needs
  `STS2_GAME_DIR`.
- `STS2_USER_DATA_DIR` forces the log dir.
- Every derived path is existence-checked, so a bad override or a renamed layout
  is an explicit error naming the expected path, never silent misbehavior.

## Platform layout (native Linux still unverified)

The macOS layout is verified by inspection, the Windows layout against a real
install of the pinned game version driven from a WSL2 host. The native-Linux
layout follows Godot export conventions and the decompiled source but has NOT
been verified on real hardware (`data_sts2_linuxbsd_x86_64`, the bare
`SlayTheSpire2` binary name). Treat a native-Linux path failure as a layout
assumption first; `STS2_GAME_DIR` overrides.

- `project.godot` sets `use_custom_user_dir` with `custom_user_dir_name =
  "SlayTheSpire2"`, so the user-data dir is `<data dir>/SlayTheSpire2/` on all
  three OSes: `~/Library/Application Support/` on macOS (confirmed),
  `%APPDATA%\` on Windows (confirmed), `~/.local/share/` (or `$XDG_DATA_HOME`)
  on Linux. The `SaveManager.cs` doc comment naming `Godot\app_userdata\sts2` on
  Windows is stale; it pre-dates the custom user dir.
- Steam Deck and Proton run the Windows build: the game process is a Windows
  process, so the `windows.x86_64` gdextension key and the shim's
  `OperatingSystem.IsWindows()` branch fire. The native Linux build's key is
  `linux.x86_64` (not `linuxbsd.x86_64`) because the game's own addons use
  `linux.*` feature tags.
- `release_info.json` beside the `.pck` is the game's own `{"commit",
  "version"}` stamp: `SlayTheSpire2.app/Contents/Resources/release_info.json` on
  macOS (verified), `<game root>/release_info.json` on Windows (verified) and
  Linux. Its `version` is the pin checked in
  [game\_version.rs](../xtask/src/game_version.rs).

## StS2 modding environment

- The decompiled `ModManager.cs` derives the mods dir from
  `OS.GetExecutablePath()` on every OS, so mods live in `<exe dir>/mods/`:
  `SlayTheSpire2.app/Contents/MacOS/mods/` on macOS (a `mods/` next to the
  `.app` is unused), `<game root>/mods/` on Windows/Linux. The game creates it
  on the first mods-enabled boot, so discovery does not require it to exist;
  install-mod's copy does the mkdir.
- The mod scanner reads every `*.json` under `mods/` recursively as a manifest,
  so mod data lives at the sibling `<exe dir>/mod_data/spire-profiler/`, outside
  the sweep: data files under `mods/` log ERRORs every boot as the scanner
  parses them as manifests (harmless, but they hide real manifest errors).
- `settings.save` keys are snake\_case; mod consent lives at
  `mod_settings.mods_enabled` (verified against a live settings.save) and is
  scoped per settings file.

## Game-version drift (decompiled snapshot vs the pinned version)

Verify patch targets against the decompiled source at `tmp/sts2-decompiled/`
(gitignored; regenerate with `cargo xtask decompile`); its `.provenance.json`
must name the pin in [game\_version.rs](../xtask/src/game_version.rs). The
update runbook lives in AGENTS.md.

`cargo xtask check-catalog` compares the hand-curated catalog in
[catalog.rs](../xtask/src/catalog.rs) and its reviewed exclusions against the
decompiled relic and power classes. The check is syntax-based: read the hook
bodies before changing the catalog.

`cargo xtask managed-test` additionally checks the installed assembly
identities, producer definition inventory, and exact patch bridges. The catalog
is a body-review baseline; runtime producer coverage is broader.

The findings below are the traps check-catalog cannot see (dead hook bodies,
renamed parameter types). Verified against the v0.111.0 snapshot.

- `CombatRoom.Resume(AbstractRoom, IRunState?)` exists but its body is `throw
  new NotImplementedException()`, so Harmony postfixes on it never fire
  (exceptions skip postfixes). Older-build mods used it as a refresh site; it
  stays patched, harmless.
- `CombatManager.SetUpCombat` takes `CombatState`; older hooks declare
  `CombatStateType`. Harmony injects by name, so the postfix takes
  `ICombatState`; do not copy older parameter types blindly.
- Turn hooks are side-based: older mods' `BeforeTurnEnd` is now
  `BeforeSideTurnEnd`.
- The canonical `CreatureCmd.Damage` overload has seven arguments and an
  `IEnumerable<Creature>` target parameter. Its state machine has two
  result-list enumerators: the first follows the completed target group; the
  second runs aggregate late hooks. The managed gate verifies the exact
  replacement sites.
- Reflection can return an inherited `MethodInfo` with a different reflected
  type from its declaring type. Harmony requires the declared method;
  deduplicate by module/metadata token and resolve that definition on its
  declaring type.
- .NET's async task builders have internal overloads of `SetResult` and
  `SetException`. Bridge lookup requires the exact public instance signature.

## Decompiling the game source

`cargo xtask decompile` recovers the Godot project source from the game's `.pck`
via GDRE Tools; `--help` carries usage. The first run downloads the pinned tool
(SHA-256 verified, cached under `tmp/gdre-tools/`; the pin lives in
[decompile.rs](../xtask/src/decompile.rs)). The output lands in gitignored
`tmp/sts2-decompiled/` with a `.provenance.json`.

Decompiled output is for personal study only, never redistributed, as the
developer permits: "It'd make me extremely happy to find out that other game
developers learned something…"
([source](https://www.reddit.com/r/godot/comments/1rm7ueb/comment/o8zqpit/)).

- GDRE segfaults when launched through `cargo` but not from the bare
  `target/debug/xtask` binary: cargo leaks `SA_SIGINFO` on SIGUSR1 across exec
  into GDRE's NativeAOT runtime. The subcommand resets every signal in a
  `pre_exec` hook before exec, so no action is needed.

- Signal 11 right after "Loading GDScript cache", with "Failed to open
  'user://logs/...'" in the output, means the Godot user-data dir is unwritable
  (read-only HOME). Point HOME at a writable repo-local dir:
  
  ```sh
  HOME="$PWD/tmp/gdre-home" cargo xtask decompile
  ```
