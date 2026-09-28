//! Wave scripting. A wave is a timed list of spawns built from a handful of
//! formation generators, with the roster and pacing opening up as the run goes
//! on.

use crate::entities::{Arena, Kind};
use crate::rng::Rng;

pub struct Spawn {
    pub at: f32,
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub home_y: f32,
    pub dir: f32,
    pub amp: f32,
}

pub struct Wave {
    pub number: u32,
    pub spawns: Vec<Spawn>,
    pub hp_mul: f32,
    pub heat: f32,
    pub boss: Option<Kind>,
    /// Flavour line shown on the wave banner.
    pub subtitle: &'static str,
}

const SUBTITLES: [&str; 8] = [
    "HOSTILES INBOUND",
    "SECTOR CONTESTED",
    "SWARM DETECTED",
    "HOLD THE LINE",
    "PRESSURE RISING",
    "DEEP PATROL",
    "HEAVY RESISTANCE",
    "NO RETREAT",
];

struct Builder {
    spawns: Vec<Spawn>,
    t: f32,
    arena: Arena,
    /// Scales the gaps between and within formations. Later waves compress the
    /// timeline so pressure comes from overlap, not only from tougher hulls.
    pace: f32,
}

impl Builder {
    fn push(&mut self, at: f32, kind: Kind, x: f32, y: f32, home_y: f32, dir: f32, amp: f32) {
        self.spawns.push(Spawn {
            at,
            kind,
            x: x.clamp(6.0, self.arena.w - 6.0),
            y,
            home_y,
            dir,
            amp,
        });
    }

    /// A shallow V of enemies entering together.
    fn arc(&mut self, kind: Kind, count: usize, rng: &mut Rng) {
        let n = count.max(1);
        let span = self.arena.w * 0.78;
        let x0 = (self.arena.w - span) * 0.5;
        let mid = (n as f32 - 1.0) * 0.5;
        for i in 0..n {
            let f = if n == 1 { 0.5 } else { i as f32 / (n - 1) as f32 };
            let x = x0 + span * f;
            let depth = (mid - (i as f32 - mid).abs()) * 5.0;
            self.push(self.t, kind, x, -8.0 - depth, 0.0, 1.0, rng.range(6.0, 14.0));
        }
        self.advance(3.4);
    }

    /// A vertical column trickling in from one point.
    fn column(&mut self, kind: Kind, count: usize, rng: &mut Rng) {
        let x = rng.range(self.arena.w * 0.18, self.arena.w * 0.82);
        for i in 0..count {
            self.push(
                self.t + i as f32 * 0.40,
                kind,
                x + rng.sym(3.0),
                -8.0,
                0.0,
                1.0,
                rng.range(10.0, 22.0),
            );
        }
        self.advance(count as f32 * 0.40 + 1.6);
    }

    /// Two mirrored columns closing on the centre.
    fn pincer(&mut self, kind: Kind, count: usize, rng: &mut Rng) {
        for i in 0..count {
            let t = self.t + i as f32 * 0.35;
            let inset = 12.0 + i as f32 * 4.0;
            self.push(t, kind, inset, -8.0, 0.0, 1.0, rng.range(8.0, 16.0));
            self.push(t, kind, self.arena.w - inset, -8.0, 0.0, 1.0, rng.range(8.0, 16.0));
        }
        self.advance(count as f32 * 0.35 + 2.0);
    }

    /// Sentinels that settle onto a firing line.
    fn post(&mut self, count: usize, rng: &mut Rng) {
        let span = self.arena.w * 0.66;
        let x0 = (self.arena.w - span) * 0.5;
        for i in 0..count {
            let f = if count == 1 {
                0.5
            } else {
                i as f32 / (count - 1) as f32
            };
            let home = self.arena.h * rng.range(0.14, 0.30);
            self.push(
                self.t + i as f32 * 0.5,
                Kind::Sentinel,
                x0 + span * f,
                -9.0,
                home,
                1.0,
                rng.range(5.0, 11.0),
            );
        }
        self.advance(count as f32 * 0.5 + 3.2);
    }

    /// Weavers that cross the arena at a fixed height.
    fn crossers(&mut self, count: usize, rng: &mut Rng) {
        for i in 0..count {
            let dir = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let x = if dir > 0.0 { 10.0 } else { self.arena.w - 10.0 };
            let home = self.arena.h * rng.range(0.16, 0.42);
            self.push(
                self.t + i as f32 * 1.1,
                Kind::Weaver,
                x,
                -9.0,
                home,
                dir,
                8.0,
            );
        }
        self.advance(count as f32 * 1.1 + 2.6);
    }

    /// Scattered drifting mines.
    fn minefield(&mut self, count: usize, rng: &mut Rng) {
        for i in 0..count {
            self.push(
                self.t + i as f32 * 0.5,
                Kind::Mine,
                rng.range(10.0, self.arena.w - 10.0),
                -8.0 - rng.range(0.0, 16.0),
                0.0,
                1.0,
                rng.range(6.0, 18.0),
            );
        }
        self.advance(count as f32 * 0.5 + 2.4);
    }

    /// Advance the spawn clock, honouring the wave's pacing.
    fn advance(&mut self, s: f32) {
        self.t += s * self.pace;
    }

    fn gap(&mut self, s: f32) {
        self.advance(s);
    }
}

pub fn build(n: u32, arena: Arena, rng: &mut Rng) -> Wave {
    let mut b = Builder {
        spawns: Vec::new(),
        t: 1.4,
        arena,
        // Down to a 60% timeline by wave 20 and held there.
        pace: (1.0 - (n.saturating_sub(1) as f32) * 0.021).max(0.60),
    };

    let hp_mul = 1.0 + (n.saturating_sub(1) as f32) * 0.085;
    let heat = (n.saturating_sub(1) as f32).min(14.0);
    let boss_wave = n % 5 == 0;
    let warden_wave = !boss_wave && n >= 3 && n % 5 == 3;

    if boss_wave {
        // A short screen of escorts, then the capital ship arrives.
        b.arc(Kind::Drifter, 5, rng);
        b.gap(-1.2);
        b.pincer(Kind::Dart, 2, rng);
        b.push(b.t + 0.5, Kind::Dreadnought, arena.w * 0.5, -16.0, arena.h * 0.20, 1.0, 0.0);
        return Wave {
            number: n,
            spawns: b.spawns,
            hp_mul,
            heat,
            boss: Some(Kind::Dreadnought),
            subtitle: "CAPITAL SHIP NEAR",
        };
    }

    // Which formations are unlocked so far.
    let mut pool: Vec<u8> = vec![0, 1]; // arc + column of drifters
    if n >= 2 {
        pool.push(2); // darts
    }
    if n >= 3 {
        pool.push(3); // weavers
    }
    if n >= 4 {
        pool.push(4); // sentinels
    }
    if n >= 6 {
        pool.push(5); // mines
    }
    if n >= 7 {
        pool.push(6); // mixed pressure
    }

    let formations = (2 + n / 2).min(8) as usize;
    let scale = |base: usize| -> usize { (base + (n as usize) / 4).min(base + 4) };

    for _ in 0..formations {
        match *rng.pick(&pool) {
            0 => b.arc(Kind::Drifter, scale(4), rng),
            1 => b.column(Kind::Drifter, scale(4), rng),
            2 => b.pincer(Kind::Dart, scale(2), rng),
            3 => b.crossers(scale(1) + 1, rng),
            4 => b.post(scale(2), rng),
            5 => b.minefield(scale(3), rng),
            _ => {
                // Mixed pressure: a screen of drifters under a sentinel post.
                b.post(2, rng);
                b.gap(-2.4);
                b.arc(Kind::Drifter, scale(4), rng);
            }
        }
    }

    if warden_wave {
        b.gap(0.6);
        b.push(b.t, Kind::Warden, arena.w * 0.5, -12.0, arena.h * 0.18, 1.0, 0.0);
    }

    Wave {
        number: n,
        spawns: b.spawns,
        hp_mul,
        heat,
        boss: if warden_wave { Some(Kind::Warden) } else { None },
        subtitle: if warden_wave {
            "ELITE ESCORT"
        } else {
            SUBTITLES[(n as usize - 1) % SUBTITLES.len()]
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena() -> Arena {
        Arena { w: 122.0, h: 130.0 }
    }

    #[test]
    fn waves_are_non_empty_and_in_bounds() {
        let mut rng = Rng::new(99);
        for n in 1..=40u32 {
            let w = build(n, arena(), &mut rng);
            assert!(!w.spawns.is_empty(), "wave {n} is empty");
            for s in &w.spawns {
                assert!(s.at >= 0.0 && s.at < 400.0, "wave {n} spawn time {}", s.at);
                assert!(
                    s.x >= 0.0 && s.x <= arena().w,
                    "wave {n} spawn x {} out of bounds",
                    s.x
                );
                assert!(s.y <= 0.0, "spawns should start above the arena");
            }
        }
    }

    #[test]
    fn every_fifth_wave_is_a_capital_ship() {
        let mut rng = Rng::new(5);
        for n in 1..=30u32 {
            let w = build(n, arena(), &mut rng);
            if n % 5 == 0 {
                assert_eq!(w.boss, Some(Kind::Dreadnought), "wave {n}");
                assert_eq!(
                    w.spawns.iter().filter(|s| s.kind == Kind::Dreadnought).count(),
                    1
                );
            } else {
                assert_ne!(w.boss, Some(Kind::Dreadnought), "wave {n}");
            }
        }
    }

    #[test]
    fn early_waves_only_use_the_starter_roster() {
        let mut rng = Rng::new(3);
        let w = build(1, arena(), &mut rng);
        for s in &w.spawns {
            assert_eq!(s.kind, Kind::Drifter, "wave 1 should be drifters only");
        }
    }

    #[test]
    fn difficulty_rises_monotonically() {
        let mut rng = Rng::new(11);
        let mut last = 0.0;
        for n in 1..=20u32 {
            let w = build(n, arena(), &mut rng);
            assert!(w.hp_mul >= last);
            last = w.hp_mul;
        }
    }
}
