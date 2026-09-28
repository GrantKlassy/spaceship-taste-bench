//! Game state, simulation and world rendering.

use crate::canvas::{scale, Canvas, Rgb};
use crate::entities::*;
use crate::fx::{thruster, Fx, Nebula, PKind, Particle, Starfield};
use crate::rng::{seed_from_clock, Rng};
use crate::sprites::{self, DrawOpts, SpriteSet};
use crate::waves::{self, Wave};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Title,
    Play,
    Paused,
    GameOver,
}

/// A large centred announcement drawn over the playfield.
pub struct Banner {
    pub title: String,
    pub sub: String,
    pub t: f32,
    pub dur: f32,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub kills: u32,
    pub shots: u32,
    pub hits: u32,
    pub bombs_used: u32,
    pub best_combo: u32,
    pub run_time: f32,
}

/// Per-frame control state supplied by the front end.
#[derive(Default, Clone, Copy)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub fire: bool,
    pub bomb_edge: bool,
    pub confirm_edge: bool,
}

/// Colours for overlay text.
///
/// Every value here is an exact xterm-256 cube entry. Ordered dithering picks
/// between neighbouring palette levels, which is right for artwork but eats
/// one-pixel-wide glyph strokes; a colour the palette can hit exactly has no
/// quantisation error to dither away, so the text stays crisp.
mod ui {
    use crate::canvas::Rgb;
    /// 255,255,255
    pub const WHITE: Rgb = [1.0, 1.0, 1.0];
    /// 135,215,255
    pub const ICE: Rgb = [0.529, 0.843, 1.0];
    /// 175,215,255
    pub const PALE: Rgb = [0.686, 0.843, 1.0];
    /// 135,175,215
    pub const STEEL: Rgb = [0.529, 0.686, 0.843];
    /// 95,135,175
    pub const SLATE: Rgb = [0.373, 0.529, 0.686];
    /// 255,215,95
    pub const GOLD: Rgb = [1.0, 0.843, 0.373];
    /// 255,135,95
    pub const EMBER: Rgb = [1.0, 0.529, 0.373];
}

const COMBO_WINDOW: f32 = 2.6;
const SHIELD_DELAY: f32 = 3.6;
const SHIELD_REGEN: f32 = 0.42;
const PLAYER_ACCEL: f32 = 620.0;
const PLAYER_DAMP: f32 = 8.5;
const PLAYER_MAX_X: f32 = 64.0;
const PLAYER_MAX_Y: f32 = 50.0;
const BOLT_SPEED: f32 = 165.0;

pub struct Game {
    pub arena: Arena,
    pub canvas: Canvas,
    pub rng: Rng,
    pub sprites: SpriteSet,
    pub stars: Starfield,
    pub nebula: Nebula,
    pub fx: Fx,

    pub player: Player,
    pub bullets: Vec<Bullet>,
    pub enemies: Vec<Enemy>,
    pub pickups: Vec<Pickup>,
    reinforce: Vec<Enemy>,

    pub wave: Wave,
    pub wave_t: f32,
    spawn_idx: usize,
    wave_cleared: f32,

    pub mode: Mode,
    pub banner: Option<Banner>,
    pub score: u64,
    pub high: u64,
    pub new_high: bool,
    pub combo: u32,
    combo_t: f32,
    pub stats: Stats,

    pub shake: f32,
    shake_x: f32,
    shake_y: f32,
    flash: Rgb,
    hitstop: f32,
    time_scale: f32,
    death_t: f32,
    pub time: f32,
    pub autofire: bool,
    pub boss_ref: Option<(Kind, f32, f32)>,
    fade: f32,
}

impl Game {
    pub fn new(pw: usize, ph: usize, high: u64) -> Self {
        let mut rng = Rng::new(seed_from_clock());
        let arena = Arena {
            w: pw as f32,
            h: ph as f32,
        };
        let stars = Starfield::new(pw, ph, &mut rng);
        let nebula = Nebula::new(pw, rng.next_u64());
        let wave = waves::build(1, arena, &mut rng);
        let player = Player::new(arena);
        Game {
            arena,
            canvas: Canvas::new(pw, ph),
            rng,
            sprites: SpriteSet::new(),
            stars,
            nebula,
            fx: Fx::default(),
            player,
            bullets: Vec::new(),
            enemies: Vec::new(),
            pickups: Vec::new(),
            reinforce: Vec::new(),
            wave,
            wave_t: 0.0,
            spawn_idx: 0,
            wave_cleared: -1.0,
            mode: Mode::Title,
            banner: None,
            score: 0,
            high,
            new_high: false,
            combo: 0,
            combo_t: 0.0,
            stats: Stats::default(),
            shake: 0.0,
            shake_x: 0.0,
            shake_y: 0.0,
            flash: [0.0; 3],
            hitstop: 0.0,
            time_scale: 1.0,
            death_t: 0.0,
            time: 0.0,
            autofire: true,
            boss_ref: None,
            fade: 0.0,
        }
    }

    pub fn resize(&mut self, pw: usize, ph: usize) {
        self.arena = Arena {
            w: pw as f32,
            h: ph as f32,
        };
        self.canvas.resize(pw, ph);
        self.stars.resize(pw, ph, &mut self.rng);
        self.nebula.resize(pw, self.rng.next_u64());
        self.player.x = self.player.x.clamp(6.0, self.arena.w - 6.0);
        self.player.y = self.player.y.clamp(10.0, self.arena.h - 6.0);
    }

    pub fn start_run(&mut self) {
        let arena = self.arena;
        self.player = Player::new(arena);
        self.bullets.clear();
        self.enemies.clear();
        self.pickups.clear();
        self.reinforce.clear();
        self.fx.clear();
        self.score = 0;
        self.combo = 0;
        self.combo_t = 0.0;
        self.stats = Stats::default();
        self.new_high = false;
        self.death_t = 0.0;
        self.wave = waves::build(1, arena, &mut self.rng);
        self.begin_wave();
        self.mode = Mode::Play;
        self.fade = 1.0;
    }

    fn begin_wave(&mut self) {
        self.wave_t = 0.0;
        self.spawn_idx = 0;
        self.wave_cleared = -1.0;
        self.stars.warp = 5.0;
        let n = self.wave.number;
        // Boss waves get a longer, more emphatic announcement.
        let dur = if self.wave.boss.is_some() { 3.0 } else { 2.4 };
        self.banner = Some(Banner {
            title: format!("WAVE {n}"),
            sub: self.wave.subtitle.to_string(),
            t: 0.0,
            dur,
        });
    }

    /// Jump straight to a given wave. Used by the snapshot tool to reach the
    /// capital-ship encounters without playing through to them.
    pub fn jump_to_wave(&mut self, n: u32) {
        self.wave = waves::build(n, self.arena, &mut self.rng);
        self.begin_wave();
    }

    fn next_wave(&mut self) {
        let n = self.wave.number + 1;
        self.wave = waves::build(n, self.arena, &mut self.rng);
        self.begin_wave();
    }

    // -----------------------------------------------------------------
    // Simulation
    // -----------------------------------------------------------------

    pub fn update(&mut self, real_dt: f32, input: Input) {
        self.time += real_dt;

        // Hit-stop freezes the simulation for a beat on heavy impacts.
        if self.hitstop > 0.0 {
            self.hitstop -= real_dt;
            self.decay_presentation(real_dt);
            return;
        }

        // Ease the time scale back to normal after a bomb.
        self.time_scale += (1.0 - self.time_scale) * (1.0 - (-3.0 * real_dt).exp());
        let dt = (real_dt * self.time_scale).min(0.05);

        self.nebula.update(dt);
        self.stars.warp += (1.0 - self.stars.warp) * (1.0 - (-2.4 * dt).exp());
        self.stars.update(dt, &mut self.rng);
        self.decay_presentation(real_dt);

        match self.mode {
            Mode::Title => {
                self.fx.update(dt);
                if input.confirm_edge {
                    self.start_run();
                }
            }
            Mode::Paused => {}
            Mode::GameOver => {
                self.fx.update(dt);
                self.update_bullets(dt);
                self.update_pickups(dt);
                self.death_t += dt;
            }
            Mode::Play => self.update_play(dt, input),
        }
    }

    fn decay_presentation(&mut self, dt: f32) {
        self.shake = (self.shake - dt * 26.0).max(0.0);
        if self.shake > 0.05 {
            let a = self.time * 47.0;
            self.shake_x = (a.sin() * 1.7 + (a * 2.3).sin()) * self.shake * 0.5;
            self.shake_y = ((a * 1.7).cos() * 1.4 + (a * 3.1).cos()) * self.shake * 0.4;
        } else {
            self.shake_x = 0.0;
            self.shake_y = 0.0;
        }
        for c in self.flash.iter_mut() {
            *c = (*c - dt * 3.2).max(0.0);
        }
        self.fade = (self.fade - dt * 1.6).max(0.0);
    }

    fn update_play(&mut self, dt: f32, input: Input) {
        self.stats.run_time += dt;
        self.wave_t += dt;

        if let Some(b) = &mut self.banner {
            b.t += dt;
            if b.t >= b.dur {
                self.banner = None;
            }
        }

        self.update_player(dt, input);
        self.spawn_due();
        self.update_enemies(dt);
        self.update_bullets(dt);
        self.update_pickups(dt);
        self.fx.update(dt);
        self.collide();

        if self.combo_t > 0.0 {
            self.combo_t -= dt;
            if self.combo_t <= 0.0 {
                self.combo = 0;
            }
        }

        // Track the strongest boss on screen for the HUD bar.
        self.boss_ref = self
            .enemies
            .iter()
            .filter(|e| e.kind.is_boss())
            .max_by(|a, b| a.max_hp.total_cmp(&b.max_hp))
            .map(|e| (e.kind, e.hp, e.max_hp));

        // Wave completion: every scripted spawn has landed and the sky is clear.
        let all_spawned = self.spawn_idx >= self.wave.spawns.len();
        if all_spawned && self.enemies.is_empty() && self.player.alive {
            if self.wave_cleared < 0.0 {
                self.wave_cleared = 0.0;
                let bonus = 500 * self.wave.number as u64 + self.player.hull as u64 * 150;
                self.score += bonus;
                self.banner = Some(Banner {
                    title: "WAVE CLEAR".to_string(),
                    sub: format!("BONUS {bonus}"),
                    t: 0.0,
                    dur: 2.2,
                });
                self.stars.warp = 2.0;
            } else {
                self.wave_cleared += dt;
                if self.wave_cleared > 2.2 {
                    self.next_wave();
                }
            }
        }

        if !self.player.alive {
            self.death_t += dt;
            if self.death_t > 1.5 {
                self.end_run();
            }
        }
    }

    fn end_run(&mut self) {
        self.mode = Mode::GameOver;
        self.death_t = 0.0;
        if self.score > self.high {
            self.high = self.score;
            self.new_high = true;
        }
    }

    fn update_player(&mut self, dt: f32, input: Input) {
        let p = &mut self.player;
        p.invuln = (p.invuln - dt).max(0.0);
        p.hit_flash = (p.hit_flash - dt * 5.0).max(0.0);
        p.muzzle = (p.muzzle - dt * 12.0).max(0.0);
        if !p.alive {
            return;
        }

        let ax = (input.right as i32 - input.left as i32) as f32;
        let ay = (input.down as i32 - input.up as i32) as f32;
        p.vx += ax * PLAYER_ACCEL * dt;
        p.vy += ay * PLAYER_ACCEL * dt;
        let damp = (-PLAYER_DAMP * dt).exp();
        if ax == 0.0 {
            p.vx *= damp;
        }
        if ay == 0.0 {
            p.vy *= damp;
        }
        p.vx = p.vx.clamp(-PLAYER_MAX_X, PLAYER_MAX_X);
        p.vy = p.vy.clamp(-PLAYER_MAX_Y, PLAYER_MAX_Y);
        p.x += p.vx * dt;
        p.y += p.vy * dt;

        let (lo_x, hi_x) = (6.0, self.arena.w - 6.0);
        let (lo_y, hi_y) = (7.0, self.arena.h - 6.0);
        if p.x < lo_x {
            p.x = lo_x;
            p.vx = 0.0;
        }
        if p.x > hi_x {
            p.x = hi_x;
            p.vx = 0.0;
        }
        if p.y < lo_y {
            p.y = lo_y;
            p.vy = 0.0;
        }
        if p.y > hi_y {
            p.y = hi_y;
            p.vy = 0.0;
        }

        // Visual bank follows horizontal velocity with a little lag.
        let target_bank = (p.vx / PLAYER_MAX_X) * 0.30;
        p.bank += (target_bank - p.bank) * (1.0 - (-9.0 * dt).exp());

        // Shield regeneration.
        if p.shield_delay > 0.0 {
            p.shield_delay -= dt;
        } else if p.shield < p.max_shield {
            p.shield = (p.shield + SHIELD_REGEN * dt).min(p.max_shield);
        }

        // Weapons.
        p.fire_cd -= dt;
        let firing = self.autofire || input.fire;
        if firing && p.fire_cd <= 0.0 {
            p.fire_cd = p.weapon_interval();
            p.muzzle = 1.0;
            let muzzles = p.muzzles();
            for &(dx, dy, ang) in muzzles {
                let vx = ang.sin() * BOLT_SPEED + p.vx * 0.25;
                let vy = -ang.cos() * BOLT_SPEED;
                self.bullets
                    .push(Bullet::player(p.x + dx, p.y + dy, vx, vy, 1.0));
            }
            self.stats.shots += muzzles.len() as u32;
        }

        if input.bomb_edge && p.bombs > 0 {
            p.bombs -= 1;
            self.stats.bombs_used += 1;
            self.detonate_bomb();
        }

        // Engine trail.
        if self.rng.chance(0.85) {
            for side in [-2.0f32, 2.0] {
                self.fx.push(Particle {
                    x: self.player.x + side,
                    y: self.player.y + 4.5,
                    px: self.player.x + side,
                    py: self.player.y + 4.5,
                    vx: self.rng.sym(5.0) - self.player.vx * 0.10,
                    vy: self.rng.range(26.0, 46.0),
                    life: self.rng.range(0.14, 0.30),
                    max_life: 0.30,
                    c0: [0.55, 0.85, 1.0],
                    c1: [0.06, 0.10, 0.22],
                    size: 1.0,
                    drag: 2.0,
                    kind: PKind::Debris,
                });
            }
        }
    }

    fn detonate_bomb(&mut self) {
        let (px, py) = (self.player.x, self.player.y);
        self.flash = [0.55, 0.62, 0.78];
        self.shake = 9.0;
        self.time_scale = 0.28;
        self.hitstop = 0.06;
        self.fx
            .shock(px, py, self.arena.w * 0.95, 0.75, [0.75, 0.92, 1.0], 4.0);
        self.fx
            .shock(px, py, self.arena.w * 0.55, 0.5, [1.0, 1.0, 1.0], 2.5);

        // Enemy fire is annihilated and pays out a trickle of points.
        let mut cleared = 0u32;
        for b in self.bullets.iter().filter(|b| !b.friendly) {
            for _ in 0..2 {
                let a = self.rng.range(0.0, TAU);
                let sp = self.rng.range(10.0, 34.0);
                self.fx.spark(
                    b.x,
                    b.y,
                    a.cos() * sp,
                    a.sin() * sp,
                    0.3,
                    [1.0, 1.0, 0.9],
                    b.c,
                );
            }
            cleared += 1;
        }
        self.bullets.retain(|b| b.friendly);
        self.score += cleared as u64 * 10;

        let dmg = 14.0;
        let mut kills = Vec::new();
        for i in 0..self.enemies.len() {
            self.enemies[i].hp -= dmg;
            self.enemies[i].hit_flash = 1.0;
            if self.enemies[i].hp <= 0.0 && !self.enemies[i].dead {
                self.enemies[i].dead = true;
                kills.push(i);
            }
        }
        for i in kills {
            let (x, y, kind) = (self.enemies[i].x, self.enemies[i].y, self.enemies[i].kind);
            let drops = self.enemies[i].drops;
            self.on_enemy_destroyed(x, y, kind, drops);
        }
        self.enemies.retain(|e| !e.dead);
    }

    fn spawn_due(&mut self) {
        while self.spawn_idx < self.wave.spawns.len() {
            let s = &self.wave.spawns[self.spawn_idx];
            if s.at > self.wave_t {
                break;
            }
            let mut e = Enemy::new(s.kind, s.x, s.y, self.wave.hp_mul, &mut self.rng);
            e.home_x = s.x;
            e.home_y = if s.home_y > 0.0 {
                s.home_y
            } else {
                self.arena.h * 0.25
            };
            e.dir = s.dir;
            if s.amp > 0.0 {
                e.amp = s.amp;
            }
            self.enemies.push(e);
            self.spawn_idx += 1;
        }
    }

    fn update_enemies(&mut self, dt: f32) {
        let arena = self.arena;
        let heat = self.wave.heat;
        let (px, py, alive) = (self.player.x, self.player.y, self.player.alive);
        let mut i = 0;
        while i < self.enemies.len() {
            let keep = {
                let mut ctx = Ctx {
                    arena,
                    player_x: px,
                    player_y: py,
                    player_alive: alive,
                    bullets: &mut self.bullets,
                    fx: &mut self.fx,
                    rng: &mut self.rng,
                    reinforce: &mut self.reinforce,
                    heat,
                };
                update_enemy(&mut self.enemies[i], dt, &mut ctx)
            };
            if keep {
                i += 1;
            } else {
                self.enemies.swap_remove(i);
            }
        }
        if !self.reinforce.is_empty() {
            let extra = std::mem::take(&mut self.reinforce);
            self.enemies.extend(extra);
        }
    }

    fn update_bullets(&mut self, dt: f32) {
        let (w, h) = (self.arena.w, self.arena.h);
        for b in self.bullets.iter_mut() {
            b.px = b.x;
            b.py = b.y;
            b.x += b.vx * dt;
            b.y += b.vy * dt;
            b.life -= dt;
        }
        self.bullets.retain(|b| {
            b.life > 0.0 && b.x > -10.0 && b.x < w + 10.0 && b.y > -12.0 && b.y < h + 12.0
        });
    }

    fn update_pickups(&mut self, dt: f32) {
        let (px, py, alive) = (self.player.x, self.player.y, self.player.alive);
        let h = self.arena.h;
        for p in self.pickups.iter_mut() {
            p.t += dt;
            p.life -= dt;
            // Drift down, then home in once the player is close.
            let dx = px - p.x;
            let dy = py - p.y;
            let d = (dx * dx + dy * dy).sqrt();
            if alive && d < 26.0 {
                let pull = (1.0 - d / 26.0).powi(2) * 220.0;
                p.vx += dx / d.max(0.01) * pull * dt;
                p.vy += dy / d.max(0.01) * pull * dt;
            } else {
                p.vy += (16.0 - p.vy) * dt * 2.0;
                p.vx *= (-1.2f32 * dt).exp();
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        self.pickups.retain(|p| p.life > 0.0 && p.y < h + 10.0);
    }

    // -----------------------------------------------------------------
    // Collisions
    // -----------------------------------------------------------------

    fn collide(&mut self) {
        // Player fire against enemies.
        let mut bi = 0;
        while bi < self.bullets.len() {
            if !self.bullets[bi].friendly {
                bi += 1;
                continue;
            }
            let (bx, by, br, dmg) = {
                let b = &self.bullets[bi];
                (b.x, b.y, b.r, b.dmg)
            };
            let mut consumed = false;
            for ei in 0..self.enemies.len() {
                let e = &self.enemies[ei];
                if e.dead || e.spawn_in > 0.0 {
                    continue;
                }
                let rr = e.radius() + br;
                let dx = e.x - bx;
                let dy = e.y - by;
                if dx * dx + dy * dy <= rr * rr {
                    let e = &mut self.enemies[ei];
                    e.hp -= dmg;
                    e.hit_flash = 1.0;
                    self.stats.hits += 1;
                    let (ex, ey, kind) = (e.x, e.y, e.kind);
                    // Impact sparks.
                    for _ in 0..3 {
                        let a = self.rng.range(0.0, TAU);
                        let sp = self.rng.range(12.0, 40.0);
                        self.fx.spark(
                            bx,
                            by,
                            a.cos() * sp,
                            a.sin() * sp - 18.0,
                            0.16,
                            [1.0, 1.0, 0.92],
                            kind.tint(),
                        );
                    }
                    if self.enemies[ei].hp <= 0.0 {
                        self.enemies[ei].dead = true;
                        let drops = self.enemies[ei].drops;
                        self.on_enemy_destroyed(ex, ey, kind, drops);
                    }
                    consumed = true;
                    break;
                }
            }
            if consumed {
                self.bullets.swap_remove(bi);
            } else {
                bi += 1;
            }
        }
        self.enemies.retain(|e| !e.dead);

        if !self.player.alive {
            return;
        }

        // Enemy fire against the player.
        let (px, py) = (self.player.x, self.player.y);
        let mut hit_idx = None;
        for (i, b) in self.bullets.iter().enumerate() {
            if b.friendly {
                continue;
            }
            let rr = PLAYER_R + b.r;
            let dx = b.x - px;
            let dy = b.y - py;
            if dx * dx + dy * dy <= rr * rr {
                hit_idx = Some(i);
                break;
            }
        }
        if let Some(i) = hit_idx {
            let c = self.bullets[i].c;
            self.bullets.swap_remove(i);
            self.hurt(1, c);
        }

        // Ramming.
        let mut ram: Option<(usize, i32)> = None;
        for (i, e) in self.enemies.iter().enumerate() {
            if e.spawn_in > 0.0 {
                continue;
            }
            let rr = e.radius() + PLAYER_R;
            let dx = e.x - px;
            let dy = e.y - py;
            if dx * dx + dy * dy <= rr * rr {
                ram = Some((i, e.kind.ram()));
                break;
            }
        }
        if let Some((i, dmg)) = ram {
            let tint = self.enemies[i].kind.tint();
            let invulnerable = self.player.invuln > 0.0;
            self.hurt(dmg, tint);
            // Light craft break apart on impact; capital ships shrug it off.
            if !invulnerable && !self.enemies[i].kind.is_boss() {
                let (ex, ey, kind) = (self.enemies[i].x, self.enemies[i].y, self.enemies[i].kind);
                let drops = self.enemies[i].drops;
                self.enemies.swap_remove(i);
                self.on_enemy_destroyed(ex, ey, kind, drops);
            }
        }

        // Pickups.
        let mut got: Vec<Boon> = Vec::new();
        self.pickups.retain(|p| {
            let dx = p.x - px;
            let dy = p.y - py;
            if dx * dx + dy * dy <= (PLAYER_R + 4.5) * (PLAYER_R + 4.5) {
                got.push(p.boon);
                false
            } else {
                true
            }
        });
        for b in got {
            self.collect(b);
        }
    }

    fn collect(&mut self, boon: Boon) {
        let (px, py) = (self.player.x, self.player.y);
        let tint = boon.tint();
        match boon {
            Boon::Weapon => {
                if self.player.weapon < MAX_WEAPON {
                    self.player.weapon += 1;
                } else {
                    self.score += 750;
                }
            }
            Boon::Shield => {
                self.player.shield = (self.player.shield + 2.0).min(self.player.max_shield);
                self.player.shield_delay = 0.0;
            }
            Boon::Hull => {
                self.player.hull = (self.player.hull + 1).min(self.player.max_hull);
            }
            Boon::Bomb => {
                self.player.bombs = (self.player.bombs + 1).min(self.player.max_bombs);
            }
            Boon::Points => {
                self.score += 1500;
            }
        }
        self.fx.shock(px, py, 13.0, 0.35, tint, 2.0);
        for _ in 0..14 {
            let a = self.rng.range(0.0, TAU);
            let sp = self.rng.range(16.0, 52.0);
            self.fx
                .spark(px, py, a.cos() * sp, a.sin() * sp, 0.35, [1.0, 1.0, 1.0], tint);
        }
        self.fx.text(px, py - 8.0, boon.label(), tint);
        self.flash = scale(tint, 0.10);
    }

    fn hurt(&mut self, dmg: i32, tint: Rgb) {
        let p = &mut self.player;
        if !p.alive || p.invuln > 0.0 {
            return;
        }
        let (px, py) = (p.x, p.y);
        if p.shield >= 1.0 {
            p.shield -= 1.0;
            p.shield_delay = SHIELD_DELAY;
            p.invuln = 0.45;
            p.hit_flash = 0.7;
            self.shake = 3.5;
            self.flash = [0.05, 0.14, 0.24];
            self.fx.shock(px, py, 12.0, 0.3, [0.45, 0.85, 1.0], 2.0);
            for _ in 0..10 {
                let a = self.rng.range(0.0, TAU);
                let sp = self.rng.range(20.0, 60.0);
                self.fx.spark(
                    px,
                    py,
                    a.cos() * sp,
                    a.sin() * sp,
                    0.3,
                    [0.85, 0.98, 1.0],
                    [0.10, 0.35, 0.60],
                );
            }
            return;
        }

        p.hull -= dmg;
        p.invuln = 1.6;
        p.hit_flash = 1.0;
        self.combo = 0;
        self.combo_t = 0.0;
        self.shake = 8.0;
        self.hitstop = 0.05;
        self.flash = [0.32, 0.05, 0.06];
        let power = if self.player.hull <= 0 { 3.4 } else { 1.3 };
        let tint = if self.player.hull <= 0 {
            [1.0, 0.65, 0.3]
        } else {
            tint
        };
        self.fx.explode(px, py, power, tint, &mut self.rng);

        if self.player.hull <= 0 {
            self.player.hull = 0;
            self.player.alive = false;
            self.death_t = 0.0;
            self.shake = 16.0;
            self.hitstop = 0.14;
            self.flash = [0.55, 0.30, 0.15];
            self.fx.shock(px, py, 70.0, 1.0, [1.0, 0.8, 0.5], 4.0);
            // Scatter the wreck.
            for _ in 0..40 {
                let a = self.rng.range(0.0, TAU);
                let sp = self.rng.range(20.0, 120.0);
                self.fx.push(Particle {
                    x: px,
                    y: py,
                    px,
                    py,
                    vx: a.cos() * sp,
                    vy: a.sin() * sp,
                    life: self.rng.range(0.5, 1.4),
                    max_life: 1.4,
                    c0: [1.0, 0.9, 0.7],
                    c1: [0.25, 0.10, 0.05],
                    size: 1.0,
                    drag: 0.8,
                    kind: PKind::Debris,
                });
            }
        }
    }

    fn on_enemy_destroyed(&mut self, x: f32, y: f32, kind: Kind, drops: bool) {
        self.stats.kills += 1;
        self.combo += 1;
        self.combo_t = COMBO_WINDOW;
        self.stats.best_combo = self.stats.best_combo.max(self.combo);
        let mult = self.combo_multiplier();
        let gained = kind.score() as u64 * mult as u64;
        self.score += gained;

        let power = match kind {
            Kind::Dreadnought => 6.0,
            Kind::Warden => 3.2,
            Kind::Sentinel | Kind::Weaver => 1.5,
            _ => 1.0,
        };
        self.fx.explode(x, y, power, kind.tint(), &mut self.rng);
        self.shake = self.shake.max(match kind {
            Kind::Dreadnought => 16.0,
            Kind::Warden => 8.0,
            _ => 1.2,
        });
        if kind.is_boss() {
            self.hitstop = if kind == Kind::Dreadnought { 0.22 } else { 0.10 };
            self.flash = [0.35, 0.28, 0.45];
            self.fx.shock(x, y, 80.0, 1.1, [1.0, 0.85, 0.6], 5.0);
        }

        if mult > 1 {
            self.fx
                .text(x, y - 6.0, format!("{gained}"), ui::GOLD);
        } else {
            self.fx.text(x, y - 6.0, format!("{gained}"), ui::PALE);
        }

        // A mine's payload survives its hull.
        if kind == Kind::Mine {
            let n = 10;
            for k in 0..n {
                let ang = k as f32 / n as f32 * TAU + self.rng.f();
                self.bullets.push(Bullet::foe(
                    x,
                    y,
                    ang.cos() * 38.0,
                    ang.sin() * 38.0,
                    [1.0, 0.35, 0.2],
                    1.8,
                    BStyle::Orb,
                ));
            }
        }

        if drops {
            self.maybe_drop(x, y, kind);
        }
    }

    fn combo_multiplier(&self) -> u32 {
        (1 + self.combo / 5).min(8)
    }

    fn maybe_drop(&mut self, x: f32, y: f32, kind: Kind) {
        let p = match kind {
            Kind::Dreadnought => 1.0,
            Kind::Warden => 1.0,
            Kind::Sentinel => 0.30,
            Kind::Weaver => 0.22,
            Kind::Mine => 0.16,
            _ => 0.10,
        };
        if !self.rng.chance(p) {
            return;
        }
        let count = if kind.is_boss() { 3 } else { 1 };
        for i in 0..count {
            let boon = self.choose_boon(kind, i);
            let ox = if count > 1 {
                (i as f32 - 1.0) * 9.0
            } else {
                0.0
            };
            let pk = Pickup::new(x + ox, y, boon, &mut self.rng);
            self.pickups.push(pk);
        }
    }

    fn choose_boon(&mut self, kind: Kind, slot: usize) -> Boon {
        if kind.is_boss() {
            return match slot {
                0 => Boon::Weapon,
                1 => {
                    if self.player.hull < self.player.max_hull {
                        Boon::Hull
                    } else {
                        Boon::Bomb
                    }
                }
                _ => Boon::Shield,
            };
        }
        // Weight the drop table towards whatever the player currently lacks.
        let mut table: Vec<(Boon, f32)> = Vec::with_capacity(5);
        table.push((
            Boon::Weapon,
            if self.player.weapon < MAX_WEAPON {
                3.2
            } else {
                0.6
            },
        ));
        table.push((
            Boon::Shield,
            if self.player.shield < self.player.max_shield * 0.6 {
                2.6
            } else {
                1.0
            },
        ));
        table.push((
            Boon::Hull,
            if self.player.hull < self.player.max_hull {
                2.2
            } else {
                0.0
            },
        ));
        table.push((
            Boon::Bomb,
            if self.player.bombs < self.player.max_bombs {
                1.6
            } else {
                0.0
            },
        ));
        table.push((Boon::Points, 1.0));
        let total: f32 = table.iter().map(|(_, w)| *w).sum();
        let mut r = self.rng.range(0.0, total);
        for (b, w) in &table {
            r -= *w;
            if r <= 0.0 {
                return *b;
            }
        }
        Boon::Points
    }

    // -----------------------------------------------------------------
    // Commands from the front end
    // -----------------------------------------------------------------

    pub fn toggle_pause(&mut self) {
        self.mode = match self.mode {
            Mode::Play => Mode::Paused,
            Mode::Paused => Mode::Play,
            m => m,
        };
    }

    pub fn shake_offset(&self) -> (i32, i32) {
        (self.shake_x.round() as i32, self.shake_y.round() as i32)
    }

    // -----------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------

    pub fn render_world(&mut self) {
        let c = &mut self.canvas;
        c.clear();
        self.nebula.draw(c, 1.0);
        self.stars.draw(c, 1.0);

        if self.mode == Mode::Title {
            draw_title_art(c, self.time, &self.sprites);
        } else {
            draw_pickups(c, &self.pickups, self.time);
            draw_enemies(c, &self.enemies, &self.sprites, self.time);
            if self.player.alive {
                draw_player(c, &self.player, &self.sprites, self.time);
            }
            draw_bullets(c, &self.bullets);
            self.fx.draw(c);
        }

        if self.flash[0] + self.flash[1] + self.flash[2] > 0.001 {
            c.wash(self.flash);
        }
        if self.fade > 0.001 {
            c.tint(1.0 - self.fade * 0.9);
        }
        if self.mode == Mode::Paused {
            c.tint(0.34);
        } else if self.mode == Mode::GameOver {
            c.tint((1.0 - self.death_t * 0.35).clamp(0.45, 1.0));
        } else if let Some(b) = &self.banner {
            // Dim slightly while a banner is up so the text reads clearly.
            let k = (1.0 - (b.t / b.dur)).clamp(0.0, 1.0);
            c.tint(1.0 - 0.35 * k);
        }

        // Banner glyphs go down once before the bloom, to seed their halo...
        self.draw_overlays(true);

        // A high threshold keeps the bloom on things that are genuinely hot -
        // bolts, explosion cores, banner text - instead of softening the whole
        // frame.
        self.canvas.bloom(0.78, 0.75);

        // ...and again afterwards, flat. Text is the one thing that must land
        // on exact palette colours: bloom would otherwise nudge a glyph across
        // a quantisation boundary and make it shimmer between two palette
        // entries from frame to frame. Compositing the text layer after
        // post-processing keeps it perfectly still.
        self.draw_overlays(false);
        if self.mode != Mode::Title {
            draw_pickup_letters(&mut self.canvas, &self.pickups);
            self.fx.draw_texts(&mut self.canvas);
        }
    }

    /// Draw the overlay text. `glow` selects the pre-bloom pass, which also
    /// draws the non-text decorations.
    fn draw_overlays(&mut self, glow: bool) {
        let c = &mut self.canvas;
        let w = c.w as f32;
        let h = c.h as f32;
        let t = self.time;

        match self.mode {
            Mode::Title => {
                let sc = fit_scale("NEBULA", c.w, 3);
                let title = ui::ICE;
                let line_h = (7 * sc + 3) as f32;
                let y = h * 0.06;
                sprites::stamp_big(c, "NEBULA", w * 0.5, y, sc, title, 1.0, glow);
                sprites::stamp_big(c, "DRIFT", w * 0.5, y + line_h, sc, title, 1.0, glow);

                let ry = y + line_h * 2.0 + 4.0;
                if glow {
                    // Underline rule, brightest in the middle.
                    let half = (w * 0.30).min(46.0);
                    c.streak(w * 0.5 - half, ry, w * 0.5 + half, ry, [0.08, 0.22, 0.38]);
                    c.glow(w * 0.5, ry, 14.0, [0.05, 0.12, 0.20]);
                }

                sprites::stamp_big(
                    c,
                    "HOLD THE LINE",
                    w * 0.5,
                    ry + 5.0,
                    1,
                    ui::STEEL,
                    1.0,
                    glow,
                );

                let pulse = 0.55 + 0.45 * (t * 3.0).sin();
                sprites::stamp_big(
                    c,
                    "PRESS ENTER TO FLY",
                    w * 0.5,
                    h * 0.785,
                    1,
                    ui::GOLD,
                    pulse,
                    glow,
                );
                sprites::stamp_big(
                    c,
                    "ARROWS OR WASD MOVE",
                    w * 0.5,
                    h * 0.870,
                    1,
                    ui::SLATE,
                    1.0,
                    glow,
                );
                sprites::stamp_big(
                    c,
                    "X BOMB   P PAUSE",
                    w * 0.5,
                    h * 0.930,
                    1,
                    ui::SLATE,
                    1.0,
                    glow,
                );
            }
            Mode::Paused => {
                let sc = fit_scale("PAUSED", c.w, 2);
                sprites::stamp_big(c, "PAUSED", w * 0.5, h * 0.42, sc, ui::WHITE, 1.0, glow);
                sprites::stamp_big(
                    c,
                    "P OR ESC TO RESUME",
                    w * 0.5,
                    h * 0.42 + (7 * sc + 6) as f32,
                    1,
                    ui::STEEL,
                    1.0,
                    glow,
                );
            }
            Mode::GameOver => {
                let k = (self.death_t * 1.6).clamp(0.0, 1.0);
                let sc = fit_scale("RUN ENDED", c.w, 2);
                let mut y = h * 0.28;
                sprites::stamp_big(c, "RUN ENDED", w * 0.5, y, sc, ui::EMBER, k, glow);
                y += (7 * sc + 8) as f32;
                if self.new_high {
                    let pulse = 0.6 + 0.4 * (t * 5.0).sin();
                    sprites::stamp_big(
                        c,
                        "NEW BEST",
                        w * 0.5,
                        y,
                        fit_scale("NEW BEST", c.w, 2),
                        ui::GOLD,
                        k * pulse,
                        glow,
                    );
                    y += (7 * fit_scale("NEW BEST", c.w, 2) + 6) as f32;
                }
                let s = self.stats;
                let secs = s.run_time as u32;
                for (i, line) in [
                    format!("SCORE {}", self.score),
                    format!("WAVE {}  KILLS {}", self.wave.number, s.kills),
                    format!(
                        "CHAIN X{}  TIME {}-{:02}",
                        (1 + s.best_combo / 5).min(8),
                        secs / 60,
                        secs % 60
                    ),
                ]
                .iter()
                .enumerate()
                {
                    sprites::stamp_big(
                        c,
                        line,
                        w * 0.5,
                        y + i as f32 * 9.0,
                        1,
                        ui::PALE,
                        k,
                        glow,
                    );
                }
                let pulse = 0.55 + 0.45 * (t * 3.0).sin();
                sprites::stamp_big(
                    c,
                    "R RESTART   Q QUIT",
                    w * 0.5,
                    h * 0.74,
                    1,
                    ui::GOLD,
                    k * pulse,
                    glow,
                );
            }
            Mode::Play => {
                if let Some(b) = &self.banner {
                    let p = b.t / b.dur;
                    // Fade in quickly, hold, then fade out.
                    let a = if p < 0.15 {
                        p / 0.15
                    } else if p > 0.75 {
                        ((1.0 - p) / 0.25).max(0.0)
                    } else {
                        1.0
                    };
                    // Rise into place. The entrance is vertical on purpose:
                    // a wide banner like "WAVE CLEAR" nearly fills the
                    // playfield, so a horizontal slide would clip its ends.
                    let rise = (1.0 - (p / 0.25).min(1.0)).powi(2) * 7.0;
                    let sc = fit_scale(&b.title, c.w, 2);
                    let y = h * 0.40;
                    sprites::stamp_big(
                        c,
                        &b.title,
                        w * 0.5,
                        y + rise,
                        sc,
                        ui::WHITE,
                        a,
                        glow,
                    );
                    sprites::stamp_big(
                        c,
                        &b.sub,
                        w * 0.5,
                        y + (7 * sc + 6) as f32 - rise * 0.8,
                        1,
                        ui::STEEL,
                        a,
                        glow,
                    );
                }
            }
        }
    }
}

/// Largest font scale (up to `max`) at which `text` still fits the canvas.
fn fit_scale(text: &str, width: usize, max: i32) -> i32 {
    let mut sc = max.max(1);
    while sc > 1 && sprites::big_width(text, sc) > width as i32 - 4 {
        sc -= 1;
    }
    sc
}

// ---------------------------------------------------------------------------
// World drawing helpers
// ---------------------------------------------------------------------------

fn draw_player(c: &mut Canvas, p: &Player, s: &SpriteSet, t: f32) {
    // Blink while invulnerable, but never disappear entirely.
    let blink = if p.invuln > 0.0 {
        0.45 + 0.55 * ((t * 26.0).sin() * 0.5 + 0.5)
    } else {
        1.0
    };

    if p.shield >= 1.0 {
        let pulse = 0.55 + 0.45 * (t * 2.2).sin();
        let strength = (p.shield / p.max_shield).clamp(0.0, 1.0);
        let a = 0.10 + 0.10 * pulse * strength;
        c.ring(p.x, p.y, 8.5, 2.6, [0.16 * a * 10.0, 0.45 * a * 10.0, 0.75 * a * 10.0]);
    }

    let speed = (p.vx * p.vx + p.vy * p.vy).sqrt();
    let len = 5.0 + (p.vy.min(0.0).abs() * 0.10) + speed * 0.02;
    for side in [-2.0f32, 2.0] {
        thruster(
            c,
            p.x + side,
            p.y + 4.0,
            len,
            t + side,
            [0.85, 0.97, 1.00],
            [0.10, 0.25, 0.75],
        );
    }

    sprites::draw(
        c,
        &s.player,
        p.x,
        p.y,
        DrawOpts {
            alpha: blink,
            flash: p.hit_flash,
            shear: p.bank,
            emissive: 1.0 + p.muzzle * 0.8,
        },
    );

    if p.muzzle > 0.0 {
        for &(dx, dy, _) in p.muzzles() {
            c.glow(
                p.x + dx,
                p.y + dy,
                3.4 * p.muzzle,
                scale([0.45, 0.95, 1.0], 0.5 * p.muzzle),
            );
        }
    }
}

fn draw_enemies(c: &mut Canvas, list: &[Enemy], s: &SpriteSet, t: f32) {
    for e in list {
        if e.spawn_in > 0.0 {
            continue;
        }
        let sprite = match e.kind {
            Kind::Drifter => &s.drifter,
            Kind::Dart => &s.dart,
            Kind::Sentinel => &s.sentinel,
            Kind::Weaver => &s.weaver,
            Kind::Mine => &s.mine,
            Kind::Warden => &s.warden,
            Kind::Dreadnought => &s.dreadnought,
        };

        // Wind-up tell: a pulsing halo before an attack lands.
        if e.telegraph > 0.0 {
            let k = e.telegraph;
            let pulse = 0.5 + 0.5 * (t * 30.0).sin();
            c.ring(
                e.x,
                e.y,
                e.radius() + 3.0 + (1.0 - k) * 4.0,
                2.0,
                scale(e.kind.tint(), 0.30 * k * (0.5 + pulse * 0.5)),
            );
        }

        // Damaged capital ships smoulder.
        if e.kind.is_boss() && e.hp < e.max_hp * 0.5 {
            let f = 1.0 - e.hp / (e.max_hp * 0.5);
            c.glow(
                e.x + (t * 3.1).sin() * 8.0,
                e.y + (t * 2.3).cos() * 4.0,
                5.0,
                scale([1.0, 0.45, 0.15], 0.25 * f),
            );
        }

        let bob = if e.kind == Kind::Mine {
            (t * 3.0 + e.phase).sin() * 0.0
        } else {
            0.0
        };
        sprites::draw(
            c,
            sprite,
            e.x,
            e.y + bob,
            DrawOpts {
                alpha: 1.0,
                flash: e.hit_flash * 0.85,
                shear: 0.0,
                emissive: 1.0,
            },
        );
    }
}

fn draw_bullets(c: &mut Canvas, list: &[Bullet]) {
    for b in list {
        match b.style {
            BStyle::Bolt => {
                c.streak(b.x, b.y + 4.0, b.x, b.y - 2.5, scale(b.c, 0.85));
                c.add_sub(b.x, b.y, [0.9, 1.0, 1.0]);
                c.glow(b.x, b.y, 3.0, scale(b.c, 0.28));
            }
            BStyle::Orb => {
                c.glow(b.x, b.y, b.r * 3.0, scale(b.c, 0.22));
                c.disc(b.x, b.y, b.r * 0.85, b.c, 0.95);
                c.add(b.x.round() as i32, b.y.round() as i32, scale(b.c, 0.6));
                c.add_sub(b.x, b.y, [0.55, 0.55, 0.55]);
            }
            BStyle::Shard => {
                let d = (b.vx * b.vx + b.vy * b.vy).sqrt().max(1.0);
                let (ux, uy) = (b.vx / d, b.vy / d);
                c.streak(
                    b.x - ux * 5.0,
                    b.y - uy * 5.0,
                    b.x + ux * 2.0,
                    b.y + uy * 2.0,
                    scale(b.c, 0.9),
                );
                c.glow(b.x, b.y, 3.5, scale(b.c, 0.30));
            }
        }
    }
}

fn draw_pickups(c: &mut Canvas, list: &[Pickup], t: f32) {
    for p in list {
        // Flash urgently as the pickup is about to expire.
        let expiring = p.life < 3.5;
        if expiring && ((p.life * 7.0) as i32) % 2 == 0 {
            continue;
        }
        let tint = p.boon.tint();
        let pulse = 0.6 + 0.4 * (t * 4.0 + p.t).sin();
        c.glow(p.x, p.y, 8.0, scale(tint, 0.18 * pulse));

        // A slowly spinning gem outline.
        let ang = t * 1.6 + p.t;
        let r = 5.2;
        let pts: Vec<(f32, f32)> = (0..4)
            .map(|k| {
                let a = ang + k as f32 * std::f32::consts::FRAC_PI_2;
                (p.x + a.cos() * r, p.y + a.sin() * r * 0.92)
            })
            .collect();
        for k in 0..4 {
            let (x0, y0) = pts[k];
            let (x1, y1) = pts[(k + 1) % 4];
            c.streak(x0, y0, x1, y1, scale(tint, 0.55));
        }
    }
}

/// Pickup labels, drawn with the rest of the text layer after post-processing.
fn draw_pickup_letters(c: &mut Canvas, list: &[Pickup]) {
    for p in list {
        if p.life < 3.5 && ((p.life * 7.0) as i32) % 2 == 0 {
            continue;
        }
        sprites::stamp_letter(c, p.boon.letter(), p.x, p.y, ui::WHITE, 1.0);
    }
}

/// The attract-mode ship that idles on the title screen.
fn draw_title_art(c: &mut Canvas, t: f32, s: &SpriteSet) {
    let cx = c.w as f32 * 0.5;
    let cy = c.h as f32 * 0.68;
    let x = cx + (t * 0.55).sin() * 12.0;
    let y = cy + (t * 0.9).sin() * 2.5;
    for side in [-2.0f32, 2.0] {
        thruster(c, x + side, y + 4.0, 6.0, t + side, [0.85, 0.97, 1.0], [0.10, 0.25, 0.75]);
    }
    sprites::draw(
        c,
        &s.player,
        x,
        y,
        DrawOpts {
            shear: ((t * 0.55).cos() * 0.18).clamp(-0.3, 0.3),
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Game {
        Game::new(122, 130, 0)
    }

    #[test]
    fn a_full_run_simulates_without_panicking() {
        let mut g = game();
        g.start_run();
        let dt = 1.0 / 60.0;
        let mut rng = Rng::new(4242);
        // Drive it with noisy input for several simulated minutes.
        for i in 0..18_000 {
            let inp = Input {
                left: rng.chance(0.3),
                right: rng.chance(0.3),
                up: rng.chance(0.2),
                down: rng.chance(0.2),
                fire: true,
                bomb_edge: i % 700 == 0,
                confirm_edge: false,
            };
            g.update(dt, inp);
            if g.mode == Mode::GameOver {
                g.start_run();
            }
            assert!(g.player.x.is_finite() && g.player.y.is_finite());
        }
        assert!(g.score < u64::MAX);
    }

    #[test]
    fn player_stays_inside_the_arena() {
        let mut g = game();
        g.start_run();
        g.banner = None;
        for _ in 0..600 {
            g.update(
                1.0 / 60.0,
                Input {
                    left: true,
                    up: true,
                    ..Default::default()
                },
            );
        }
        assert!(g.player.x >= 5.9, "x={}", g.player.x);
        assert!(g.player.y >= 6.9, "y={}", g.player.y);
        for _ in 0..600 {
            g.update(
                1.0 / 60.0,
                Input {
                    right: true,
                    down: true,
                    ..Default::default()
                },
            );
        }
        assert!(g.player.x <= g.arena.w - 5.9, "x={}", g.player.x);
        assert!(g.player.y <= g.arena.h - 5.9, "y={}", g.player.y);
    }

    #[test]
    fn taking_damage_drains_shield_before_hull() {
        let mut g = game();
        g.start_run();
        g.player.invuln = 0.0;
        let hull = g.player.hull;
        g.hurt(1, [1.0, 0.0, 0.0]);
        assert_eq!(g.player.hull, hull, "shield should absorb the first hit");
        assert!(g.player.shield < 3.0);
    }

    #[test]
    fn running_out_of_hull_ends_the_run() {
        let mut g = game();
        g.start_run();
        g.player.shield = 0.0;
        for _ in 0..10 {
            g.player.invuln = 0.0;
            g.hurt(1, [1.0, 0.0, 0.0]);
        }
        assert!(!g.player.alive);
        for _ in 0..200 {
            g.update(1.0 / 60.0, Input::default());
        }
        assert_eq!(g.mode, Mode::GameOver);
        assert!(g.high >= g.score);
    }

    #[test]
    fn bombs_clear_enemy_fire() {
        let mut g = game();
        g.start_run();
        for i in 0..20 {
            g.bullets.push(Bullet::foe(
                10.0 + i as f32,
                40.0,
                0.0,
                20.0,
                [1.0, 0.0, 0.0],
                2.0,
                BStyle::Orb,
            ));
        }
        g.player.bombs = 1;
        g.update(1.0 / 60.0, Input { bomb_edge: true, ..Default::default() });
        assert_eq!(g.player.bombs, 0);
        assert!(
            g.bullets.iter().all(|b| b.friendly),
            "all hostile fire should be cleared"
        );
    }

    #[test]
    fn combo_multiplier_is_capped() {
        let mut g = game();
        g.combo = 10_000;
        assert_eq!(g.combo_multiplier(), 8);
    }

    #[test]
    fn waves_advance_when_the_sky_is_cleared() {
        let mut g = game();
        g.start_run();
        assert_eq!(g.wave.number, 1);
        // Fast-forward past every scripted spawn, removing enemies as they appear.
        for _ in 0..6000 {
            g.update(1.0 / 60.0, Input::default());
            g.enemies.clear();
            g.player.hull = 5;
            g.player.alive = true;
            if g.wave.number >= 2 {
                break;
            }
        }
        assert!(g.wave.number >= 2, "wave did not advance");
        assert!(g.score > 0, "wave clear should award a bonus");
    }

    #[test]
    fn late_waves_stay_bounded_and_stable() {
        let mut g = game();
        g.start_run();
        let dt = 1.0 / 60.0;
        let mut rng = Rng::new(9001);
        let mut worst_bullets = 0usize;
        let mut worst_parts = 0usize;
        let mut worst_enemies = 0usize;
        // Sit at several deep waves in turn, invincible, and watch the
        // entity counts. A run that never terminates a wave is the worst
        // case for accumulation.
        for wave in [12u32, 20, 30, 45] {
            g.jump_to_wave(wave);
            g.banner = None;
            for i in 0..4_000 {
                g.player.hull = g.player.max_hull;
                g.player.shield = g.player.max_shield;
                g.player.alive = true;
                g.player.weapon = 3;
                g.update(
                    dt,
                    Input {
                        left: rng.chance(0.3),
                        right: rng.chance(0.3),
                        fire: true,
                        bomb_edge: i % 900 == 0,
                        ..Default::default()
                    },
                );
                worst_bullets = worst_bullets.max(g.bullets.len());
                worst_parts = worst_parts.max(g.fx.count());
                worst_enemies = worst_enemies.max(g.enemies.len());
                assert!(g.player.x.is_finite() && g.player.y.is_finite());
                for e in &g.enemies {
                    assert!(e.x.is_finite() && e.y.is_finite() && e.hp.is_finite());
                }
            }
        }
        assert!(
            worst_bullets < 1500,
            "hostile fire accumulated without bound: {worst_bullets}"
        );
        assert!(worst_parts <= 3000, "particles exceeded the cap: {worst_parts}");
        assert!(worst_enemies < 200, "enemy count ran away: {worst_enemies}");
        eprintln!(
            "peak bullets {worst_bullets}, particles {worst_parts}, enemies {worst_enemies}"
        );
    }

    #[test]
    fn resize_keeps_the_player_in_bounds() {
        let mut g = game();
        g.start_run();
        g.player.x = 118.0;
        g.resize(60, 70);
        assert!(g.player.x <= 54.0);
        g.update(1.0 / 60.0, Input::default());
    }
}
