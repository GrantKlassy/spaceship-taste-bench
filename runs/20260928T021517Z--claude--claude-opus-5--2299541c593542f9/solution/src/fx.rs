//! Background and particle effects: a parallax starfield, a scrolling
//! procedural nebula, sparks, smoke, shockwaves and floating score text.

use crate::canvas::{lerp_rgb, scale, Canvas, Rgb};
use crate::rng::Rng;

// ---------------------------------------------------------------------------
// Nebula
// ---------------------------------------------------------------------------

/// Height of the nebula texture. It tiles seamlessly in Y so it can scroll
/// forever.
const NEB_H: usize = 192;

pub struct Nebula {
    w: usize,
    tex: Vec<Rgb>,
    offset: f32,
    pub speed: f32,
}

/// Value noise on an integer lattice that wraps vertically every `py` cells.
fn lattice(rng_seed: u64, x: i32, y: i32, py: i32) -> f32 {
    let y = y.rem_euclid(py);
    let mut h = rng_seed
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    ((h >> 40) as f32) * (1.0 / 16_777_216.0)
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(seed: u64, x: f32, y: f32, period_y: i32) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = smoothstep(x - x0 as f32);
    let ty = smoothstep(y - y0 as f32);
    let a = lattice(seed, x0, y0, period_y);
    let b = lattice(seed, x0 + 1, y0, period_y);
    let c = lattice(seed, x0, y0 + 1, period_y);
    let d = lattice(seed, x0 + 1, y0 + 1, period_y);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

impl Nebula {
    pub fn new(w: usize, seed: u64) -> Self {
        let mut n = Nebula {
            w,
            tex: vec![[0.0; 3]; w * NEB_H],
            offset: 0.0,
            speed: 2.6,
        };
        n.generate(seed);
        n
    }

    pub fn resize(&mut self, w: usize, seed: u64) {
        self.w = w;
        self.tex.resize(w * NEB_H, [0.0; 3]);
        self.generate(seed);
    }

    fn generate(&mut self, seed: u64) {
        for y in 0..NEB_H {
            for x in 0..self.w {
                let fx = x as f32;
                let fy = y as f32;
                // Four octaves of vertically-tiling fBm.
                let mut amp = 0.5f32;
                let mut f = 1.0f32 / 48.0;
                let mut period = (NEB_H as f32 * f).round() as i32;
                let mut v = 0.0f32;
                let mut norm = 0.0f32;
                for o in 0..4 {
                    let p = period.max(1);
                    v += value_noise(seed ^ (o as u64 * 0x1234_5), fx * f, fy * f, p) * amp;
                    norm += amp;
                    amp *= 0.5;
                    f *= 2.0;
                    period = period.saturating_mul(2);
                }
                v /= norm;

                // A second, lower-frequency field masks the gas into patches so
                // the sky is not uniformly foggy.
                let mask = value_noise(seed ^ 0xABCD, fx / 90.0, fy / 90.0, 2);
                let density = ((v - 0.46) * 2.6 * (mask * 1.5).min(1.0)).clamp(0.0, 1.0);

                // The gas is deliberately neutral. xterm-256 has 24 evenly
                // spaced greys but nothing between black and 95 per channel,
                // so a dim *coloured* cloud can only be drawn as a checkerboard
                // of black and bright primaries. Neutral dust rides the grey
                // ramp instead and stays perfectly smooth; the colour in this
                // sky comes from the stars and the fighting.
                let shade = value_noise(seed ^ 0x77, fx / 70.0, fy / 70.0, 3);
                let l = 0.006 + density * (0.058 + shade * 0.062);
                // A whisper of cool bias, small enough to stay on the ramp.
                self.tex[y * self.w + x] = [l * 0.95, l * 0.99, l * 1.07];
            }
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.offset = (self.offset + self.speed * dt) % NEB_H as f32;
    }

    pub fn draw(&self, canvas: &mut Canvas, intensity: f32) {
        let off = self.offset as usize;
        let w = self.w.min(canvas.w);
        for y in 0..canvas.h {
            let sy = (y + off) % NEB_H;
            let src = sy * self.w;
            let dst = y * canvas.w;
            for x in 0..w {
                let c = self.tex[src + x];
                let p = &mut canvas.buf[dst + x];
                p[0] += c[0] * intensity;
                p[1] += c[1] * intensity;
                p[2] += c[2] * intensity;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Starfield
// ---------------------------------------------------------------------------

struct Star {
    x: f32,
    y: f32,
    speed: f32,
    bright: f32,
    tint: Rgb,
    twinkle: f32,
    /// Nearby stars get a soft coloured halo, which is where the background
    /// gets its hue from.
    halo: f32,
}

pub struct Starfield {
    stars: Vec<Star>,
    w: f32,
    h: f32,
    t: f32,
    /// Multiplier on scroll speed, raised while warping between waves.
    pub warp: f32,
}

const STAR_TINTS: [Rgb; 7] = [
    [1.00, 1.00, 1.00],
    [0.68, 0.80, 1.00],
    [1.00, 0.90, 0.72],
    [1.00, 0.74, 0.62],
    [0.80, 0.94, 1.00],
    [0.86, 0.76, 1.00],
    [0.74, 1.00, 0.92],
];

impl Starfield {
    pub fn new(w: usize, h: usize, rng: &mut Rng) -> Self {
        let mut sf = Starfield {
            stars: Vec::new(),
            w: w as f32,
            h: h as f32,
            t: 0.0,
            warp: 1.0,
        };
        sf.populate(rng);
        sf
    }

    pub fn resize(&mut self, w: usize, h: usize, rng: &mut Rng) {
        self.w = w as f32;
        self.h = h as f32;
        self.populate(rng);
    }

    fn populate(&mut self, rng: &mut Rng) {
        let count = ((self.w * self.h) / 46.0) as usize;
        self.stars.clear();
        for _ in 0..count {
            // Three depth layers, skewed towards the dim distant ones.
            let layer = rng.f();
            let (speed, bright, halo) = if layer < 0.60 {
                (rng.range(4.0, 9.0), rng.range(0.10, 0.26), 0.0)
            } else if layer < 0.90 {
                (rng.range(13.0, 21.0), rng.range(0.28, 0.55), 0.0)
            } else {
                (
                    rng.range(28.0, 42.0),
                    rng.range(0.60, 1.00),
                    if rng.chance(0.28) { rng.range(1.5, 2.6) } else { 0.0 },
                )
            };
            self.stars.push(Star {
                x: rng.range(0.0, self.w),
                y: rng.range(0.0, self.h),
                speed,
                bright,
                tint: *rng.pick(&STAR_TINTS),
                twinkle: rng.range(0.0, 6.28),
                halo,
            });
        }
    }

    pub fn update(&mut self, dt: f32, rng: &mut Rng) {
        self.t += dt;
        for s in self.stars.iter_mut() {
            s.y += s.speed * self.warp * dt;
            if s.y >= self.h {
                s.y -= self.h;
                s.x = rng.range(0.0, self.w);
            }
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, intensity: f32) {
        for s in &self.stars {
            let tw = 0.82 + 0.18 * (self.t * 2.1 + s.twinkle).sin();
            let b = s.bright * tw * intensity;
            // At high warp the near stars stretch into short streaks.
            if s.halo > 0.0 {
                canvas.glow(s.x, s.y, s.halo, scale(s.tint, b * 0.20));
            }
            let stretch = (s.speed * self.warp - 40.0).max(0.0) * 0.05;
            if stretch > 0.6 {
                canvas.streak(s.x, s.y - stretch, s.x, s.y, scale(s.tint, b * 1.4));
            } else {
                canvas.add_sub(s.x, s.y, scale(s.tint, b));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Particles
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum PKind {
    /// Bright additive spark that leaves a short trail.
    Spark,
    /// Soft dark-ish puff that fades out.
    Smoke,
    /// Glowing blob with a halo.
    Ember,
    /// Tiny solid fragment of hull.
    Debris,
}

pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub px: f32,
    pub py: f32,
    pub vx: f32,
    pub vy: f32,
    pub life: f32,
    pub max_life: f32,
    pub c0: Rgb,
    pub c1: Rgb,
    pub size: f32,
    pub drag: f32,
    pub kind: PKind,
}

pub struct Shockwave {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub max_r: f32,
    pub life: f32,
    pub max_life: f32,
    pub c: Rgb,
    pub thick: f32,
}

pub struct FloatText {
    pub x: f32,
    pub y: f32,
    pub vy: f32,
    pub life: f32,
    pub max_life: f32,
    pub text: String,
    pub c: Rgb,
}

#[derive(Default)]
pub struct Fx {
    pub parts: Vec<Particle>,
    pub waves: Vec<Shockwave>,
    pub texts: Vec<FloatText>,
}

impl Fx {
    pub fn clear(&mut self) {
        self.parts.clear();
        self.waves.clear();
        self.texts.clear();
    }

    pub fn spark(&mut self, x: f32, y: f32, vx: f32, vy: f32, life: f32, c0: Rgb, c1: Rgb) {
        self.parts.push(Particle {
            x,
            y,
            px: x,
            py: y,
            vx,
            vy,
            life,
            max_life: life,
            c0,
            c1,
            size: 1.0,
            drag: 2.2,
            kind: PKind::Spark,
        });
    }

    pub fn push(&mut self, p: Particle) {
        // Hard cap so a chaotic frame can never spiral the frame time.
        if self.parts.len() < 3000 {
            self.parts.push(p);
        }
    }

    pub fn shock(&mut self, x: f32, y: f32, max_r: f32, life: f32, c: Rgb, thick: f32) {
        self.waves.push(Shockwave {
            x,
            y,
            r: 0.0,
            max_r,
            life,
            max_life: life,
            c,
            thick,
        });
    }

    pub fn text(&mut self, x: f32, y: f32, text: impl Into<String>, c: Rgb) {
        self.texts.push(FloatText {
            x,
            y,
            vy: -9.0,
            life: 0.9,
            max_life: 0.9,
            text: text.into(),
            c,
        });
    }

    /// A generic explosion: flash core, embers, sparks, smoke and debris.
    /// `power` scales both the count and the radius.
    pub fn explode(&mut self, x: f32, y: f32, power: f32, tint: Rgb, rng: &mut Rng) {
        let n_spark = (10.0 * power) as usize;
        for _ in 0..n_spark {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let sp = rng.range(18.0, 95.0) * power.sqrt();
            self.spark(
                x,
                y,
                a.cos() * sp,
                a.sin() * sp,
                rng.range(0.22, 0.55),
                [1.0, 0.95, 0.80],
                tint,
            );
        }
        let n_ember = (5.0 * power) as usize;
        for _ in 0..n_ember {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let sp = rng.range(6.0, 40.0) * power.sqrt();
            self.push(Particle {
                x,
                y,
                px: x,
                py: y,
                vx: a.cos() * sp,
                vy: a.sin() * sp,
                life: rng.range(0.35, 0.8),
                max_life: 0.8,
                c0: [1.0, 0.85, 0.55],
                c1: scale(tint, 0.5),
                size: rng.range(1.2, 2.6) * power.sqrt(),
                drag: 3.0,
                kind: PKind::Ember,
            });
        }
        let n_smoke = (4.0 * power) as usize;
        for _ in 0..n_smoke {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let sp = rng.range(3.0, 16.0);
            self.push(Particle {
                x,
                y,
                px: x,
                py: y,
                vx: a.cos() * sp,
                vy: a.sin() * sp + 8.0,
                life: rng.range(0.5, 1.1),
                max_life: 1.1,
                c0: scale(tint, 0.30),
                c1: [0.03, 0.03, 0.05],
                size: rng.range(2.0, 5.0) * power.sqrt(),
                drag: 1.4,
                kind: PKind::Smoke,
            });
        }
        let n_debris = (3.0 * power) as usize;
        for _ in 0..n_debris {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let sp = rng.range(25.0, 70.0);
            self.push(Particle {
                x,
                y,
                px: x,
                py: y,
                vx: a.cos() * sp,
                vy: a.sin() * sp + 18.0,
                life: rng.range(0.4, 0.9),
                max_life: 0.9,
                c0: scale(tint, 0.85),
                c1: scale(tint, 0.15),
                size: 1.0,
                drag: 0.5,
                kind: PKind::Debris,
            });
        }
        self.shock(x, y, 9.0 * power, 0.30 + 0.1 * power, [1.0, 0.9, 0.7], 2.2);
    }

    pub fn update(&mut self, dt: f32) {
        for p in self.parts.iter_mut() {
            p.px = p.x;
            p.py = p.y;
            let damp = (-p.drag * dt).exp();
            p.vx *= damp;
            p.vy *= damp;
            if p.kind == PKind::Smoke {
                p.vy -= 4.0 * dt; // smoke drifts back up the screen a little
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.life -= dt;
        }
        self.parts.retain(|p| p.life > 0.0);

        for w in self.waves.iter_mut() {
            w.life -= dt;
            let t = 1.0 - (w.life / w.max_life).max(0.0);
            // Ease out so the ring snaps open then slows.
            w.r = w.max_r * (1.0 - (1.0 - t).powi(3));
        }
        self.waves.retain(|w| w.life > 0.0);

        for t in self.texts.iter_mut() {
            t.life -= dt;
            t.y += t.vy * dt;
            t.vy *= (-2.0f32 * dt).exp();
        }
        self.texts.retain(|t| t.life > 0.0);
    }

    pub fn draw(&self, canvas: &mut Canvas) {
        for w in &self.waves {
            let t = (w.life / w.max_life).max(0.0);
            let a = t * t;
            canvas.ring(w.x, w.y, w.r, w.thick, scale(w.c, a * 0.9));
        }

        for p in &self.parts {
            let t = (p.life / p.max_life).clamp(0.0, 1.0);
            let col = lerp_rgb(p.c1, p.c0, t);
            match p.kind {
                PKind::Spark => {
                    let b = t * t;
                    canvas.streak(p.px, p.py, p.x, p.y, scale(col, b * 0.9));
                }
                PKind::Ember => {
                    let b = t.powf(0.7);
                    canvas.glow(p.x, p.y, p.size * (0.4 + t), scale(col, b * 0.55));
                }
                PKind::Smoke => {
                    let a = t * t * 0.45;
                    canvas.disc(p.x, p.y, p.size * (1.6 - t), col, a);
                }
                PKind::Debris => {
                    canvas.add_sub(p.x, p.y, scale(col, t));
                }
            }
        }
    }

    pub fn draw_texts(&self, canvas: &mut Canvas) {
        for t in &self.texts {
            let k = (t.life / t.max_life).clamp(0.0, 1.0);
            // Pop in quickly, then fade.
            let a = if k > 0.85 { (1.0 - k) / 0.15 } else { k / 0.85 };
            crate::sprites::stamp_text(canvas, &t.text, t.x, t.y, t.c, a.clamp(0.0, 1.0) * 0.95);
        }
    }

    pub fn count(&self) -> usize {
        self.parts.len()
    }
}

/// Engine exhaust: a warm plume with a flickering length.
pub fn thruster(canvas: &mut Canvas, x: f32, y: f32, len: f32, t: f32, c_hot: Rgb, c_cool: Rgb) {
    let flicker = 0.80 + 0.20 * ((t * 31.0).sin() * 0.5 + (t * 17.3).sin() * 0.5);
    let l = (len * flicker).max(0.6);
    let steps = (l * 2.0).ceil() as i32;
    for i in 0..=steps {
        let k = i as f32 / steps as f32;
        let yy = y + k * l;
        let col = lerp_rgb(c_hot, c_cool, k.powf(0.7));
        let fade = (1.0 - k).powf(0.9);
        canvas.add_sub(x, yy, scale(col, fade * 0.85));
        if k < 0.45 {
            canvas.glow(x, yy, 1.6 * (1.0 - k), scale(col, fade * 0.30));
        }
    }
}
