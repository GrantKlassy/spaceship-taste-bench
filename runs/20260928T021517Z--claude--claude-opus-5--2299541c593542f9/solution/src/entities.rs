//! Everything that lives in the playfield: the player's interceptor,
//! projectiles, the enemy roster with their behaviours, and pickups.

use crate::canvas::Rgb;
use crate::fx::Fx;
use crate::rng::Rng;

pub const TAU: f32 = std::f32::consts::TAU;

#[derive(Clone, Copy)]
pub struct Arena {
    pub w: f32,
    pub h: f32,
}

#[inline]
pub fn aim(fx: f32, fy: f32, tx: f32, ty: f32, speed: f32) -> (f32, f32) {
    let dx = tx - fx;
    let dy = ty - fy;
    let d = (dx * dx + dy * dy).sqrt().max(0.001);
    (dx / d * speed, dy / d * speed)
}

// ---------------------------------------------------------------------------
// Projectiles
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum BStyle {
    /// Player bolt: a bright vertical sliver.
    Bolt,
    /// Enemy orb: a glowing ball with a dark rim.
    Orb,
    /// Fast angular shard.
    Shard,
}

pub struct Bullet {
    pub x: f32,
    pub y: f32,
    pub px: f32,
    pub py: f32,
    pub vx: f32,
    pub vy: f32,
    pub dmg: f32,
    pub r: f32,
    pub life: f32,
    pub c: Rgb,
    pub style: BStyle,
    pub friendly: bool,
}

impl Bullet {
    pub fn player(x: f32, y: f32, vx: f32, vy: f32, dmg: f32) -> Bullet {
        Bullet {
            x,
            y,
            px: x,
            py: y,
            vx,
            vy,
            dmg,
            r: 2.0,
            life: 4.0,
            c: [0.60, 1.00, 1.00],
            style: BStyle::Bolt,
            friendly: true,
        }
    }

    pub fn foe(x: f32, y: f32, vx: f32, vy: f32, c: Rgb, r: f32, style: BStyle) -> Bullet {
        Bullet {
            x,
            y,
            px: x,
            py: y,
            vx,
            vy,
            dmg: 1.0,
            r,
            life: 12.0,
            c,
            style,
            friendly: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Player
// ---------------------------------------------------------------------------

pub const PLAYER_R: f32 = 3.4;
pub const MAX_WEAPON: u8 = 5;

pub struct Player {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub hull: i32,
    pub max_hull: i32,
    pub shield: f32,
    pub max_shield: f32,
    pub shield_delay: f32,
    pub weapon: u8,
    pub bombs: u8,
    pub max_bombs: u8,
    pub fire_cd: f32,
    pub invuln: f32,
    pub hit_flash: f32,
    pub bank: f32,
    pub alive: bool,
    pub muzzle: f32,
}

impl Player {
    pub fn new(arena: Arena) -> Self {
        Player {
            x: arena.w * 0.5,
            y: arena.h - 22.0,
            vx: 0.0,
            vy: 0.0,
            hull: 5,
            max_hull: 5,
            shield: 3.0,
            max_shield: 3.0,
            shield_delay: 0.0,
            weapon: 1,
            bombs: 2,
            max_bombs: 4,
            fire_cd: 0.0,
            invuln: 1.2,
            hit_flash: 0.0,
            bank: 0.0,
            alive: true,
            muzzle: 0.0,
        }
    }

    pub fn weapon_interval(&self) -> f32 {
        match self.weapon {
            1 => 0.120,
            2 => 0.120,
            3 => 0.130,
            4 => 0.120,
            _ => 0.110,
        }
    }

    /// Muzzle offsets and directions for the current weapon level.
    /// Returns (dx, dy, angle-from-vertical in radians).
    pub fn muzzles(&self) -> &'static [(f32, f32, f32)] {
        match self.weapon {
            1 => &[(0.0, -6.0, 0.0)],
            2 => &[(-2.5, -5.0, 0.0), (2.5, -5.0, 0.0)],
            3 => &[(0.0, -6.5, 0.0), (-4.0, -3.0, -0.13), (4.0, -3.0, 0.13)],
            4 => &[
                (-2.5, -5.5, 0.0),
                (2.5, -5.5, 0.0),
                (-5.0, -2.5, -0.19),
                (5.0, -2.5, 0.19),
            ],
            _ => &[
                (0.0, -7.0, 0.0),
                (-2.8, -5.5, -0.09),
                (2.8, -5.5, 0.09),
                (-5.2, -2.5, -0.24),
                (5.2, -2.5, 0.24),
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Enemies
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Drifter,
    Dart,
    Sentinel,
    Weaver,
    Mine,
    Warden,
    Dreadnought,
}

impl Kind {
    pub fn base_hp(self) -> f32 {
        match self {
            Kind::Drifter => 3.0,
            Kind::Dart => 3.0,
            Kind::Sentinel => 9.0,
            Kind::Weaver => 6.0,
            Kind::Mine => 4.0,
            Kind::Warden => 40.0,
            Kind::Dreadnought => 200.0,
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            Kind::Drifter => 4.2,
            Kind::Dart => 3.2,
            Kind::Sentinel => 6.2,
            Kind::Weaver => 5.2,
            Kind::Mine => 3.4,
            Kind::Warden => 8.5,
            Kind::Dreadnought => 15.0,
        }
    }

    pub fn score(self) -> u32 {
        match self {
            Kind::Drifter => 100,
            Kind::Dart => 150,
            Kind::Sentinel => 300,
            Kind::Weaver => 250,
            Kind::Mine => 200,
            Kind::Warden => 1500,
            Kind::Dreadnought => 6000,
        }
    }

    /// Damage dealt to the player on contact.
    pub fn ram(self) -> i32 {
        match self {
            Kind::Mine => 2,
            Kind::Warden | Kind::Dreadnought => 2,
            _ => 1,
        }
    }

    pub fn tint(self) -> Rgb {
        match self {
            Kind::Drifter => [1.00, 0.56, 0.16],
            Kind::Dart => [1.00, 0.36, 0.86],
            Kind::Sentinel => [0.48, 1.00, 0.62],
            Kind::Weaver => [1.00, 0.90, 0.32],
            Kind::Mine => [1.00, 0.26, 0.16],
            Kind::Warden => [0.80, 0.45, 1.00],
            Kind::Dreadnought => [0.85, 0.50, 1.00],
        }
    }

    pub fn is_boss(self) -> bool {
        matches!(self, Kind::Warden | Kind::Dreadnought)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Warden => "WARDEN",
            Kind::Dreadnought => "DREADNOUGHT",
            _ => "",
        }
    }
}

pub struct Enemy {
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub t: f32,
    pub phase: f32,
    pub fire_cd: f32,
    pub hit_flash: f32,
    pub dead: bool,
    pub home_x: f32,
    pub home_y: f32,
    pub dir: f32,
    pub state: u8,
    pub burst: u8,
    pub amp: f32,
    pub spawn_in: f32,
    pub drops: bool,
    pub telegraph: f32,
}

impl Enemy {
    pub fn new(kind: Kind, x: f32, y: f32, hp_mul: f32, rng: &mut Rng) -> Self {
        let hp = kind.base_hp() * hp_mul;
        Enemy {
            kind,
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            hp,
            max_hp: hp,
            t: 0.0,
            phase: rng.range(0.0, TAU),
            fire_cd: rng.range(0.6, 2.2),
            hit_flash: 0.0,
            dead: false,
            home_x: x,
            home_y: 0.0,
            dir: 1.0,
            state: 0,
            burst: 0,
            amp: rng.range(8.0, 20.0),
            spawn_in: 0.0,
            drops: true,
            telegraph: 0.0,
        }
    }

    pub fn radius(&self) -> f32 {
        self.kind.radius()
    }
}

/// Everything an enemy needs in order to act.
pub struct Ctx<'a> {
    pub arena: Arena,
    pub player_x: f32,
    pub player_y: f32,
    pub player_alive: bool,
    pub bullets: &'a mut Vec<Bullet>,
    pub fx: &'a mut Fx,
    pub rng: &'a mut Rng,
    pub reinforce: &'a mut Vec<Enemy>,
    /// Scales fire rate and projectile speed with the wave number.
    pub heat: f32,
}

const ORB_SPEED: f32 = 42.0;

fn shoot_orb(c: &mut Ctx, x: f32, y: f32, vx: f32, vy: f32, tint: Rgb, r: f32) {
    c.bullets
        .push(Bullet::foe(x, y, vx, vy, tint, r, BStyle::Orb));
}

/// Advance one enemy. Returns false when it should be removed for leaving the
/// arena (as opposed to being destroyed).
pub fn update_enemy(e: &mut Enemy, dt: f32, c: &mut Ctx) -> bool {
    e.t += dt;
    e.hit_flash = (e.hit_flash - dt * 6.0).max(0.0);
    e.telegraph = (e.telegraph - dt).max(0.0);
    if e.spawn_in > 0.0 {
        e.spawn_in -= dt;
        return true;
    }
    e.fire_cd -= dt;

    let a = c.arena;
    let heat = c.heat;

    match e.kind {
        // Weaves downwards, lobbing the occasional slow orb.
        Kind::Drifter => {
            e.vy = 21.0 + heat * 3.0;
            e.x = e.home_x + (e.t * 1.5 + e.phase).sin() * e.amp;
            e.y += e.vy * dt;
            if e.fire_cd <= 0.0 && e.y > 8.0 && e.y < a.h * 0.62 && c.player_alive {
                e.fire_cd = c.rng.range(2.0, 3.6) / (1.0 + heat * 0.10);
                let (vx, vy) = aim(e.x, e.y, c.player_x, c.player_y, ORB_SPEED + heat * 2.0);
                shoot_orb(c, e.x, e.y + 4.0, vx, vy, Kind::Drifter.tint(), 1.8);
            }
            e.y < a.h + 14.0
        }

        // Drops in, locks on, then dives. Pure ramming threat.
        Kind::Dart => {
            match e.state {
                0 => {
                    e.y += 26.0 * dt;
                    if e.t > 0.55 {
                        e.state = 1;
                        let tx = if c.player_alive { c.player_x } else { e.x };
                        let ty = if c.player_alive { c.player_y } else { a.h };
                        let (vx, vy) = aim(e.x, e.y, tx, ty + 30.0, 1.0);
                        e.vx = vx;
                        e.vy = vy;
                        e.telegraph = 0.25;
                    }
                }
                _ => {
                    let sp = (96.0 + heat * 8.0).min(150.0);
                    e.x += e.vx * sp * dt;
                    e.y += e.vy * sp * dt;
                    // Faint contrail.
                    if c.rng.chance(0.7) {
                        c.fx.spark(
                            e.x + c.rng.sym(1.2),
                            e.y - 3.0,
                            c.rng.sym(6.0),
                            -20.0,
                            0.22,
                            [1.0, 0.75, 1.0],
                            [0.4, 0.05, 0.35],
                        );
                    }
                }
            }
            e.y < a.h + 16.0 && e.x > -20.0 && e.x < a.w + 20.0
        }

        // Descends to a hover line, then fires aimed three-round bursts.
        Kind::Sentinel => {
            if e.y < e.home_y {
                e.y += 20.0 * dt;
            } else {
                e.x = e.home_x + (e.t * 0.7 + e.phase).sin() * (e.amp * 1.4);
                e.y = e.home_y + (e.t * 1.1 + e.phase).sin() * 2.0;
                // 0 = idle, 1 = winding up (the tell), 2 = firing the burst.
                if e.fire_cd <= 0.0 {
                    match e.state {
                        0 => {
                            if c.player_alive {
                                e.state = 1;
                                e.telegraph = 0.45;
                                e.fire_cd = 0.45;
                            } else {
                                e.fire_cd = 1.0;
                            }
                        }
                        1 => {
                            e.state = 2;
                            e.burst = 3;
                        }
                        _ => {
                            let (vx, vy) =
                                aim(e.x, e.y, c.player_x, c.player_y, ORB_SPEED + 12.0 + heat * 3.0);
                            shoot_orb(c, e.x, e.y + 5.0, vx, vy, Kind::Sentinel.tint(), 2.0);
                            e.burst -= 1;
                            if e.burst == 0 {
                                e.state = 0;
                                e.fire_cd = (2.8 - heat * 0.12).max(1.2);
                            } else {
                                e.fire_cd = 0.15;
                            }
                        }
                    }
                }
            }
            e.y < a.h + 14.0
        }

        // Sweeps across the arena raining three-way spreads.
        Kind::Weaver => {
            if e.y < e.home_y {
                e.y += 26.0 * dt;
            } else {
                e.x += e.dir * (30.0 + heat * 2.0) * dt;
                e.y = e.home_y + (e.t * 1.6 + e.phase).sin() * 6.0;
                if e.x < 6.0 {
                    e.dir = 1.0;
                }
                if e.x > a.w - 6.0 {
                    e.dir = -1.0;
                }
                if e.fire_cd <= 0.0 && c.player_alive {
                    e.fire_cd = (1.5 - heat * 0.06).max(0.7);
                    let sp = ORB_SPEED + heat * 2.0;
                    for k in -1..=1 {
                        let ang = std::f32::consts::FRAC_PI_2 + k as f32 * 0.32;
                        shoot_orb(
                            c,
                            e.x,
                            e.y + 4.0,
                            ang.cos() * sp,
                            ang.sin() * sp,
                            Kind::Weaver.tint(),
                            1.8,
                        );
                    }
                }
            }
            e.y < a.h + 14.0
        }

        // Drifts lazily; lethal on contact and shatters into a bullet ring.
        Kind::Mine => {
            e.vy = 15.0;
            e.x = e.home_x + (e.t * 0.9 + e.phase).sin() * e.amp * 0.7;
            e.y += e.vy * dt;
            e.y < a.h + 14.0
        }

        // Mini-boss: strafes near the top, alternating aimed bursts and rings.
        Kind::Warden => {
            if e.y < e.home_y {
                e.y += 22.0 * dt;
            } else {
                e.x += e.dir * 26.0 * dt;
                e.y = e.home_y + (e.t * 0.8).sin() * 4.0;
                if e.x < 12.0 {
                    e.dir = 1.0;
                }
                if e.x > a.w - 12.0 {
                    e.dir = -1.0;
                }
                if e.fire_cd <= 0.0 {
                    match e.state {
                        0 => {
                            // Aimed five-round fan.
                            let sp = ORB_SPEED + 14.0 + heat * 3.0;
                            let (ax, ay) = aim(e.x, e.y, c.player_x, c.player_y, 1.0);
                            let base = ay.atan2(ax);
                            for k in -2..=2 {
                                let ang = base + k as f32 * 0.16;
                                shoot_orb(
                                    c,
                                    e.x,
                                    e.y + 5.0,
                                    ang.cos() * sp,
                                    ang.sin() * sp,
                                    Kind::Warden.tint(),
                                    2.0,
                                );
                            }
                            e.burst += 1;
                            e.fire_cd = (1.05 - heat * 0.04).max(0.5);
                            if e.burst >= 3 {
                                e.burst = 0;
                                e.state = 1;
                                e.telegraph = 0.55;
                                e.fire_cd = 0.75;
                            }
                        }
                        _ => {
                            // Expanding ring.
                            let sp = ORB_SPEED - 4.0;
                            let n = 14;
                            for k in 0..n {
                                let ang = k as f32 / n as f32 * TAU + e.t * 0.6;
                                shoot_orb(
                                    c,
                                    e.x,
                                    e.y,
                                    ang.cos() * sp,
                                    ang.sin() * sp,
                                    [1.0, 0.65, 0.35],
                                    2.0,
                                );
                            }
                            c.fx
                                .shock(e.x, e.y, 16.0, 0.35, [1.0, 0.7, 0.4], 2.0);
                            e.state = 0;
                            e.fire_cd = 1.3;
                        }
                    }
                }
            }
            true
        }

        // The wave boss. Three escalating phases plus launched escorts.
        Kind::Dreadnought => {
            if e.y < e.home_y {
                e.y += 16.0 * dt;
                return true;
            }
            let frac = e.hp / e.max_hp;
            let target_phase: u8 = if frac < 0.35 {
                2
            } else if frac < 0.70 {
                1
            } else {
                0
            };
            if target_phase != e.state {
                e.state = target_phase;
                e.fire_cd = 0.9;
                e.telegraph = 0.9;
                c.fx.shock(e.x, e.y, 34.0, 0.55, [1.0, 0.8, 1.0], 3.0);
            }

            let sweep = (34.0 + e.state as f32 * 12.0) * dt;
            e.x += e.dir * sweep;
            let margin = 24.0;
            if e.x < margin {
                e.x = margin;
                e.dir = 1.0;
            }
            if e.x > a.w - margin {
                e.x = a.w - margin;
                e.dir = -1.0;
            }
            e.y = e.home_y + (e.t * 0.6).sin() * 3.0;

            if e.fire_cd <= 0.0 {
                match e.state {
                    0 => {
                        // Slow wide fan from the core.
                        let sp = ORB_SPEED + 6.0 + heat * 2.0;
                        for k in -4..=4 {
                            let ang = std::f32::consts::FRAC_PI_2 + k as f32 * 0.17;
                            shoot_orb(
                                c,
                                e.x,
                                e.y + 8.0,
                                ang.cos() * sp,
                                ang.sin() * sp,
                                Kind::Dreadnought.tint(),
                                2.2,
                            );
                        }
                        e.fire_cd = 1.6;
                        e.burst += 1;
                        if e.burst % 3 == 0 {
                            launch_escort(e, c);
                        }
                    }
                    1 => {
                        // Alternating pod volleys plus a rotating ring.
                        let sp = ORB_SPEED + 10.0;
                        let side = if e.burst % 2 == 0 { -1.0 } else { 1.0 };
                        for k in -1..=1 {
                            let (ax, ay) =
                                aim(e.x + side * 16.0, e.y + 2.0, c.player_x, c.player_y, 1.0);
                            let base = ay.atan2(ax) + k as f32 * 0.20;
                            shoot_orb(
                                c,
                                e.x + side * 16.0,
                                e.y + 2.0,
                                base.cos() * sp,
                                base.sin() * sp,
                                [1.0, 0.55, 0.9],
                                2.0,
                            );
                        }
                        e.burst = e.burst.wrapping_add(1);
                        if e.burst % 4 == 0 {
                            let n = 18;
                            for k in 0..n {
                                let ang = k as f32 / n as f32 * TAU + e.t * 0.9;
                                shoot_orb(
                                    c,
                                    e.x,
                                    e.y,
                                    ang.cos() * (ORB_SPEED - 6.0),
                                    ang.sin() * (ORB_SPEED - 6.0),
                                    [0.9, 0.5, 1.0],
                                    2.0,
                                );
                            }
                            c.fx.shock(e.x, e.y, 30.0, 0.4, [0.9, 0.6, 1.0], 2.5);
                            launch_escort(e, c);
                        }
                        e.fire_cd = 0.58;
                    }
                    _ => {
                        // Desperation: dense spiral and fast aimed shards.
                        let n = 5;
                        let sp = ORB_SPEED + 16.0;
                        for k in 0..n {
                            let ang = e.t * 2.6 + k as f32 / n as f32 * TAU;
                            shoot_orb(
                                c,
                                e.x,
                                e.y,
                                ang.cos() * sp,
                                ang.sin() * sp,
                                [1.0, 0.45, 0.75],
                                2.0,
                            );
                        }
                        if e.burst % 3 == 0 && c.player_alive {
                            let (vx, vy) = aim(e.x, e.y + 8.0, c.player_x, c.player_y, 78.0);
                            c.bullets.push(Bullet::foe(
                                e.x,
                                e.y + 8.0,
                                vx,
                                vy,
                                [1.0, 0.9, 0.6],
                                1.8,
                                BStyle::Shard,
                            ));
                        }
                        e.burst = e.burst.wrapping_add(1);
                        if e.burst % 10 == 0 {
                            launch_escort(e, c);
                        }
                        e.fire_cd = 0.26;
                    }
                }
            }
            true
        }
    }
}

fn launch_escort(e: &Enemy, c: &mut Ctx) {
    for side in [-1.0f32, 1.0] {
        let mut d = Enemy::new(Kind::Drifter, e.x + side * 17.0, e.y + 6.0, 1.0, c.rng);
        d.home_x = d.x;
        d.drops = false;
        c.reinforce.push(d);
    }
    c.fx.shock(e.x, e.y + 6.0, 12.0, 0.25, [1.0, 0.8, 0.5], 1.6);
}

// ---------------------------------------------------------------------------
// Pickups
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Boon {
    Weapon,
    Shield,
    Hull,
    Bomb,
    Points,
}

impl Boon {
    pub fn letter(self) -> char {
        match self {
            Boon::Weapon => 'W',
            Boon::Shield => 'S',
            Boon::Hull => 'H',
            Boon::Bomb => 'B',
            Boon::Points => 'P',
        }
    }

    pub fn tint(self) -> Rgb {
        match self {
            Boon::Weapon => [1.00, 0.85, 0.25],
            Boon::Shield => [0.35, 0.80, 1.00],
            Boon::Hull => [0.35, 1.00, 0.55],
            Boon::Bomb => [1.00, 0.45, 0.30],
            Boon::Points => [0.85, 0.70, 1.00],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Boon::Weapon => "WEAPON UP",
            Boon::Shield => "SHIELD",
            Boon::Hull => "HULL",
            Boon::Bomb => "BOMB",
            Boon::Points => "BONUS",
        }
    }
}

pub struct Pickup {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub t: f32,
    pub boon: Boon,
    pub life: f32,
}

impl Pickup {
    pub fn new(x: f32, y: f32, boon: Boon, rng: &mut Rng) -> Self {
        Pickup {
            x,
            y,
            vx: rng.sym(8.0),
            vy: 14.0,
            t: rng.range(0.0, TAU),
            boon,
            life: 13.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aim_returns_unit_scaled_velocity() {
        let (vx, vy) = aim(0.0, 0.0, 3.0, 4.0, 10.0);
        assert!((vx - 6.0).abs() < 1e-4, "{vx}");
        assert!((vy - 8.0).abs() < 1e-4, "{vy}");
    }

    #[test]
    fn aim_at_self_does_not_produce_nan() {
        let (vx, vy) = aim(5.0, 5.0, 5.0, 5.0, 40.0);
        assert!(vx.is_finite() && vy.is_finite());
    }

    #[test]
    fn every_weapon_level_has_muzzles() {
        let a = Arena { w: 100.0, h: 100.0 };
        let mut p = Player::new(a);
        for lvl in 1..=MAX_WEAPON {
            p.weapon = lvl;
            assert!(!p.muzzles().is_empty(), "level {lvl}");
            assert!(p.weapon_interval() > 0.0);
        }
    }

    #[test]
    fn drifters_leave_the_arena_and_are_culled() {
        let a = Arena { w: 100.0, h: 100.0 };
        let mut rng = Rng::new(1);
        let mut e = Enemy::new(Kind::Drifter, 50.0, 0.0, 1.0, &mut rng);
        let mut bullets = Vec::new();
        let mut fx = Fx::default();
        let mut reinforce = Vec::new();
        let mut alive = true;
        for _ in 0..1000 {
            let mut ctx = Ctx {
                arena: a,
                player_x: 50.0,
                player_y: 90.0,
                player_alive: true,
                bullets: &mut bullets,
                fx: &mut fx,
                rng: &mut rng,
                reinforce: &mut reinforce,
                heat: 0.0,
            };
            alive = update_enemy(&mut e, 1.0 / 60.0, &mut ctx);
            if !alive {
                break;
            }
        }
        assert!(!alive, "drifter should eventually exit the arena");
    }
}
