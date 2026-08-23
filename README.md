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
auto-fetches from `rust-toolchain.toml`, and the build bootstraps necessary
non-Rust tools.

```sh
cargo xtask build          # build cross-platform mod bundle
cargo xtask install-mod    # copy the bundle to the game's mods directory
```

Enable the mod in the game's mod settings, then play: F8 toggles the panel
(combat panel in play, run panel on the run-history screen) and clicking a
character avatar in a panel's header filters the chart to that player (click the
active avatar again for the full team view); the run-history screen gains its
own panel.

## Where things live

- The crate overview (architecture, layers, and the standing contracts) is the
  module doc at the top of `profiler-core/src/lib.rs`; each subsystem's spec
  sits in its own module doc next to the code.
- `docs/pitfalls.md`: environment traps (toolchain, headless testing,
  GDExtension interop, and platform layout).

## Roadmap

Deferred:

- Turn-by-turn timeline, per-card efficiency metrics, per-combat bars in the Run
  Summary tab, and a generated-count hover note.
