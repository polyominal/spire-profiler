# Spire Profiler

A per-source (cards, relics, powers, potions) combat profiler for Slay the Spire
2. During every combat the mod tracks what each source contributes, such as
damage, defense, and forge. It aggregates those numbers per run, and shows them
in two in-game charts: the combat panel and the run-history summary.

![Run Summary panel: per-source damage bars with a hover breakdown and category
legend](docs/images/run-summary.png)

## AI use disclaimer

This project is being built with heavy use of LLMs. The code is reviewed by
humans, and the quality standard should be set by humans rather than by
generated output.

## Quick start

Prerequisites: a Steam install of Slay the Spire 2. The pinned Rust nightly
auto-fetches from [rust-toolchain.toml](rust-toolchain.toml), and the build
bootstraps necessary non-Rust tools.

```sh
cargo xtask build          # build cross-platform mod bundle
cargo xtask install-mod    # copy the bundle to the game's mods directory
```

Enable the mod in the game's mod settings, then play: F8 toggles the panel
(combat panel in play, run panel on the run-history screen), and clicking a
character avatar in a panel's header filters the chart to that player (click the
active avatar again for the full team view).

## Where things live

- Specs live in module docs: the crate overview (architecture, layers, standing
  contracts) at the top of [lib.rs](profiler-core/src/lib.rs), each subsystem's
  spec next to its code.
- `docs/` holds the environment guides: building ([build.md](docs/build.md)),
  verification gates and headless testing ([verify.md](docs/verify.md)),
  GDExtension interop ([gdextension.md](docs/gdextension.md)), and the game
  environment ([game.md](docs/game.md)).
