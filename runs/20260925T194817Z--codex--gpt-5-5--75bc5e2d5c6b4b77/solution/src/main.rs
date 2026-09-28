use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use crossterm::{execute, queue};

const GAME_W: usize = 84;
const GAME_H: usize = 58;
const HUD_W: usize = 34;
const TARGET_FRAME: Duration = Duration::from_millis(33);

fn main() -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let result = run(&mut terminal.stdout);
    terminal.leave()?;
    result
}

fn run(stdout: &mut Stdout) -> io::Result<()> {
    let mut game = Game::new();
    let mut last = Instant::now();
    let mut accumulator = 0.0_f32;
    let fixed_dt = 1.0 / 30.0;

    loop {
        let now = Instant::now();
        let frame_dt = (now - last).as_secs_f32().min(0.1);
        last = now;
        accumulator += frame_dt;

        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                    if game.handle_key(key) {
                        return Ok(());
                    }
                }
            }
        }

        while accumulator >= fixed_dt {
            game.update(fixed_dt);
            accumulator -= fixed_dt;
        }

        game.draw(stdout)?;

        let elapsed = now.elapsed();
        if elapsed < TARGET_FRAME {
            std::thread::sleep(TARGET_FRAME - elapsed);
        }
    }
}

struct Terminal {
    stdout: Stdout,
    active: bool,
}

impl Terminal {
    fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
        Ok(Self {
            stdout,
            active: true,
        })
    }

    fn leave(&mut self) -> io::Result<()> {
        if self.active {
            execute!(self.stdout, ResetColor, Show, LeaveAlternateScreen)?;
            disable_raw_mode()?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}

impl Cell {
    fn new(ch: char, fg: Color, bg: Color) -> Self {
        Self { ch, fg, bg }
    }
}

struct Canvas {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            cells: vec![Cell::new(' ', Color::White, Color::Black); w * h],
        }
    }

    fn clear(&mut self, bg: Color) {
        self.cells.fill(Cell::new(' ', Color::White, bg));
    }

    fn set(&mut self, x: i32, y: i32, ch: char, fg: Color) {
        self.set_bg(x, y, ch, fg, Color::Black);
    }

    fn set_bg(&mut self, x: i32, y: i32, ch: char, fg: Color, bg: Color) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.cells[y as usize * self.w + x as usize] = Cell::new(ch, fg, bg);
        }
    }

    fn text(&mut self, x: i32, y: i32, text: &str, fg: Color) {
        for (offset, ch) in text.chars().enumerate() {
            self.set(x + offset as i32, y, ch, fg);
        }
    }

    fn line_h(&mut self, x: i32, y: i32, len: i32, ch: char, fg: Color) {
        for i in 0..len {
            self.set(x + i, y, ch, fg);
        }
    }

    fn line_v(&mut self, x: i32, y: i32, len: i32, ch: char, fg: Color) {
        for i in 0..len {
            self.set(x, y + i, ch, fg);
        }
    }

    fn flush(&self, stdout: &mut Stdout) -> io::Result<()> {
        queue!(stdout, MoveTo(0, 0))?;
        let mut fg = Color::Reset;
        let mut bg = Color::Reset;
        for y in 0..self.h {
            for x in 0..self.w {
                let cell = self.cells[y * self.w + x];
                if cell.fg != fg {
                    queue!(stdout, SetForegroundColor(cell.fg))?;
                    fg = cell.fg;
                }
                if cell.bg != bg {
                    queue!(stdout, SetBackgroundColor(cell.bg))?;
                    bg = cell.bg;
                }
                queue!(stdout, Print(cell.ch))?;
            }
            if y + 1 < self.h {
                queue!(stdout, Print("\r\n"))?;
            }
        }
        queue!(stdout, ResetColor)?;
        stdout.flush()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Title,
    Running,
    Paused,
    GameOver,
}

#[derive(Default)]
struct Input {
    up: f32,
    down: f32,
    left: f32,
    right: f32,
    fire: bool,
}

struct Game {
    mode: Mode,
    canvas: Canvas,
    rng: Rng,
    input: Input,
    player: Player,
    bullets: Vec<Shot>,
    enemies: Vec<Enemy>,
    particles: Vec<Particle>,
    pickups: Vec<Pickup>,
    stars: Vec<Star>,
    score: u32,
    best: u32,
    wave: u32,
    spawn_timer: f32,
    pickup_timer: f32,
    time: f32,
    message_timer: f32,
}

impl Game {
    fn new() -> Self {
        let mut rng = Rng::new(0x5EED_57A9_u64);
        let mut stars = Vec::new();
        for _ in 0..220 {
            stars.push(Star {
                x: rng.range_f32(1.0, (GAME_W - 2) as f32),
                y: rng.range_f32(1.0, (GAME_H - 2) as f32),
                speed: rng.range_f32(4.0, 22.0),
                glyph: if rng.chance(0.72) { '.' } else { '·' },
                color: if rng.chance(0.65) {
                    Color::DarkGrey
                } else {
                    Color::Grey
                },
            });
        }
        Self {
            mode: Mode::Title,
            canvas: Canvas::new(GAME_W + HUD_W + 3, GAME_H + 2),
            rng,
            input: Input::default(),
            player: Player::new(),
            bullets: Vec::new(),
            enemies: Vec::new(),
            particles: Vec::new(),
            pickups: Vec::new(),
            stars,
            score: 0,
            best: 0,
            wave: 1,
            spawn_timer: 0.0,
            pickup_timer: 7.0,
            time: 0.0,
            message_timer: 0.0,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char(' ') | KeyCode::Enter => match self.mode {
                Mode::Title | Mode::GameOver => self.reset_run(),
                Mode::Running => self.input.fire = true,
                Mode::Paused => self.mode = Mode::Running,
            },
            KeyCode::Char('p') => {
                self.mode = match self.mode {
                    Mode::Running => Mode::Paused,
                    Mode::Paused => Mode::Running,
                    other => other,
                };
            }
            KeyCode::Char('r') if self.mode == Mode::GameOver => self.reset_run(),
            KeyCode::Up | KeyCode::Char('w') => self.input.up = 0.13,
            KeyCode::Down | KeyCode::Char('s') => self.input.down = 0.13,
            KeyCode::Left | KeyCode::Char('a') => self.input.left = 0.13,
            KeyCode::Right | KeyCode::Char('d') => self.input.right = 0.13,
            KeyCode::Char('z') | KeyCode::Char('j') => self.input.fire = true,
            _ => {}
        }
        false
    }

    fn reset_run(&mut self) {
        self.mode = Mode::Running;
        self.player = Player::new();
        self.bullets.clear();
        self.enemies.clear();
        self.particles.clear();
        self.pickups.clear();
        self.score = 0;
        self.wave = 1;
        self.spawn_timer = 0.25;
        self.pickup_timer = 7.0;
        self.time = 0.0;
        self.message_timer = 2.5;
    }

    fn update(&mut self, dt: f32) {
        self.update_stars(dt);
        if self.mode != Mode::Running {
            self.input = Input::default();
            return;
        }

        self.time += dt;
        self.wave = 1 + (self.time / 24.0) as u32;
        self.message_timer = (self.message_timer - dt).max(0.0);
        self.update_player(dt);
        self.input.up = (self.input.up - dt).max(0.0);
        self.input.down = (self.input.down - dt).max(0.0);
        self.input.left = (self.input.left - dt).max(0.0);
        self.input.right = (self.input.right - dt).max(0.0);
        self.update_bullets(dt);
        self.update_enemies(dt);
        self.update_pickups(dt);
        self.update_particles(dt);
        self.spawn_timer -= dt;
        self.pickup_timer -= dt;

        if self.spawn_timer <= 0.0 {
            self.spawn_enemy();
            let pressure = (self.wave as f32 * 0.035).min(0.42);
            self.spawn_timer = self.rng.range_f32(0.34, 0.85) - pressure;
        }

        if self.pickup_timer <= 0.0 {
            self.spawn_pickup();
            self.pickup_timer = self.rng.range_f32(8.0, 13.5);
        }

        self.resolve_collisions();
        self.cleanup();
        self.input.fire = false;
    }

    fn update_stars(&mut self, dt: f32) {
        for star in &mut self.stars {
            star.y += star.speed * dt;
            if star.y > (GAME_H - 2) as f32 {
                star.y = 1.0;
                star.x = self.rng.range_f32(1.0, (GAME_W - 2) as f32);
            }
        }
    }

    fn update_player(&mut self, dt: f32) {
        let mut dx: f32 = 0.0;
        let mut dy: f32 = 0.0;
        if self.input.left > 0.0 {
            dx -= 1.0;
        }
        if self.input.right > 0.0 {
            dx += 1.0;
        }
        if self.input.up > 0.0 {
            dy -= 1.0;
        }
        if self.input.down > 0.0 {
            dy += 1.0;
        }
        if dx != 0.0 || dy != 0.0 {
            let len = (dx * dx + dy * dy).sqrt();
            self.player.x += dx / len * self.player.speed * dt;
            self.player.y += dy / len * self.player.speed * dt;
        }
        self.player.x = self.player.x.clamp(3.0, (GAME_W - 4) as f32);
        self.player.y = self.player.y.clamp(5.0, (GAME_H - 4) as f32);

        self.player.fire_cooldown = (self.player.fire_cooldown - dt).max(0.0);
        self.player.invuln = (self.player.invuln - dt).max(0.0);

        if self.input.fire && self.player.fire_cooldown <= 0.0 {
            self.bullets.push(Shot {
                x: self.player.x - 1.0,
                y: self.player.y - 1.0,
                dy: -64.0,
                friendly: true,
                damage: 1,
            });
            self.bullets.push(Shot {
                x: self.player.x + 1.0,
                y: self.player.y - 1.0,
                dy: -64.0,
                friendly: true,
                damage: 1,
            });
            self.player.fire_cooldown = if self.player.overdrive > 0.0 { 0.085 } else { 0.14 };
        }
        self.player.overdrive = (self.player.overdrive - dt).max(0.0);
    }

    fn update_bullets(&mut self, dt: f32) {
        for bullet in &mut self.bullets {
            bullet.y += bullet.dy * dt;
        }
    }

    fn update_enemies(&mut self, dt: f32) {
        let px = self.player.x;
        for enemy in &mut self.enemies {
            enemy.y += enemy.vy * dt;
            enemy.x += (enemy.phase + self.time * enemy.sway).sin() * enemy.drift * dt;
            enemy.cooldown -= dt;
            if enemy.kind == EnemyKind::Raider && enemy.cooldown <= 0.0 && enemy.y > 3.0 {
                let aim = (px - enemy.x).clamp(-12.0, 12.0) * 0.12;
                self.bullets.push(Shot {
                    x: enemy.x,
                    y: enemy.y + 1.0,
                    dy: 34.0 + aim.abs(),
                    friendly: false,
                    damage: 1,
                });
                enemy.cooldown = self.rng.range_f32(1.3, 2.5);
            }
        }
    }

    fn update_pickups(&mut self, dt: f32) {
        for pickup in &mut self.pickups {
            pickup.y += 18.0 * dt;
            pickup.phase += dt * 6.0;
        }
    }

    fn update_particles(&mut self, dt: f32) {
        for particle in &mut self.particles {
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;
            particle.life -= dt;
        }
    }

    fn spawn_enemy(&mut self) {
        let roll = self.rng.next_f32();
        let kind = if self.wave >= 3 && roll > 0.72 {
            EnemyKind::Raider
        } else if roll > 0.45 {
            EnemyKind::Comet
        } else {
            EnemyKind::Shard
        };

        let (hp, vy, radius, glyph, color) = match kind {
            EnemyKind::Shard => (1, self.rng.range_f32(18.0, 30.0), 1.15, '◆', Color::Yellow),
            EnemyKind::Comet => (2, self.rng.range_f32(11.0, 19.0), 1.75, '●', Color::Red),
            EnemyKind::Raider => (3, self.rng.range_f32(8.0, 13.0), 1.55, '▼', Color::Magenta),
        };
        self.enemies.push(Enemy {
            x: self.rng.range_f32(4.0, (GAME_W - 5) as f32),
            y: 2.0,
            vy: vy + self.wave as f32 * 1.4,
            hp,
            radius,
            glyph,
            color,
            kind,
            drift: self.rng.range_f32(1.5, 8.0),
            sway: self.rng.range_f32(1.2, 3.4),
            phase: self.rng.range_f32(0.0, 6.28),
            cooldown: self.rng.range_f32(0.9, 2.4),
        });
    }

    fn spawn_pickup(&mut self) {
        self.pickups.push(Pickup {
            x: self.rng.range_f32(5.0, (GAME_W - 6) as f32),
            y: 2.0,
            phase: 0.0,
        });
    }

    fn resolve_collisions(&mut self) {
        let mut score_gain = 0;
        let mut explosions = Vec::new();

        for bullet in &mut self.bullets {
            if !bullet.friendly {
                continue;
            }
            for enemy in &mut self.enemies {
                if enemy.hp <= 0 {
                    continue;
                }
                if dist2(bullet.x, bullet.y, enemy.x, enemy.y) <= enemy.radius * enemy.radius {
                    enemy.hp -= bullet.damage;
                    bullet.y = -100.0;
                    if enemy.hp <= 0 {
                        score_gain += match enemy.kind {
                            EnemyKind::Shard => 25,
                            EnemyKind::Comet => 55,
                            EnemyKind::Raider => 90,
                        };
                        explosions.push((enemy.x, enemy.y, enemy.color));
                    }
                    break;
                }
            }
        }

        for (x, y, color) in explosions {
            self.burst(x, y, color, 13);
        }
        self.score += score_gain;
        self.best = self.best.max(self.score);

        if self.player.invuln <= 0.0 {
            let mut hit = false;
            let mut impact = None;
            for enemy in &mut self.enemies {
                if enemy.hp > 0 && dist2(self.player.x, self.player.y, enemy.x, enemy.y) <= 4.0 {
                    enemy.hp = 0;
                    hit = true;
                    impact = Some((enemy.x, enemy.y));
                    break;
                }
            }
            if let Some((x, y)) = impact {
                self.burst(x, y, Color::Red, 16);
            }
            for bullet in &mut self.bullets {
                if !bullet.friendly && dist2(self.player.x, self.player.y, bullet.x, bullet.y) <= 2.25 {
                    bullet.y = 999.0;
                    hit = true;
                    break;
                }
            }
            if hit {
                self.damage_player();
            }
        }

        let mut collected = 0;
        self.pickups.retain(|pickup| {
            if dist2(self.player.x, self.player.y, pickup.x, pickup.y) <= 5.0 {
                collected += 1;
                false
            } else {
                true
            }
        });
        for _ in 0..collected {
            self.player.shields = (self.player.shields + 1).min(5);
            self.player.overdrive = 5.0;
            self.score += 75;
            self.burst(self.player.x, self.player.y, Color::Cyan, 18);
        }
    }

    fn damage_player(&mut self) {
        self.player.shields -= 1;
        self.player.invuln = 1.4;
        self.player.overdrive = 1.8;
        self.burst(self.player.x, self.player.y, Color::White, 24);
        if self.player.shields < 0 {
            self.mode = Mode::GameOver;
            self.best = self.best.max(self.score);
            self.burst(self.player.x, self.player.y, Color::Red, 42);
        }
    }

    fn burst(&mut self, x: f32, y: f32, color: Color, count: usize) {
        for _ in 0..count {
            let angle = self.rng.range_f32(0.0, 6.283);
            let speed = self.rng.range_f32(8.0, 36.0);
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                life: self.rng.range_f32(0.22, 0.72),
                color,
            });
        }
    }

    fn cleanup(&mut self) {
        self.bullets.retain(|b| b.y > 0.0 && b.y < GAME_H as f32);
        self.enemies
            .retain(|e| e.hp > 0 && e.y < (GAME_H + 3) as f32 && e.x > -4.0 && e.x < (GAME_W + 4) as f32);
        self.pickups.retain(|p| p.y < (GAME_H - 1) as f32);
        self.particles.retain(|p| p.life > 0.0);
    }

    fn draw(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        let (tw, th) = terminal::size().unwrap_or((0, 0));
        if tw < self.canvas.w as u16 || th < self.canvas.h as u16 {
            self.draw_too_small(stdout, tw, th)
        } else {
            self.draw_game(stdout)
        }
    }

    fn draw_too_small(&mut self, stdout: &mut Stdout, tw: u16, th: u16) -> io::Result<()> {
        queue!(stdout, Clear(ClearType::All), MoveTo(0, 0), SetForegroundColor(Color::White))?;
        queue!(
            stdout,
            Print(format!(
                "Starwake needs at least {}x{} cells. Current terminal: {}x{}.\r\nResize, then press any key.",
                self.canvas.w, self.canvas.h, tw, th
            )),
            ResetColor
        )?;
        stdout.flush()
    }

    fn draw_game(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        self.canvas.clear(Color::Black);
        self.draw_frame();
        self.draw_stars();
        self.draw_entities();
        self.draw_hud();
        self.draw_overlay();
        self.canvas.flush(stdout)
    }

    fn draw_frame(&mut self) {
        let w = GAME_W as i32;
        let h = GAME_H as i32;
        let frame = Color::DarkCyan;
        self.canvas.set(0, 0, '╔', frame);
        self.canvas.set(w + 1, 0, '╗', frame);
        self.canvas.set(0, h + 1, '╚', frame);
        self.canvas.set(w + 1, h + 1, '╝', frame);
        self.canvas.line_h(1, 0, w, '═', frame);
        self.canvas.line_h(1, h + 1, w, '═', frame);
        self.canvas.line_v(0, 1, h, '║', frame);
        self.canvas.line_v(w + 1, 1, h, '║', frame);
        self.canvas.line_v(w + 3, 0, h + 2, '│', Color::DarkGrey);
    }

    fn draw_stars(&mut self) {
        for star in &self.stars {
            self.canvas
                .set(star.x.round() as i32 + 1, star.y.round() as i32 + 1, star.glyph, star.color);
        }
    }

    fn draw_entities(&mut self) {
        for pickup in &self.pickups {
            let ch = if pickup.phase.sin() > 0.0 { '✦' } else { '+' };
            self.canvas
                .set(pickup.x.round() as i32 + 1, pickup.y.round() as i32 + 1, ch, Color::Cyan);
        }
        for bullet in &self.bullets {
            let ch = if bullet.friendly { '┃' } else { '!' };
            let color = if bullet.friendly { Color::Cyan } else { Color::Red };
            self.canvas
                .set(bullet.x.round() as i32 + 1, bullet.y.round() as i32 + 1, ch, color);
        }
        for enemy in &self.enemies {
            self.canvas
                .set(enemy.x.round() as i32 + 1, enemy.y.round() as i32 + 1, enemy.glyph, enemy.color);
            if enemy.hp > 1 {
                self.canvas
                    .set(enemy.x.round() as i32, enemy.y.round() as i32 + 1, '·', Color::DarkRed);
                self.canvas
                    .set(enemy.x.round() as i32 + 2, enemy.y.round() as i32 + 1, '·', Color::DarkRed);
            }
        }
        for particle in &self.particles {
            self.canvas
                .set(particle.x.round() as i32 + 1, particle.y.round() as i32 + 1, '·', particle.color);
        }
        if self.mode != Mode::GameOver || (self.time * 14.0).sin() > -0.2 {
            let color = if self.player.invuln > 0.0 {
                Color::White
            } else if self.player.overdrive > 0.0 {
                Color::Cyan
            } else {
                Color::Green
            };
            let x = self.player.x.round() as i32 + 1;
            let y = self.player.y.round() as i32 + 1;
            self.canvas.set(x, y - 1, '▲', color);
            self.canvas.set(x - 1, y, '◀', color);
            self.canvas.set(x, y, '█', color);
            self.canvas.set(x + 1, y, '▶', color);
            self.canvas.set(x - 1, y + 1, '╱', Color::DarkGrey);
            self.canvas.set(x + 1, y + 1, '╲', Color::DarkGrey);
        }
    }

    fn draw_hud(&mut self) {
        let x = GAME_W as i32 + 6;
        self.canvas.text(x, 2, "STARWAKE", Color::Cyan);
        self.canvas.text(x, 4, &format!("SCORE  {:>8}", self.score), Color::White);
        self.canvas.text(x, 5, &format!("BEST   {:>8}", self.best), Color::DarkGrey);
        self.canvas.text(x, 7, &format!("WAVE   {:>8}", self.wave), Color::Yellow);
        self.canvas.text(x, 8, &format!("TIME   {:>6.0}s", self.time), Color::Grey);

        self.canvas.text(x, 11, "SHIELDS", Color::DarkGrey);
        for i in 0..5 {
            let ch = if i < self.player.shields.max(0) { '■' } else { '□' };
            let color = if i < self.player.shields.max(0) {
                Color::Green
            } else {
                Color::DarkGrey
            };
            self.canvas.set(x + i * 2, 13, ch, color);
        }

        self.canvas.text(x, 16, "OVERDRIVE", Color::DarkGrey);
        let bars = (self.player.overdrive / 5.0 * 14.0).round() as i32;
        for i in 0..14 {
            let color = if i < bars { Color::Cyan } else { Color::DarkGrey };
            self.canvas.set(x + i, 18, '▰', color);
        }

        self.canvas.text(x, 23, "CONTROLS", Color::DarkGrey);
        self.canvas.text(x, 25, "WASD / ARROWS", Color::Grey);
        self.canvas.text(x, 26, "SPACE / J / Z   fire", Color::Grey);
        self.canvas.text(x, 27, "P               pause", Color::Grey);
        self.canvas.text(x, 28, "Q / ESC         quit", Color::Grey);

        self.canvas.text(x, 34, "SIGNALS", Color::DarkGrey);
        self.canvas.text(x, 36, "◆ shards   ● comets", Color::Yellow);
        self.canvas.text(x, 37, "▼ raiders  ✦ shield", Color::Magenta);

        if self.message_timer > 0.0 {
            self.canvas.text(x, 43, "Clear a lane.", Color::White);
            self.canvas.text(x, 44, "Take shield cores.", Color::Cyan);
        }
    }

    fn draw_overlay(&mut self) {
        match self.mode {
            Mode::Title => {
                self.panel(22, 18, 42, 14, Color::DarkBlue);
                self.canvas.text(34, 21, "STARWAKE", Color::Cyan);
                self.canvas.text(27, 24, "Drift through a collapsing", Color::White);
                self.canvas.text(29, 25, "shipping lane at full burn.", Color::White);
                self.canvas.text(28, 28, "Press SPACE to launch", Color::Yellow);
                self.canvas.text(31, 30, "Q or ESC quits", Color::DarkGrey);
            }
            Mode::Paused => {
                self.panel(27, 22, 32, 9, Color::DarkBlue);
                self.canvas.text(39, 25, "PAUSED", Color::Yellow);
                self.canvas.text(32, 28, "Press P or SPACE to resume", Color::White);
            }
            Mode::GameOver => {
                self.panel(23, 20, 40, 13, Color::DarkRed);
                self.canvas.text(36, 23, "SHIP LOST", Color::Red);
                self.canvas.text(30, 26, &format!("Final score: {}", self.score), Color::White);
                self.canvas.text(30, 28, "SPACE or R to run again", Color::Yellow);
                self.canvas.text(34, 30, "Q or ESC quits", Color::DarkGrey);
            }
            Mode::Running => {}
        }
    }

    fn panel(&mut self, x: i32, y: i32, w: i32, h: i32, bg: Color) {
        for py in y..y + h {
            for px in x..x + w {
                let edge = px == x || px == x + w - 1 || py == y || py == y + h - 1;
                let ch = if edge { ' ' } else { ' ' };
                self.canvas.set_bg(px, py, ch, Color::White, bg);
            }
        }
    }
}

struct Player {
    x: f32,
    y: f32,
    speed: f32,
    fire_cooldown: f32,
    shields: i32,
    invuln: f32,
    overdrive: f32,
}

impl Player {
    fn new() -> Self {
        Self {
            x: (GAME_W / 2) as f32,
            y: (GAME_H - 7) as f32,
            speed: 38.0,
            fire_cooldown: 0.0,
            shields: 3,
            invuln: 1.0,
            overdrive: 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnemyKind {
    Shard,
    Comet,
    Raider,
}

struct Enemy {
    x: f32,
    y: f32,
    vy: f32,
    hp: i32,
    radius: f32,
    glyph: char,
    color: Color,
    kind: EnemyKind,
    drift: f32,
    sway: f32,
    phase: f32,
    cooldown: f32,
}

struct Shot {
    x: f32,
    y: f32,
    dy: f32,
    friendly: bool,
    damage: i32,
}

struct Pickup {
    x: f32,
    y: f32,
    phase: f32,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    color: Color,
}

struct Star {
    x: f32,
    y: f32,
    speed: f32,
    glyph: char,
    color: Color,
}

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }

    fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.next_f32() < probability
    }
}

fn dist2(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}
