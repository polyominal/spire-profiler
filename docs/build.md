# Building the mod

How the mod is compiled and packaged: the xtask command surface, toolchain pins,
cross-compilation, the hermetic .NET SDK bootstrap, and the supported host
environments. Verification gates live in `verify.md`; where the game and its
data live on disk is `game.md`.

## Command surface

Run `cargo xtask --help` (and `cargo xtask <command> --help`) for the command
list and per-command flags. The facts the help strings do not carry:

- `build` always produces the complete cross-platform bundle (there is no
  host-only mode) and runs `check-abi` inside.
- `install-tool` is the optional up-front path (host tools + bootstraps
  provisioned eagerly), not a prerequisite: the first build or command that
  needs a piece provisions it lazily.
- `smoke` is the pre-commit gate (see `verify.md`).

Prerequisites: a Steam install of Slay the Spire 2 and the pinned Rust nightly
(`rust-toolchain.toml` auto-fetches it). No pre-installed .NET SDK, zig, or
cross-target stds are needed — the build bootstraps the toolchains (see the
sections below; `install-tool` provisions all of them eagerly). Native Windows
hosts are rejected at `Platform::detect` by every command that touches the game
or the toolchain bootstraps; only the repo-local gates (`smoke`, `check-docs`,
...) run there. The Windows-side dev environment is WSL2 — see the WSL2
section.

## Rust toolchain (pinned nightly)

- The toolchain is pinned in `rust-toolchain.toml`; rustup selects it
  automatically inside the workspace, so no toolchain install step is needed.
- `install-tool` builds the host tools (nextest, insta, zigbuild) with `cargo
  +stable install ... --locked`, never the pinned nightly: the rustup proxy
  exports `RUSTUP_TOOLCHAIN` to every process cargo spawns, so a bare `cargo
  install` run inside the workspace compiles the tool with the pinned nightly,
  and nightlies reject the `rustc_*` attributes cargo-insta's locked rustix
  dependency uses (the tool build dies inside rustix). Stable leaves that cfg
  inert. The same hazard applies to any `cargo install` copied from these docs
  inside the repo — hence the `+stable` in every install one-liner.
- Unit tests run under `cargo nextest` (not plain `cargo test`). Install via
  `cargo +stable install cargo-nextest --locked` or let `cargo xtask
  install-tool` do it.
- Adding a crate needs a reason recorded in the commit message.
- Do not update the pinned nightly casually: bump the date in
  `rust-toolchain.toml` only when a new nightly is needed, and re-run the full
  gate set (`verify.md`) after.

## Cross-compilation (cargo-zigbuild as the cross linker)

- `cargo xtask build` produces the complete native matrix pinned in
  `xtask/src/cross.rs` (a four-row table) with one `zigbuild` invocation (`cargo
  zigbuild --target <triple> ...`, which runs rustc with zig as the linker).
  Needs: `cargo-zigbuild` at the version pinned in `cross.rs` (installed pinned
  by `install-tool`), zig at the version pinned in `xtask/src/zig.rs`
  (bootstrapped into `zig-sdk/` — eagerly by `install-tool`, or lazily by the
  first build — and passed to cargo-zigbuild via `CARGO_ZIGBUILD_ZIG_PATH`;
  the build ALWAYS uses that bootstrap, PATH is never consulted, and every
  resolve version-checks the bootstrap binary against the pin — a mismatch
  wipes the dir and re-bootstraps the pinned tarball), and the rust stds of all
  four rows (auto-installed via `rustup target add` for the pinned nightly when
  missing — zig supplies only the linker and C libraries, so rustc still needs
  each target's std). The cross tooling is preflighted once, up front, before
  any compile: every missing piece fails the build with the exact command in the
  error. macOS and Linux hosts both produce the full four-library bundle; native
  Windows hosts fail at `Platform::detect` before any compile (the Windows-side
  dev environment is WSL2). The `.gdextension`'s `[libraries]` section is
  rendered from the same matrix, so the keys and the file names cannot drift.
- **Windows-gnu cross-compiles via cargo-zigbuild.** Plain `zig cc` rejects
  rust's `x86_64-pc-windows-gnu` std (it links `-lmsvcrt -lmingwex -lmingw32`
  and passes a rustc-generated `-Wl,<file>.def` export list), but cargo-zigbuild
  rewrites the linker invocation, filters the `.def` argument, and supplies the
  mingw CRT pieces itself. Verified: the result is a PE32+ `profiler_core.dll`
  exporting every profiler ABI symbol plus `gdextension_entry`. The DLL links
  UCRT api-sets (`api-ms-win-crt-*`), i.e. Windows 10+ — fine for a game whose
  own floor is Windows 10, but not loadable on Win7/8.
- The Linux `.so` is built with the glibc floor encoded in the target triple
  pinned in `cross.rs` (`x86_64-unknown-linux-gnu.2.17`; the oldest distro Steam
  still supports — bump deliberately if Steam raises it). cargo-zigbuild
  passes the floor to zig cc, and `llvm-objdump -T` confirms no symbol requires
  a newer GLIBC. cargo strips the `.2.17` suffix from the output directory,
  keeping the plain `target/x86_64-unknown-linux-gnu/` layout. Every row is
  zig-built, so the floor holds on every host.
- **macOS thin dylibs.** The bundle ships TWO thin dylibs, one per macOS arch,
  each keyed to its `macos.*` entry, and the shim's selector has a
  `RuntimeInformation.ProcessArchitecture` branch for macOS. Verified on an
  arm64 macOS host: `cargo zigbuild --target aarch64-apple-darwin` (and x86\_64)
  link the cdylibs from zig's bundled libSystem stubs with NO Apple SDK
  (`SDKROOT` unset), producing ad-hoc linker-signed Mach-O thin dylibs that link
  only libSystem; the aarch64 one additionally dlopen-passes on that host. No
  lipo step.
- The cross builds compile only the serde family plus profiler-core — the FFI
  in `gdext.rs` is plain `extern "C"` against the vendored header (no generated
  bindings), so there is no per-target codegen blowup.

## Hermetic .NET SDK bootstrap

The xtask owns its .NET SDK (no pre-installed dotnet required): the vendored
`tools/dotnet-install.sh` (official Microsoft installer, MIT, provenance banner
at the top) installs the channel pinned in `xtask/src/dotnet.rs` into the
gitignored `dotnet-sdk/`, and the build shells out to that binary. Lessons from
wiring it:

- **Why that channel and why it is offline**: the generated csproj targets
  net9.0, and 9.0.x tarballs bundle the net9.0 targeting pack
  (`dotnet-sdk/packs/Microsoft.NETCore.App.Ref/`), so the shim builds with NO
  NuGet restore. An SDK 10.x bundles only the 10.x pack, and the net9.0 build
  silently fetches `microsoft.netcore.app.ref` from nuget.org on first build. Do
  not "upgrade" the pin to LTS: `dotnet-install.sh`'s upstream default channel
  is `LTS`, which resolves to an SDK 10.x and reintroduces the NuGet dependency
  (the vendored copy's default is overridden to the pinned channel for this
  reason — see its banner).
- **Channel feed lifetime**: .NET 9 is an STS release (out of support since
  2026-05); the bootstrap depends on the channel's CDN feed staying up. If
  Microsoft ever retires it, pin the exact version with `--version` in the
  vendored script instead (the payload URLs remain).
- **Worktrees**: gitignored files are NOT shared across worktrees — every
  worktree bootstraps its own ~500 MB `dotnet-sdk/` (automatically, on its first
  build). There is no location override.
- **Windows**: the vendored installer is bash (macOS/Linux), and native Windows
  hosts are rejected at `Platform::detect` before any bootstrap runs — the
  Windows-side dev environment is WSL2 (section below), where the bash installer
  applies.
- **DOTNET\_ROOT hijack**: a relocated SDK's `dotnet` muxer consults the
  `DOTNET_ROOT` env var for its root, and a pre-existing value pointing at
  another SDK install confuses it. The xtask pins `DOTNET_ROOT` to the bootstrap
  dir whenever it runs the bootstrapped binary; if you run `dotnet-sdk/dotnet`
  by hand and it fails to find SDKs, check that env var first.
- **Reruns are cheap**: `ensure_bootstrap` skips the installer entirely when
  `dotnet-sdk/dotnet` exists, so `install-tool` on a provisioned machine is fast
  and version-stable (the channel resolves to the latest patch, which the cached
  install keeps until the dir is deleted).

## WSL2 host against the Windows install

A shell-only WSL2 box uses the Windows game as its game instance: a Linux host
whose tree has the Windows data dir resolves as the Windows layout, and the
build's game inputs (the managed assemblies, `release_info.json`) are
platform-neutral. It is the ONLY supported foreign layout — the game otherwise
matches the host platform, and `Platform::detect` rejects native Windows hosts
outright (the Windows-side dev environment is WSL2, full stop). Verified end to
end against a Windows install of the pinned game version driven from a WSL2
host: `build`, `install-mod`, `release`, `decompile`, and `headless-test` PASS.
Setup and traps:

- No env setup is needed against a default install: discovery reads the Windows
  Steam manifests under every `/mnt/<drive>` drive, and headless-test derives
  `%APPDATA%` through interop (`cmd.exe` + `wslpath`). `STS2_GAME_DIR` and
  `STS2_USER_DATA_DIR` remain the overrides for setups this misses. The
  `/mnt/<drive>` probing assumes WSL's default automount root — a custom
  `[automount] root` in /etc/wsl.conf is not probed.
- Detection keys the data dir to the host arch, so an aarch64 WSL2 host (Windows
  on ARM) finds nothing in the x86\_64 Windows install; x86\_64 is the only
  verified WSL2 host arch.
- headless-test spawns the Windows exe (and the `%APPDATA%` query) through WSL
  interop, which registers at instance boot: a `.exe` spawn failing with "Exec
  format error" means interop is off or the instance predates the config edit
  — set `[interop] enabled=true` in /etc/wsl.conf and restart (`wsl
  --shutdown` from Windows).
- The self-test scratch dir crosses the boundary through a `WSLENV` `/p` bridge
  that headless-test sets itself: interop strips every env var WSLENV does not
  name, and `/p` translates the Linux path to its `\\wsl$` form. Self-test data
  never touches real play data.
- The mods-consent settings file needed by headless boots is on the Windows side
  too — see `verify.md` for the one-time enable.
- `release` and `decompile` shell out to the `zip` and `unzip` CLIs, which a
  minimal WSL distro may lack (Arch: `pacman -S zip unzip`).

## Release packaging

`cargo xtask release` packages the bundle under `dist/` (gitignored): one
`-universal` archive carrying every platform's library, plus one zip per row of
the cross matrix carrying only its own; a `SHA256SUMS` file sits next to the
zips. Every archive's root is the `spire-profiler/` folder, so extracting into
the mods directory is the whole install.
