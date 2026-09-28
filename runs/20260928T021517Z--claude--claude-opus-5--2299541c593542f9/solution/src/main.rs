//! Nebula Drift - a terminal spaceship shooter.
//!
//! Renders into an RGB pixel buffer at twice the terminal's row resolution,
//! then packs pairs of rows into half-block characters and quantises to the
//! xterm-256 palette. The result is a 122x130 pixel display in a 124x69 cell
//! terminal.

mod canvas;
mod entities;
mod fx;
mod game;
mod hud;
mod rng;
mod sprites;
mod term;
mod tty;
mod waves;

use std::io::{BufWriter, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use game::{Game, Input, Mode};
use hud::Layout;
use term::{Cell, Screen};
use tty::{KeyCode, KeyEvent, KeyKind, Tty};

const TARGET_FPS: f32 = 60.0;

// ---------------------------------------------------------------------------
// Held-key tracking
// ---------------------------------------------------------------------------

/// Tracks whether a key is currently held.
///
/// When the terminal supports the keyboard enhancement protocol we get real
/// release events and the answer is exact. Otherwise we infer it from the
/// terminal's auto-repeat: the first press is given a long grace period to
/// cover the initial repeat delay, and once repeats start arriving the grace
/// period shrinks so releases feel responsive.
#[derive(Default, Clone, Copy)]
struct Hold {
    remaining: f32,
    last_press: f32,
    repeating: bool,
}

const GRACE_FIRST: f32 = 0.50;
const GRACE_REPEAT: f32 = 0.17;
const REPEAT_GAP: f32 = 0.14;
/// Safety net in precise mode in case a release event is ever lost.
const PRECISE_HOLD: f32 = 2.5;

impl Hold {
    fn press(&mut self, now: f32, precise: bool) {
        if precise {
            self.remaining = PRECISE_HOLD;
            return;
        }
        if self.last_press > 0.0 && now - self.last_press < REPEAT_GAP {
            self.repeating = true;
        }
        self.remaining = if self.repeating {
            GRACE_REPEAT
        } else {
            GRACE_FIRST
        };
        self.last_press = now;
    }

    fn release(&mut self) {
        self.remaining = 0.0;
        self.repeating = false;
    }

    fn tick(&mut self, dt: f32) {
        if self.remaining > 0.0 {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.remaining = 0.0;
                self.repeating = false;
            }
        }
    }

    fn down(&self) -> bool {
        self.remaining > 0.0
    }
}

#[derive(Default)]
struct Controls {
    left: Hold,
    right: Hold,
    up: Hold,
    down: Hold,
    fire: Hold,
    bomb_edge: bool,
    confirm_edge: bool,
    restart_edge: bool,
    pause_edge: bool,
    autofire_edge: bool,
    quit: bool,
    precise: bool,
}

impl Controls {
    fn tick(&mut self, dt: f32) {
        self.left.tick(dt);
        self.right.tick(dt);
        self.up.tick(dt);
        self.down.tick(dt);
        self.fire.tick(dt);
    }

    fn clear_edges(&mut self) {
        self.bomb_edge = false;
        self.confirm_edge = false;
        self.restart_edge = false;
        self.pause_edge = false;
        self.autofire_edge = false;
    }

    fn snapshot(&self) -> Input {
        Input {
            left: self.left.down(),
            right: self.right.down(),
            up: self.up.down(),
            down: self.down.down(),
            fire: self.fire.down(),
            bomb_edge: self.bomb_edge,
            confirm_edge: self.confirm_edge,
        }
    }

    fn handle_key(&mut self, k: KeyEvent, now: f32) {
        let pressed = matches!(k.kind, KeyKind::Press | KeyKind::Repeat);
        let released = k.kind == KeyKind::Release;
        // The first release event we ever see proves the terminal is reporting
        // key-up, so from then on held keys can be tracked exactly instead of
        // being inferred from auto-repeat timing.
        if released {
            self.precise = true;
        }

        if k.ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }

        // Directions and fire are held; everything else is an edge trigger.
        let held: Option<&mut Hold> = match k.code {
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => Some(&mut self.left),
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => Some(&mut self.right),
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') => Some(&mut self.up),
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => Some(&mut self.down),
            KeyCode::Char(' ') => Some(&mut self.fire),
            _ => None,
        };
        if let Some(h) = held {
            if pressed {
                h.press(now, self.precise);
            } else if released {
                h.release();
            }
            // Space also confirms on menu screens.
            if k.kind == KeyKind::Press && k.code == KeyCode::Char(' ') {
                self.confirm_edge = true;
            }
            return;
        }

        if k.kind != KeyKind::Press {
            return;
        }
        match k.code {
            KeyCode::Char('x') | KeyCode::Char('X') => self.bomb_edge = true,
            KeyCode::Char('p') | KeyCode::Char('P') | KeyCode::Esc => self.pause_edge = true,
            KeyCode::Char('r') | KeyCode::Char('R') => self.restart_edge = true,
            KeyCode::Char('f') | KeyCode::Char('F') => self.autofire_edge = true,
            KeyCode::Char('q') | KeyCode::Char('Q') => self.quit = true,
            KeyCode::Enter => self.confirm_edge = true,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal lifecycle
// ---------------------------------------------------------------------------

/// Install a panic hook that puts the terminal back before the message prints.
fn guard_terminal_on_panic() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tty::restore_global();
        default_hook(info);
    }));
}

// ---------------------------------------------------------------------------
// High score persistence
// ---------------------------------------------------------------------------

fn score_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })?;
    Some(base.join("nebula-drift").join("highscore"))
}

fn load_high() -> u64 {
    score_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn save_high(v: u64) {
    if let Some(p) = score_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, v.to_string());
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let dir = args.get(i + 1).cloned().unwrap_or_else(|| "snap".into());
        return match snapshot(&dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("snapshot failed: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--bench") {
        bench();
        return ExitCode::SUCCESS;
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nebula-drift: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "Nebula Drift - a terminal spaceship shooter\n\
         \n\
         USAGE:\n\
         \x20   cargo run --release --locked\n\
         \n\
         CONTROLS:\n\
         \x20   Arrows / WASD   thrust\n\
         \x20   Space           fire (autofire is on by default)\n\
         \x20   X               bomb\n\
         \x20   F               toggle autofire\n\
         \x20   P / Esc         pause\n\
         \x20   R               restart after a run ends\n\
         \x20   Q               quit\n\
         \n\
         OPTIONS:\n\
         \x20   --help          show this message\n\
         \x20   --bench         measure frame cost and output size, then exit\n\
         \x20   --snapshot DIR  write PPM frames for offline inspection\n\
         \n\
         Best played in a 124x69 terminal with 256 colour support.\n\
         The game needs no network access and stores only a high score under\n\
         $XDG_DATA_HOME/nebula-drift/highscore."
    );
}

fn run() -> std::io::Result<()> {
    guard_terminal_on_panic();

    let mut tty = Tty::enter()?;
    let mut out = BufWriter::with_capacity(1 << 19, std::io::stdout());

    let (mut cols, mut rows) = tty.size();
    let mut lay = Layout::new(cols.max(1), rows.max(1));
    let mut screen = Screen::new(cols.max(1), rows.max(1));
    let high = load_high();
    let mut g = Game::new(lay.pf_w.max(8), lay.pf_h().max(8), high);

    let mut controls = Controls::default();
    let mut events: Vec<KeyEvent> = Vec::with_capacity(64);

    let frame_budget = Duration::from_secs_f32(1.0 / TARGET_FPS);
    let start = Instant::now();
    let mut last = Instant::now();
    let mut saved_high = high;

    loop {
        // The window size is cheap to read, so poll it rather than wiring up
        // a SIGWINCH handler.
        let (c, r) = tty.size();
        if (c, r) != (cols, rows) {
            cols = c;
            rows = r;
            lay = Layout::new(cols.max(1), rows.max(1));
            screen.resize(cols.max(1), rows.max(1));
            if Layout::fits(cols, rows) {
                g.resize(lay.pf_w.max(8), lay.pf_h().max(8));
            }
        }

        events.clear();
        tty.poll(&mut events);
        let now_s = start.elapsed().as_secs_f32();
        for k in events.drain(..) {
            controls.handle_key(k, now_s);
        }

        if controls.quit {
            break;
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.1);
        last = now;
        controls.tick(dt);

        // Mode-level commands.
        if controls.pause_edge {
            g.toggle_pause();
        }
        if controls.autofire_edge {
            g.autofire = !g.autofire;
        }
        if controls.restart_edge && matches!(g.mode, Mode::GameOver | Mode::Title) {
            g.start_run();
        }

        let input = controls.snapshot();
        controls.clear_edges();

        if !Layout::fits(cols, rows) {
            hud::draw_too_small(&mut screen, cols, rows);
            screen.flush(&mut out)?;
            out.flush()?;
            std::thread::sleep(Duration::from_millis(80));
            continue;
        }

        g.update(dt, input);

        if g.high > saved_high {
            saved_high = g.high;
            save_high(saved_high);
        }

        g.render_world();
        let (ox, oy) = g.shake_offset();
        screen.clear(Cell::BLANK);
        hud::draw_chrome(&mut screen, &g, lay);
        screen.blit_offset(&g.canvas, lay.pf_col, lay.pf_row, ox, oy);
        screen.flush(&mut out)?;
        out.flush()?;

        let spent = Instant::now() - now;
        if spent < frame_budget {
            std::thread::sleep(frame_budget - spent);
        }
    }

    if g.high > saved_high {
        save_high(g.high);
    }
    out.write_all(b"\x1b[m")?;
    out.flush()?;
    drop(out);
    tty.restore();
    Ok(())
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

/// Drive the game with a scripted pilot and dump frames as binary PPM.
fn snapshot(dir: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let (cols, rows) = (124usize, 69usize);
    let lay = Layout::new(cols, rows);
    let mut screen = Screen::new(cols, rows);
    let mut g = Game::new(lay.pf_w, lay.pf_h(), 123_400);

    // (label, seconds to reach, still on the title screen?)
    let marks: [(&str, f32); 8] = [
        ("00-title", 1.5),
        ("01-wave1-banner", 2.4),
        ("02-wave1-combat", 8.0),
        ("03-wave2", 46.0),
        ("04-midgame", 95.0),
        ("05-boss", 175.0),
        ("06-late", 240.0),
        ("07-game-over", 244.0),
    ];
    // The pilot is kept alive until this point, then left to its fate.
    let die_at = 241.5f32;

    // A sheet of every sprite at rest, for checking the art in isolation.
    {
        let mut sheet = Game::new(lay.pf_w, lay.pf_h(), 0);
        sheet.mode = game::Mode::Play;
        sheet.banner = None;
        sheet.player.x = 16.0;
        sheet.player.y = 16.0;
        let kinds = [
            entities::Kind::Drifter,
            entities::Kind::Dart,
            entities::Kind::Mine,
            entities::Kind::Weaver,
            entities::Kind::Sentinel,
            entities::Kind::Warden,
            entities::Kind::Dreadnought,
        ];
        for (i, k) in kinds.iter().enumerate() {
            let mut e = entities::Enemy::new(*k, 0.0, 0.0, 1.0, &mut sheet.rng);
            e.x = 16.0 + (i % 3) as f32 * 40.0;
            e.y = 40.0 + (i / 3) as f32 * 30.0;
            if *k == entities::Kind::Dreadnought {
                e.x = 61.0;
                e.y = 108.0;
            }
            e.home_y = e.y;
            sheet.enemies.push(e);
        }
        for (i, b) in [
            entities::Boon::Weapon,
            entities::Boon::Shield,
            entities::Boon::Hull,
            entities::Boon::Bomb,
            entities::Boon::Points,
        ]
        .iter()
        .enumerate()
        {
            let mut p = entities::Pickup::new(0.0, 0.0, *b, &mut sheet.rng);
            p.x = 20.0 + i as f32 * 20.0;
            p.y = 16.0;
            sheet.pickups.push(p);
        }
        sheet.render_world();
        screen.clear(Cell::BLANK);
        hud::draw_chrome(&mut screen, &sheet, lay);
        screen.blit_offset(&sheet.canvas, lay.pf_col, lay.pf_row, 0, 0);
        let (w, h, px) = screen.to_rgb();
        let mut f = std::fs::File::create(format!("{dir}/sheet.ppm"))?;
        write!(f, "P6\n{w} {h}\n255\n")?;
        f.write_all(&px)?;
        eprintln!("{dir}/sheet.ppm");
    }

    // A scripted capital-ship encounter, which the autopilot rarely survives
    // long enough to reach on its own.
    {
        let mut boss = Game::new(lay.pf_w, lay.pf_h(), 0);
        boss.start_run();
        boss.jump_to_wave(5);
        boss.banner = None;
        let mut pilot = rng::Rng::new(31337);
        let mut bt = 0.0f32;
        let mut shot = 0;
        let marks = [14.0f32, 26.0];
        while shot < marks.len() {
            boss.player.hull = boss.player.max_hull;
            boss.player.weapon = 4;
            let phase = (bt * 0.9).sin();
            boss.update(
                1.0 / 60.0,
                Input {
                    left: phase < -0.2,
                    right: phase > 0.2,
                    fire: true,
                    up: pilot.chance(0.05),
                    ..Default::default()
                },
            );
            bt += 1.0 / 60.0;
            if bt >= marks[shot] {
                boss.render_world();
                let (ox, oy) = boss.shake_offset();
                screen.clear(Cell::BLANK);
                hud::draw_chrome(&mut screen, &boss, lay);
                screen.blit_offset(&boss.canvas, lay.pf_col, lay.pf_row, ox, oy);
                let (w, h, px) = screen.to_rgb();
                let name = format!("{dir}/boss-{shot}");
                let mut f = std::fs::File::create(format!("{name}.ppm"))?;
                write!(f, "P6\n{w} {h}\n255\n")?;
                f.write_all(&px)?;
                std::fs::write(format!("{name}.txt"), screen.to_text())?;
                eprintln!("{name}.ppm  enemies={}", boss.enemies.len());
                shot += 1;
            }
        }
    }

    let dt = 1.0 / 60.0;
    let mut t = 0.0f32;
    let mut started = false;
    let mut pilot = rng::Rng::new(20_260_928);
    let mut next = 0usize;

    while next < marks.len() {
        // A simple autopilot: weave, dodge downward pressure, bomb rarely.
        let input = if !started {
            Input::default()
        } else {
            let phase = (t * 0.7).sin();
            Input {
                left: phase < -0.2,
                right: phase > 0.2,
                up: pilot.chance(0.10),
                down: pilot.chance(0.08),
                fire: true,
                bomb_edge: (t * 60.0) as u32 % 1200 == 0,
                confirm_edge: false,
            }
        };
        g.update(dt, input);
        t += dt;

        if !started && t > 1.6 {
            g.start_run();
            started = true;
        }
        // Keep the pilot alive so later marks show real combat.
        if g.mode == Mode::GameOver && t < die_at {
            g.start_run();
        }
        if started && t < die_at {
            g.player.hull = g.player.max_hull;
        }
        if started && t >= die_at && g.player.alive {
            g.player.shield = 0.0;
            g.player.invuln = 0.0;
            g.player.hull = 1;
        }

        if t >= marks[next].1 {
            g.render_world();
            let (ox, oy) = g.shake_offset();
            screen.clear(Cell::BLANK);
            hud::draw_chrome(&mut screen, &g, lay);
            screen.blit_offset(&g.canvas, lay.pf_col, lay.pf_row, ox, oy);
            let (w, h, px) = screen.to_rgb();
            let path = format!("{dir}/{}.ppm", marks[next].0);
            let mut f = std::fs::File::create(&path)?;
            write!(f, "P6\n{w} {h}\n255\n")?;
            f.write_all(&px)?;
            let txt = format!("{dir}/{}.txt", marks[next].0);
            std::fs::write(txt, screen.to_text())?;
            eprintln!(
                "{path}  t={:.1}s wave={} enemies={} parts={}",
                t,
                g.wave.number,
                g.enemies.len(),
                g.fx.count()
            );
            next += 1;
        }
    }
    Ok(())
}

/// Measure simulation, render and serialisation cost.
fn bench() {
    let (cols, rows) = (124usize, 69usize);
    let lay = Layout::new(cols, rows);
    let mut screen = Screen::new(cols, rows);
    let mut g = Game::new(lay.pf_w, lay.pf_h(), 0);
    g.start_run();
    let mut sink: Vec<u8> = Vec::with_capacity(1 << 20);

    let dt = 1.0 / 60.0;
    let frames = 1200;
    // Warm up so the first wave is populated.
    for _ in 0..240 {
        g.update(dt, Input { fire: true, ..Default::default() });
    }

    let mut times: Vec<f32> = Vec::with_capacity(frames);
    let mut bytes_total = 0usize;
    let mut bytes_worst = 0usize;
    let t0 = Instant::now();
    let mut pilot = rng::Rng::new(7);
    for i in 0..frames {
        let f0 = Instant::now();
        g.player.hull = g.player.max_hull;
        g.update(
            dt,
            Input {
                left: pilot.chance(0.3),
                right: pilot.chance(0.3),
                fire: true,
                bomb_edge: i % 300 == 0,
                ..Default::default()
            },
        );
        g.render_world();
        let (ox, oy) = g.shake_offset();
        screen.clear(Cell::BLANK);
        hud::draw_chrome(&mut screen, &g, lay);
        screen.blit_offset(&g.canvas, lay.pf_col, lay.pf_row, ox, oy);
        sink.clear();
        let n = screen.flush(&mut sink).unwrap();
        bytes_total += n;
        bytes_worst = bytes_worst.max(n);
        times.push((Instant::now() - f0).as_secs_f32() * 1000.0);
    }
    let total = (Instant::now() - t0).as_secs_f32();
    times.sort_by(f32::total_cmp);
    let pct = |p: f32| times[((times.len() as f32 - 1.0) * p) as usize];
    println!("frames         {frames}");
    println!("mean frame     {:.3} ms", total / frames as f32 * 1000.0);
    println!("median frame   {:.3} ms", pct(0.50));
    println!("p99 frame      {:.3} ms", pct(0.99));
    println!("worst frame    {:.3} ms", times[times.len() - 1]);
    println!("mean output    {} bytes", bytes_total / frames);
    println!("worst output   {bytes_worst} bytes");
    println!(
        "mean bandwidth {:.2} MB/s at 60 fps",
        bytes_total as f32 / frames as f32 * 60.0 / 1e6
    );
    println!("wave reached   {}", g.wave.number);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, kind: KeyKind) -> KeyEvent {
        KeyEvent {
            code,
            kind,
            ctrl: false,
        }
    }

    #[test]
    fn precise_mode_tracks_press_and_release_exactly() {
        let mut c = Controls {
            precise: true,
            ..Default::default()
        };
        c.handle_key(key(KeyCode::Right, KeyKind::Press), 0.0);
        assert!(c.snapshot().right);
        // Still held a full second later, with no repeats at all.
        for _ in 0..60 {
            c.tick(1.0 / 60.0);
        }
        assert!(c.snapshot().right, "a held key must not decay in precise mode");
        c.handle_key(key(KeyCode::Right, KeyKind::Release), 1.0);
        assert!(!c.snapshot().right, "release must stop thrust immediately");
    }

    #[test]
    fn a_release_event_switches_the_input_layer_to_precise_mode() {
        let mut c = Controls::default();
        assert!(!c.precise);
        c.handle_key(key(KeyCode::Left, KeyKind::Release), 0.0);
        assert!(c.precise, "seeing a key-up proves the terminal reports them");
    }

    #[test]
    fn fallback_mode_holds_through_the_auto_repeat_delay() {
        let mut c = Controls::default();
        c.handle_key(key(KeyCode::Char('d'), KeyKind::Press), 0.0);
        // The terminal's initial repeat delay is typically ~0.4s. The key must
        // still read as held across that gap, or the ship would stutter.
        let mut t = 0.0f32;
        for _ in 0..24 {
            c.tick(1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        assert!(c.snapshot().right, "stalled after {t:.2}s of repeat delay");
    }

    #[test]
    fn fallback_mode_releases_promptly_once_repeats_are_flowing() {
        let mut c = Controls::default();
        let mut t = 0.0f32;
        // Simulate a stream of auto-repeats at 30ms.
        for _ in 0..12 {
            c.handle_key(key(KeyCode::Char('d'), KeyKind::Press), t);
            for _ in 0..2 {
                c.tick(0.015);
                t += 0.015;
            }
        }
        assert!(c.snapshot().right);
        // Repeats stop: the key should let go within the short grace period,
        // not the long one used before repeats were observed.
        let mut held_for = 0.0f32;
        while c.snapshot().right && held_for < 1.0 {
            c.tick(1.0 / 60.0);
            held_for += 1.0 / 60.0;
        }
        assert!(
            held_for < GRACE_FIRST,
            "hung on for {held_for:.2}s after repeats stopped"
        );
    }

    #[test]
    fn a_lost_release_event_cannot_stick_a_key_forever() {
        let mut c = Controls {
            precise: true,
            ..Default::default()
        };
        c.handle_key(key(KeyCode::Up, KeyKind::Press), 0.0);
        let mut t = 0.0f32;
        while c.snapshot().up && t < 10.0 {
            c.tick(1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        assert!(t < 10.0, "a dropped key-up left the key stuck down");
    }

    #[test]
    fn edge_triggers_fire_once_per_press() {
        let mut c = Controls::default();
        c.handle_key(key(KeyCode::Char('x'), KeyKind::Press), 0.0);
        assert!(c.snapshot().bomb_edge);
        c.clear_edges();
        assert!(!c.snapshot().bomb_edge, "bomb must not repeat while held");
        // Auto-repeat of the same key must not drop another bomb either.
        c.handle_key(key(KeyCode::Char('x'), KeyKind::Repeat), 0.05);
        assert!(!c.snapshot().bomb_edge);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut c = Controls::default();
        c.handle_key(
            KeyEvent {
                code: KeyCode::Char('c'),
                kind: KeyKind::Press,
                ctrl: true,
            },
            0.0,
        );
        assert!(c.quit);
    }

    #[test]
    fn plain_c_does_not_quit() {
        let mut c = Controls::default();
        c.handle_key(key(KeyCode::Char('c'), KeyKind::Press), 0.0);
        assert!(!c.quit);
    }

    #[test]
    fn both_arrow_and_wasd_bindings_drive_the_same_axis() {
        for (a, b) in [
            (KeyCode::Left, KeyCode::Char('a')),
            (KeyCode::Right, KeyCode::Char('d')),
            (KeyCode::Up, KeyCode::Char('w')),
            (KeyCode::Down, KeyCode::Char('s')),
        ] {
            let mut ca = Controls::default();
            ca.handle_key(key(a, KeyKind::Press), 0.0);
            let mut cb = Controls::default();
            cb.handle_key(key(b, KeyKind::Press), 0.0);
            let (sa, sb) = (ca.snapshot(), cb.snapshot());
            assert_eq!(
                (sa.left, sa.right, sa.up, sa.down),
                (sb.left, sb.right, sb.up, sb.down),
                "{a:?} and {b:?} disagree"
            );
        }
    }
}
