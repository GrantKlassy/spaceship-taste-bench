use std::f32::consts::PI;
pub const W: f32 = 86.0;
pub const H: f32 = 50.0;
pub const SECTOR_TIME: f32 = 38.0;
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Title,
    Playing,
    Paused,
    Upgrade,
    Lost,
    Won,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Scout,
    Striker,
    Rock,
}
#[derive(Clone, Copy)]
pub struct Enemy {
    pub x: f32,
    pub y: f32,
    pub hp: f32,
    pub age: f32,
    pub kind: Kind,
    pub seed: f32,
    pub fire: f32,
}
#[derive(Clone, Copy)]
pub struct Shot {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub friendly: bool,
    pub damage: f32,
}
#[derive(Clone, Copy)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub life: f32,
    pub max: f32,
    pub color: u8,
}
#[derive(Clone, Copy, PartialEq)]
pub enum LootKind {
    Core,
    Repair,
}
#[derive(Clone, Copy)]
pub struct Loot {
    pub x: f32,
    pub y: f32,
    pub kind: LootKind,
}
#[derive(Clone, Copy)]
pub struct Star {
    pub x: f32,
    pub y: f32,
    pub layer: u8,
}
#[derive(Clone, Copy)]
pub struct Boss {
    pub x: f32,
    pub y: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub age: f32,
    pub fire: f32,
    pub sweep: f32,
    pub warning: f32,
    pub beam: f32,
    pub target: f32,
}
#[derive(Default, Clone, Copy)]
pub struct Input {
    pub x: f32,
    pub y: f32,
}
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / 16777216.0
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}
pub struct Game {
    pub mode: Mode,
    pub time: f32,
    pub sector: u8,
    pub sector_time: f32,
    pub x: f32,
    pub y: f32,
    pub hull: f32,
    pub max_hull: f32,
    pub invuln: f32,
    pub phase: f32,
    pub phase_cd: f32,
    pub nova: f32,
    pub nova_ring: f32,
    pub score: u32,
    pub kills: u32,
    pub combo: u32,
    pub combo_time: f32,
    pub best: u32,
    pub enemies: Vec<Enemy>,
    pub shots: Vec<Shot>,
    pub particles: Vec<Particle>,
    pub loot: Vec<Loot>,
    pub stars: Vec<Star>,
    pub boss: Option<Boss>,
    pub banner: String,
    pub banner_time: f32,
    pub feed: Vec<String>,
    pub weapon: u8,
    pub engine: u8,
    pub guardian_spawned: bool,
    pub fire_timer: f32,
    pub spawn_timer: f32,
    pub rng: Rng,
    pub flash: f32,
}
impl Game {
    pub fn new(seed: u64, best: u32) -> Self {
        let mut rng = Rng::new(seed);
        let stars = (0..95)
            .map(|_| Star {
                x: rng.range(0., W - 1.),
                y: rng.range(0., H - 1.),
                layer: (rng.f() * 3.) as u8,
            })
            .collect();
        Self {
            mode: Mode::Title,
            time: 0.,
            sector: 1,
            sector_time: 0.,
            x: W / 2.,
            y: H - 7.,
            hull: 100.,
            max_hull: 100.,
            invuln: 0.,
            phase: 0.,
            phase_cd: 0.,
            nova: 70.,
            nova_ring: 0.,
            score: 0,
            kills: 0,
            combo: 0,
            combo_time: 0.,
            best,
            enemies: vec![],
            shots: vec![],
            particles: vec![],
            loot: vec![],
            stars,
            boss: None,
            banner: String::new(),
            banner_time: 0.,
            feed: vec![],
            weapon: 0,
            engine: 0,
            guardian_spawned: false,
            fire_timer: 0.,
            spawn_timer: 1.,
            rng,
            flash: 0.,
        }
    }
    pub fn start(&mut self) {
        let best = self.best.max(self.score);
        let seed = (self.rng.f() * 1e12) as u64;
        *self = Self::new(seed, best);
        self.mode = Mode::Playing;
        self.announce("01  /  THE GLASS SHOALS", 3.5);
        self.log("Vesper, you are clear to fly.");
    }
    pub fn sector_name(&self) -> &'static str {
        match self.sector {
            1 => "THE GLASS SHOALS",
            2 => "EMBER CURRENT",
            _ => "THE QUIET GATE",
        }
    }
    pub fn boss_name(&self) -> &'static str {
        match self.sector {
            1 => "THE KEEPER",
            2 => "THE FURNACE",
            _ => "THE LAST LIGHT",
        }
    }
    pub fn announce(&mut self, s: &str, t: f32) {
        self.banner = s.into();
        self.banner_time = t;
    }
    pub fn log(&mut self, s: &str) {
        self.feed.push(s.into());
        if self.feed.len() > 5 {
            self.feed.remove(0);
        }
    }
    pub fn phase_drive(&mut self) {
        if self.mode == Mode::Playing && self.phase_cd <= 0. {
            self.phase = 0.85;
            self.phase_cd = (4.2 - self.engine as f32 * 0.7).max(2.1);
            self.burst(self.x, self.y, 18, 117);
        }
    }
    pub fn discharge(&mut self) {
        if self.mode != Mode::Playing || self.nova < 100. {
            return;
        }
        self.nova = 0.;
        self.nova_ring = 0.9;
        self.invuln = self.invuln.max(0.7);
        self.shots.retain(|s| s.friendly);
        for e in &mut self.enemies {
            e.hp -= 12.;
        }
        if let Some(b) = &mut self.boss {
            b.hp -= 38.;
        }
        self.burst(self.x, self.y, 55, 159);
        self.log("Nova released. Sky is clear.");
    }
    pub fn upgrade(&mut self, choice: u8) {
        if self.mode != Mode::Upgrade {
            return;
        }
        match choice {
            1 => {
                self.weapon += 1;
                self.log("Twin lances calibrated.");
            }
            2 => {
                self.max_hull += 30.;
                self.log("Hull plating reinforced.");
            }
            _ => {
                self.engine += 1;
                self.log("Phase drive overclocked.");
            }
        }
        self.hull = (self.hull + 35.).min(self.max_hull);
        self.nova = (self.nova + 30.).min(100.);
        self.sector += 1;
        self.sector_time = 0.;
        self.guardian_spawned = false;
        self.boss = None;
        self.enemies.clear();
        self.shots.clear();
        self.loot.clear();
        self.particles.clear();
        self.x = W / 2.;
        self.y = H - 7.;
        self.invuln = 2.;
        self.phase = 0.;
        self.phase_cd = 0.;
        self.spawn_timer = 1.8;
        self.mode = Mode::Playing;
        self.announce(&format!("0{}  /  {}", self.sector, self.sector_name()), 3.5);
    }
    pub fn burst(&mut self, x: f32, y: f32, n: usize, color: u8) {
        for _ in 0..n {
            let a = self.rng.range(0., PI * 2.);
            let v = self.rng.range(3., 20.);
            let life = self.rng.range(0.25, 0.85);
            self.particles
                .push(Particle {
                    x,
                    y,
                    vx: a.cos() * v,
                    vy: a.sin() * v * 0.55,
                    life,
                    max: life,
                    color,
                });
        }
    }
    fn hurt(&mut self, damage: f32) {
        if self.invuln > 0. || self.phase > 0. || self.mode != Mode::Playing {
            return;
        }
        self.hull = (self.hull - damage).max(0.);
        self.invuln = 1.05;
        self.flash = 0.22;
        self.combo = 0;
        self.combo_time = 0.;
        self.burst(self.x, self.y, 16, 203);
        if self.hull <= 0. {
            self.mode = Mode::Lost;
            self.best = self.best.max(self.score);
            self.burst(self.x, self.y, 60, 208);
        }
    }
    pub fn tick(&mut self, dt: f32, input: Input) {
        if self.mode == Mode::Paused || self.mode == Mode::Upgrade {
            return;
        }
        self.time += dt;
        for star in &mut self.stars {
            star.y += dt * (1.0 + star.layer as f32 * 1.8);
            if star.y >= H {
                star.y -= H;
                star.x = self.rng.range(0., W - 1.);
            }
        }
        for p in &mut self.particles {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.life -= dt;
            p.vx *= 1. - dt * 1.8;
        }
        self.particles.retain(|p| p.life > 0.);
        if self.mode != Mode::Playing {
            return;
        }
        self.sector_time += dt;
        self.invuln = (self.invuln - dt).max(0.);
        self.phase = (self.phase - dt).max(0.);
        self.phase_cd = (self.phase_cd - dt).max(0.);
        self.flash = (self.flash - dt).max(0.);
        self.nova_ring = (self.nova_ring - dt).max(0.);
        self.banner_time = (self.banner_time - dt).max(0.);
        self.combo_time = (self.combo_time - dt).max(0.);
        if self.combo_time <= 0. {
            self.combo = 0;
        }
        self.nova = (self.nova + dt * 1.8).min(100.);
        let norm = (input.x * input.x + input.y * input.y).sqrt().max(1.);
        let speed = if self.phase > 0. { 46. } else { 29. + self.engine as f32 * 2. };
        self.x = (self.x + input.x / norm * speed * dt).clamp(3., W - 4.);
        self.y = (self.y + input.y / norm * speed * 0.57 * dt).clamp(4., H - 3.);
        if self.rng.f() < 0.8 {
            self.particles
                .push(Particle {
                    x: self.x,
                    y: self.y + 2.,
                    vx: self.rng.range(-2., 2.),
                    vy: 8.,
                    life: 0.3,
                    max: 0.3,
                    color: if self.phase > 0. { 159 } else { 67 },
                });
        }
        self.fire_timer -= dt;
        if self.fire_timer <= 0. {
            self.fire_timer += 0.17;
            for dx in [-1.2, 1.2] {
                self.shots
                    .push(Shot {
                        x: self.x + dx,
                        y: self.y - 1.5,
                        vx: 0.,
                        vy: -44.,
                        friendly: true,
                        damage: 1. + self.weapon as f32 * 0.4,
                    });
            }
            if self.weapon > 0 {
                for sign in [-1., 1.] {
                    self.shots
                        .push(Shot {
                            x: self.x + sign * 2.,
                            y: self.y - 0.5,
                            vx: sign * 4.,
                            vy: -41.,
                            friendly: true,
                            damage: 0.6 + self.weapon as f32 * 0.2,
                        });
                }
            }
        }
        if self.sector_time < SECTOR_TIME {
            self.spawn_timer -= dt;
            if self.spawn_timer <= 0. {
                self.spawn_timer = self.rng.range(0.7, 1.3)
                    / (1. + self.sector as f32 * 0.18);
                self.spawn();
            }
        } else if !self.guardian_spawned {
            self.guardian_spawned = true;
            self.enemies.clear();
            self.shots.retain(|s| s.friendly);
            let hp = 220. + self.sector as f32 * 100.;
            self.boss = Some(Boss {
                x: W / 2.,
                y: -4.,
                hp,
                max_hp: hp,
                age: 0.,
                fire: 2.,
                sweep: 7.,
                warning: 0.,
                beam: 0.,
                target: W / 2.,
            });
            self.announce(&format!("GUARDIAN  /  {}", self.boss_name()), 3.);
            self.log("Large contact. Stay moving.");
        }
        self.update_enemies(dt);
        self.update_boss(dt);
        self.update_shots(dt);
        self.resolve_enemies();
        self.update_loot(dt);
        if self.mode == Mode::Playing && self.boss.is_some_and(|b| b.hp <= 0.) {
            let b = self.boss.take().unwrap();
            self.burst(b.x, b.y, 100, 220);
            self.score += 2500 * self.sector as u32;
            self.best = self.best.max(self.score);
            self.shots.clear();
            self.enemies.clear();
            if self.sector == 3 {
                self.mode = Mode::Won;
            } else {
                self.mode = Mode::Upgrade;
            }
        }
    }
    fn spawn(&mut self) {
        let roll = self.rng.f();
        let kind = if roll < 0.24 {
            Kind::Rock
        } else if roll < 0.49 {
            Kind::Striker
        } else {
            Kind::Scout
        };
        let hp = match kind {
            Kind::Rock => 8.,
            Kind::Striker => 6.,
            Kind::Scout => 3.,
        };
        let x = self.rng.range(6., W - 7.);
        let seed = self.rng.range(0., PI * 2.);
        self.enemies
            .push(Enemy {
                x,
                y: -2.,
                hp,
                age: 0.,
                kind,
                seed,
                fire: self.rng.range(0.8, 2.2),
            });
    }
    fn aimed(
        shots: &mut Vec<Shot>,
        x: f32,
        y: f32,
        tx: f32,
        ty: f32,
        speed: f32,
        angle: f32,
    ) {
        let dx = tx - x;
        let dy = (ty - y) * 1.8;
        let a = dy.atan2(dx) + angle;
        shots
            .push(Shot {
                x,
                y,
                vx: a.cos() * speed,
                vy: a.sin() * speed / 1.8,
                friendly: false,
                damage: 12.,
            });
    }
    fn update_enemies(&mut self, dt: f32) {
        let mut impact = false;
        for e in &mut self.enemies {
            e.age += dt;
            e.y
                += dt
                    * match e.kind {
                        Kind::Scout => 7. + self.sector as f32,
                        Kind::Striker => 4.2,
                        Kind::Rock => 5.5,
                    };
            if e.kind != Kind::Rock {
                e.x = (e.x + (e.age * 1.8 + e.seed).sin() * dt * 5.).clamp(3., W - 4.);
            }
            e.fire -= dt;
            if e.kind != Kind::Rock && e.fire <= 0. && e.y > 1. && e.y < self.y - 3. {
                e.fire = if e.kind == Kind::Striker { 1.7 } else { 3.2 };
                Self::aimed(
                    &mut self.shots,
                    e.x,
                    e.y + 1.,
                    self.x,
                    self.y,
                    17. + self.sector as f32 * 2.,
                    0.,
                );
                if e.kind == Kind::Striker && self.sector > 1 {
                    for a in [-0.22, 0.22] {
                        Self::aimed(
                            &mut self.shots,
                            e.x,
                            e.y + 1.,
                            self.x,
                            self.y,
                            19.,
                            a,
                        );
                    }
                }
            }
            if (e.x - self.x).abs() < 3. && (e.y - self.y).abs() < 1.9 {
                impact = true;
                if self.phase <= 0. {
                    e.hp = 0.;
                }
            }
        }
        if impact {
            self.hurt(20.);
        }
        self.enemies.retain(|e| e.y < H + 4.);
    }
    fn update_boss(&mut self, dt: f32) {
        let mut beam_hit = false;
        let mut contact = false;
        if let Some(b) = &mut self.boss {
            b.age += dt;
            b.y = (b.y + dt * 3.).min(8.);
            b.x = W / 2. + (b.age * 0.42).sin() * (22. + self.sector as f32 * 2.);
            b.fire -= dt;
            b.sweep -= dt;
            if b.fire <= 0. && b.y > 4. {
                b.fire = (1.2 - self.sector as f32 * 0.13)
                    * if b.hp < b.max_hp * 0.4 { 0.77 } else { 1. };
                for i in -(self.sector as i32 + 1)..=(self.sector as i32 + 1) {
                    Self::aimed(
                        &mut self.shots,
                        b.x,
                        b.y + 2.,
                        self.x,
                        self.y,
                        20. + self.sector as f32 * 2.,
                        i as f32 * 0.18,
                    );
                }
            }
            if b.sweep <= 0. {
                b.sweep = 7.;
                b.warning = 1.65;
                b.target = self.x;
            }
            if b.warning > 0. {
                b.warning -= dt;
                if b.warning <= 0. {
                    b.beam = 0.65;
                }
            } else if b.beam > 0. {
                b.beam = (b.beam - dt).max(0.);
                if (self.x - b.target).abs() < 3.7 && self.y > b.y {
                    beam_hit = true;
                }
            }
            contact = (self.x - b.x).abs() < 9. && (self.y - b.y).abs() < 3.;
        }
        if beam_hit {
            self.hurt(28.);
        }
        if contact {
            self.hurt(22.);
        }
    }
    fn update_shots(&mut self, dt: f32) {
        let mut impacts = Vec::new();
        let mut damage = 0.;
        for s in &mut self.shots {
            let old_y = s.y;
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            if s.friendly {
                for e in &mut self.enemies {
                    let (rx, ry) = if e.kind == Kind::Rock {
                        (3., 1.5)
                    } else {
                        (2.6, 1.2)
                    };
                    if e.hp > 0. && (s.x - e.x).abs() < rx && s.y <= e.y + ry
                        && old_y >= e.y - ry
                    {
                        e.hp -= s.damage;
                        s.y = -100.;
                        impacts.push((s.x, e.y, 117));
                        break;
                    }
                }
                if s.y > -90. {
                    if let Some(b) = &mut self.boss {
                        if (s.x - b.x).abs() < 8.5 && s.y <= b.y + 2.
                            && old_y >= b.y - 2.
                        {
                            b.hp -= s.damage;
                            s.y = -100.;
                            impacts.push((s.x, b.y + 2., 220));
                        }
                    }
                }
            } else if (s.x - self.x).abs() < 1.65 && (s.y - self.y).abs() < 1.15 {
                damage += s.damage;
                s.y = H + 100.;
            }
        }
        self.shots.retain(|s| s.y > -5. && s.y < H + 4. && s.x > 0. && s.x < W);
        for (x, y, c) in impacts {
            self.burst(x, y, 2, c);
        }
        if damage > 0. {
            self.hurt(damage.min(20.));
        }
    }
    fn resolve_enemies(&mut self) {
        let dead: Vec<_> = self.enemies.iter().filter(|e| e.hp <= 0.).copied().collect();
        self.enemies.retain(|e| e.hp > 0.);
        for e in dead {
            self.kills += 1;
            self.combo += 1;
            self.combo_time = 4.;
            let mult = 1 + (self.combo / 5).min(4);
            self.score
                += match e.kind {
                    Kind::Rock => 60,
                    Kind::Scout => 100,
                    Kind::Striker => 160,
                } * mult;
            self.nova = (self.nova + 4.).min(100.);
            self.burst(e.x, e.y, 18, if e.kind == Kind::Rock { 180 } else { 208 });
            self.loot
                .push(Loot {
                    x: e.x,
                    y: e.y,
                    kind: if self.rng.f() < 0.16 {
                        LootKind::Repair
                    } else {
                        LootKind::Core
                    },
                });
        }
    }
    fn update_loot(&mut self, dt: f32) {
        let mut collected = vec![];
        for l in &mut self.loot {
            l.y += dt * 5.;
            let dx = self.x - l.x;
            let dy = self.y - l.y;
            if dx.abs() < 13. && dy.abs() < 7. {
                l.x += dx * dt * 4.;
                l.y += dy * dt * 4.;
            }
            if dx.abs() < 3. && dy.abs() < 2. {
                collected.push(l.kind);
                l.y = H + 10.;
            }
        }
        self.loot.retain(|l| l.y < H + 2.);
        for kind in collected {
            match kind {
                LootKind::Core => {
                    self.score += 50;
                    self.nova = (self.nova + 3.).min(100.);
                }
                LootKind::Repair => {
                    self.hull = (self.hull + 14.).min(self.max_hull);
                    self.burst(self.x, self.y, 10, 121);
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_blocks_damage_and_has_cooldown() {
        let mut g = Game::new(1, 0);
        g.start();
        g.phase_drive();
        g.hurt(50.);
        assert_eq!(g.hull, 100.);
        g.tick(1., Input::default());
        g.hurt(12.);
        assert_eq!(g.hull, 88.);
        let cd = g.phase_cd;
        g.phase_drive();
        assert_eq!(g.phase_cd, cd);
    }
    #[test]
    fn pause_freezes_simulation() {
        let mut g = Game::new(1, 0);
        g.start();
        g.mode = Mode::Paused;
        g.tick(3., Input { x: 1., y: 1. });
        assert_eq!(g.sector_time, 0.);
        assert_eq!(g.x, W / 2.);
    }
    #[test]
    fn guardians_and_upgrades_complete_the_voyage() {
        let mut g = Game::new(1, 0);
        g.start();
        for sector in 1..=3 {
            g.sector_time = SECTOR_TIME;
            g.tick(0.02, Input::default());
            assert!(g.boss.is_some());
            assert_eq!(g.sector, sector);
            g.boss.as_mut().unwrap().hp = 0.;
            g.tick(0.02, Input::default());
            if sector < 3 {
                assert_eq!(g.mode, Mode::Upgrade);
                g.upgrade(sector);
                assert_eq!(g.mode, Mode::Playing);
            } else {
                assert_eq!(g.mode, Mode::Won);
            }
        }
        assert!(g.score >= 15000);
    }
    #[test]
    fn nova_clears_hostile_shots_and_spends_charge() {
        let mut g = Game::new(1, 0);
        g.start();
        g.nova = 100.;
        g.shots
            .push(Shot {
                x: 1.,
                y: 1.,
                vx: 0.,
                vy: 1.,
                friendly: false,
                damage: 12.,
            });
        g.discharge();
        assert!(g.shots.is_empty());
        assert_eq!(g.nova, 0.);
        assert!(g.invuln > 0.);
    }
    #[test]
    fn movement_is_bounded() {
        let mut g = Game::new(1, 0);
        g.start();
        for _ in 0..500 {
            g.tick(1. / 60., Input { x: -1., y: 1. });
        }
        assert!(g.x >= 3.);
        assert!(g.y <= H - 3.);
    }
    #[test]
    fn long_simulation_stays_bounded() {
        let mut g = Game::new(6543, 0);
        g.start();
        for i in 0..24000 {
            g.hull = 100.;
            g.invuln = 1.;
            let tx = g
                .boss
                .map(|b| b.x)
                .unwrap_or(W / 2. + (i as f32 * 0.005).sin() * 25.);
            let input = Input {
                x: ((tx - g.x) * 0.5).clamp(-1., 1.),
                y: 0.,
            };
            g.tick(1. / 60., input);
            if g.nova >= 100. {
                g.discharge();
            }
            if g.mode == Mode::Upgrade {
                g.upgrade(1);
            }
            assert!(g.shots.len() < 1000);
            assert!(g.particles.len() < 1500);
            assert!(g.x.is_finite());
            if g.mode == Mode::Won {
                return;
            }
        }
        panic!("Autopilot should finish the voyage");
    }
}
