# VESPER

*A small ship. Three guardians. One way through.*

A complete terminal arcade game in Rust: luminous starfields, automatic twin
lances, telegraphed guardian attacks, collectible salvage, and a choice of ship
upgrades. Cross three sectors and bring the light home. A voyage takes about
three to five minutes; every new flight starts fresh.

```sh
cargo run --release --locked
```

Use a **UTF-8, 256-color terminal**. Designed for **124 columns × 69 rows**;
minimum 120 × 65. All dependencies are included in `vendor/`, and Cargo is
configured for offline builds. Only Rust, Cargo, and the usual Linux linker
are needed. No downloads, graphical window, external assets, or services.

| Key | Action |
| --- | --- |
| Enter | Launch; fly again after a victory or loss |
| WASD or arrow keys | Move; your lances fire automatically |
| Space | Phase drive: 0.85 seconds of immunity and increased speed |
| X | Nova at 100% charge: erase hostile bullets and damage all enemies |
| P / Esc | Pause or resume |
| 1 / 2 / 3 | Choose a ship upgrade after defeating a guardian |
| R | Retry from the result screen |
| Q | Open pause during flight; quit from menus |
| Ctrl-C | Quit immediately |

Keep moving and line up your shots. Orange dots are hostile fire; a flashing
gold line warns of a guardian beam. Phase through danger or escape with a
nova. Gold `◇` salvage adds score and charge; green `+` repairs restore hull.
Nearby pickups pull toward your ship. Consecutive kills build a multiplier
up to ×5; taking a hit or waiting four seconds breaks the chain.

Every refit repairs 35 hull and adds 30% nova charge. Choose more firepower,
more hull, or a faster phase drive. The final guardian guards the way home.

The game pauses when resized or when a supporting terminal loses focus.
Modern terminals with the Kitty keyboard protocol support exact held-key
movement; other terminals use key repeat and short directional taps.
Personal bests are saved to `$XDG_STATE_HOME/vesper/best`, or
`~/.local/state/vesper/best`. A read-only home directory is fine: the game
still runs and keeps the best score for the current session.

Run the simulation, input, and rendering checks with:

```sh
cargo test --locked
```

Original game source: MIT license. Vendored dependencies retain their own
licenses in their respective directories.
