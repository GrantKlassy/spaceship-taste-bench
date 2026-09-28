# Nebula Drift

A real-time spaceship shooter that runs entirely in the terminal.

You fly a lone interceptor holding a collapsing sector. Waves of hostiles
descend; every fifth wave sends a capital ship. There is one hull, no continues,
and a high score to beat.

```
cargo run --release --locked
```

Best in a **124x69 terminal with 256-colour support**, which is what the layout
is tuned for. It adapts to any size down to 60x24 and handles live resizing.

---

## Controls

| Key | Action |
| --- | --- |
| `←` `→` `↑` `↓` or `W` `A` `S` `D` | Thrust. The ship carries momentum, so ease into turns. |
| `Space` | Fire |
| `X` | Bomb — clears all enemy fire and damages everything on screen |
| `F` | Toggle autofire (**on** by default) |
| `P` or `Esc` | Pause |
| `R` | Restart, once a run has ended |
| `Q` or `Ctrl-C` | Quit |

Autofire is on when you start, so you can ignore `Space` and concentrate on
flying. Turn it off with `F` if you would rather control your own shots.

---

## How it plays

**Staying alive.** Your **shield** takes the first hits and recharges a few
seconds after you stop being hit. Once it is down, hits cost **hull**, and hull
does not come back on its own — you have to find a repair. At zero hull the run
is over.

**Scoring.** Kills in quick succession build a chain. Every five links raises
your multiplier, up to **x8**. Taking a hull hit breaks the chain, so the
greedy line and the safe line are not the same line. Clearing a wave pays a
bonus scaled by the wave number and the hull you still have.

**Pickups** fall from tougher kills and drift toward you when you get close:

| | |
| --- | --- |
| **W** | Weapon up — five levels, each a wider spread |
| **S** | Shield recharge |
| **H** | Hull repair |
| **B** | Extra bomb (carry up to four) |
| **P** | Bonus points |

**The opposition.** Drifters weave downward and lob slow shots. Darts lock on
and dive — they do not shoot, they ram. Sentinels take up a firing line and
aim in bursts. Weavers sweep across raining spreads. Mines are harmless until
they burst, then throw a ring of fire in every direction. A **Warden** escorts
every third wave, and a **Dreadnought** arrives every fifth, fighting through
three escalating phases.

Watch for the pulsing halo around an enemy: that is the wind-up before it fires.

Your high score is kept in `$XDG_DATA_HOME/nebula-drift/highscore` (falling back
to `~/.local/share`). Nothing else is written, and the game never touches the
network.

---

## How it is drawn

The playfield is not characters — it is a **122x130 pixel framebuffer**.

Each terminal cell renders the half-block `▀` with the foreground painted as
the upper pixel and the background as the lower one, which buys twice the
vertical resolution. The game draws into a floating-point RGB buffer with
additive blending, sub-pixel splats and a bloom pass, then tone-maps and
quantises that to the xterm-256 palette.

Getting that palette to look good is most of the work. It has a fine 24-step
grey ramp but nothing at all between black and 95 per channel, so dim colours
have nowhere to land. Three things follow from that:

- **Dithering is scaled to the local palette spacing** rather than being a
  fixed amount, so dark bands and the grey ramp each get the right treatment
  instead of one being banded while the other turns to static.
- **The nebula is neutral by design.** A dim *coloured* cloud can only be drawn
  as a checkerboard of black and bright primaries; neutral dust rides the grey
  ramp and stays smooth. The colour in the sky comes from the stars.
- **Text is composited after the bloom, in exact palette colours**, so
  quantisation can never chew up a one-pixel glyph stroke or make it shimmer
  between two palette entries from frame to frame.

Only the cells that actually changed are repainted each frame. At 60 fps the
renderer costs about 0.45 ms per frame, with a p99 of 0.7 ms.

The terminal layer sits directly on `termios`, which is about all the game
needs: raw mode, the window size, and a byte stream to decode. Input uses the
terminal's keyboard-enhancement protocol for true key-release events where it
is available, and falls back to inferring held keys from auto-repeat timing
where it is not — with a long grace period before repeats start, and a short
one after, so holding a direction never stutters and letting go never drifts.
Support is detected passively, from the first release event that arrives, so
startup never waits on a query.

A kill signal restores the terminal through an async-signal-safe handler, and
so does a panic, so neither can leave you staring at a shell with no echo.

---

## Building

The only dependency is `libc`, and it is vendored under `vendor/`, so the build
works with no network access and no warm registry cache:

```
cargo build --release --locked
cargo test --release
```

Everything else — the renderer, the palette quantiser, the fonts, the escape
sequence decoder — is in `src/`.

### Diagnostics

```
cargo run --release --locked -- --help
cargo run --release --locked -- --bench          # frame cost and output size
cargo run --release --locked -- --snapshot DIR   # write PPM frames for inspection
```
