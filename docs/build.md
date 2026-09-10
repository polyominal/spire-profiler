# Building the mod

Run `cargo xtask --help`.

## Host setup

- Supported build hosts are macOS, Linux, and WSL2. Native Windows is not a
  build host; use WSL2.
- `build` produces the complete pinned target matrix.
- Release packaging and its tests need the `zip` and `unzip` host CLIs.
- Host cargo tools use the pinned stable recorded in
  [main.rs](../xtask/src/main.rs), not the workspace nightly. Rustup by default
  exports the workspace nightly to child Cargo processes, and cargo-insta's
  locked rustix dependency does not compile there. Preserve the `+<stable>`
  prefix when copying an install command by hand.

## Formatting

- `cargo xtask fmt` formats Rust, all handwritten C\# under `shim/` (including
  fixtures), and the project Markdown docs. `cargo xtask fmt --check` fails on
  drift without rewriting sources; `cargo xtask fmt-md` formats only docs.
- C\# layout follows [shim/.editorconfig](../shim/.editorconfig) using the
  pinned SDK's `dotnet format whitespace` in folder mode. Formatting needs no
  game, generated project, or NuGet restore; the SDK bootstraps automatically.
  It does not apply analyzer fixes or format generated build sources.

## Cross-compilation

- The target matrix and `.gdextension` library keys come from the same table in
  [cross.rs](../xtask/src/cross.rs).
- Windows uses cargo-zigbuild because plain `zig cc` cannot link Rust's
  windows-gnu std. The resulting DLL runs on Windows 10 or newer.
- The Linux target triple's glibc suffix is the compatibility floor. It is the
  oldest distribution Steam still supports; raise it deliberately.
- macOS ships one thin library per architecture. Zig's bundled libSystem stubs
  link them without an Apple SDK.

## .NET bootstrap

- The handwritten C\# files under `shim/` are compiled from the production
  source inventory in [shim.rs](../xtask/src/shim.rs). Only
  `NativeLibrarySelector.g.cs` is generated from the native target matrix.
- The project uses explicit compile inputs, so stale files in
  `target/xtask-gen/` cannot join a later build. Managed tests use the same
  production inventory with fixture sources appended.
- Keep the SDK on 9.x while the shim targets net9.0. SDK 9 bundles that
  targeting pack; SDK 10 would silently fetch it from NuGet.
- Production builds and managed fixtures use the SDK's recommended .NET 9
  quality analyzers, pinned with `AnalysisLevel=9.0-recommended`; compiler and
  analyzer warnings fail both builds. Fixture-only exceptions preserve Harmony
  target shapes and keep expected arrays beside their assertions.
- xtask sets `DOTNET_ROOT` to the bootstrap directory while invoking the SDK. If
  running that binary by hand fails to find an SDK, check the inherited
  `DOTNET_ROOT`.

## WSL2

- A default Windows Steam install needs no environment overrides. Discovery
  probes `/mnt/<drive>` and headless-test uses Windows interop.
- An `Exec format error` while spawning a Windows executable means interop is
  disabled or the instance predates its configuration. Enable interop and
  restart the WSL instance.
