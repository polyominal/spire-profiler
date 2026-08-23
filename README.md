# Spire Profiler

A per-source (cards, relics, powers, potions) combat profiler for Slay the Spire
2. During every combat the mod tracks what each source contributes, such as
damage, defense, and forge. It aggregates those numbers per run, and shows them
in two in-game charts: the combat panel and the run-history summary.

![Run Summary panel: per-source damage bars with a hover breakdown and category
legend](docs/images/run-summary.png)

## A minimal example

Playing **Bash, Dominate, Strike, Strike, Strike**:

![Combat panel after Bash, Dominate, and three Strikes: per-source damage bars
with Bash's hover breakdown](docs/images/minimal-example.png)

1. **Bash** hits for 8 direct damage, and applies 2 Vulnerable.
2. **Dominate** applies 1 more Vulnerable, and gives 3 Strength.
3. Each **Strike** hits for 13: 6 base + 3 Strength, then ×1.5 from Vulnerable
   (9 → 13). The mod re-runs the game's damage resolution per modifier: the
   additive Strength owns its 3, the multiplicative Vulnerable owns 9 × 0.5 =
   4. The 7 = 4 + 3 points are credited to:
   - Strength's 3 to Dominate
   - Vulnerable's 4 proportionally to stacks: Bash (2 of 3) gets 2, Dominate
     (last applier) gets the residue

Hence, after the five plays:

- Strike ×3: 18 = 3 × 6 direct
- Dominate ×1: 15 = 3 × (3 Strength + 2 Vulnerable) modifier
- Bash ×1: 14 = 8 direct + 3 × 2 Vulnerable modifier

The rows sum to the 47 behind "DPS 47.0 · 1 turns · 5 plays · took 0": every
point of damage is credited exactly once — the attacker keeps its base, and
each bonus goes to the source that applied it.

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
