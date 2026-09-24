use std::io::{self, Write};
use crossterm::{
    cursor::MoveTo, queue, style::{Color, SetBackgroundColor, SetForegroundColor, Print},
};
use crate::game::*;
pub const WIDTH: usize = 120;
pub const HEIGHT: usize = 65;
const BG: u8 = 233;
const FIELD: u8 = 232;
const WHITE: u8 = 189;
const DIM: u8 = 60;
const CYAN: u8 = 117;
const GOLD: u8 = 222;
const RED: u8 = 203;
#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: u8,
    pub bg: u8,
}
pub struct Canvas {
    pub cells: Vec<Cell>,
}
impl Canvas {
    pub fn new() -> Self {
        Self {
            cells: vec![Cell { ch:' ', fg : WHITE, bg : BG }; WIDTH * HEIGHT],
        }
    }
    pub fn put(&mut self, x: i32, y: i32, ch: char, fg: u8) {
        if x >= 0 && y >= 0 && x < WIDTH as i32 && y < HEIGHT as i32 {
            let c = &mut self.cells[y as usize * WIDTH + x as usize];
            c.ch = ch;
            c.fg = fg;
        }
    }
    pub fn text(&mut self, x: i32, y: i32, s: &str, fg: u8) {
        for (i, ch) in s.chars().enumerate() {
            self.put(x + i as i32, y, ch, fg);
        }
    }
    pub fn center(&mut self, y: i32, s: &str, fg: u8) {
        self.text((WIDTH as i32 - s.chars().count() as i32) / 2, y, s, fg);
    }
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, bg: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                if xx >= 0 && yy >= 0 && xx < WIDTH as i32 && yy < HEIGHT as i32 {
                    self.cells[yy as usize * WIDTH + xx as usize] = Cell {
                        ch: ' ',
                        fg: WHITE,
                        bg,
                    };
                }
            }
        }
    }
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, fg: u8) {
        for xx in x + 1..x + w - 1 {
            self.put(xx, y, '─', fg);
            self.put(xx, y + h - 1, '─', fg);
        }
        for yy in y + 1..y + h - 1 {
            self.put(x, yy, '│', fg);
            self.put(x + w - 1, yy, '│', fg);
        }
        for (xx, yy, c) in [
            (x, y, '╭'),
            (x + w - 1, y, '╮'),
            (x, y + h - 1, '╰'),
            (x + w - 1, y + h - 1, '╯'),
        ] {
            self.put(xx, yy, c, fg);
        }
    }
    pub fn bar(&mut self, x: i32, y: i32, n: i32, f: f32, fg: u8) {
        let count = (n as f32 * f.clamp(0., 1.)).round() as i32;
        for i in 0..n {
            self.put(
                x + i,
                y,
                if i < count { '━' } else { '─' },
                if i < count { fg } else { 238 },
            );
        }
    }
    fn field(&mut self, x: i32, y: i32, ch: char, fg: u8) {
        if x >= 0 && x < W as i32 && y >= 0 && y < H as i32 {
            self.put(x + 2, y + 9, ch, fg);
        }
    }
    fn sprite(&mut self, x: f32, y: f32, lines: &[&str], fg: u8) {
        let yy = y.round() as i32 - lines.len() as i32 / 2;
        for (j, line) in lines.iter().enumerate() {
            let xx = x.round() as i32 - line.chars().count() as i32 / 2;
            for (i, ch) in line.chars().enumerate() {
                if ch != ' ' {
                    self.field(xx + i as i32, yy + j as i32, ch, fg);
                }
            }
        }
    }
    fn panel(&mut self, y: i32, label: &str, value: &str, color: u8) {
        self.text(94, y, label, DIM);
        self.text(94, y + 1, value, color);
    }
    fn modal(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.fill(x + 1, y + 1, w, h, 232);
        self.fill(x, y, w, h, BG);
        self.rect(x, y, w, h, 67);
    }
}
pub struct Renderer {
    previous: Vec<Cell>,
    size: (u16, u16),
}
impl Renderer {
    pub fn new() -> Self {
        Self {
            previous: vec![],
            size: (0, 0),
        }
    }
    pub fn invalidate(&mut self) {
        self.previous.clear();
        self.size = (0, 0);
    }
    pub fn draw(
        &mut self,
        out: &mut impl Write,
        c: &Canvas,
        size: (u16, u16),
    ) -> io::Result<()> {
        let ox = (size.0.saturating_sub(WIDTH as u16)) / 2;
        let oy = (size.1.saturating_sub(HEIGHT as u16)) / 2;
        if self.size != size {
            self.previous.clear();
            self.size = size;
            queue!(
                out, SetBackgroundColor(Color::AnsiValue(BG)),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
            )?;
        }
        let mut fg = 255;
        let mut bg = 255;
        for y in 0..HEIGHT {
            let mut x = 0;
            while x < WIDTH {
                let i = y * WIDTH + x;
                let cell = c.cells[i];
                if self.previous.get(i) == Some(&cell) {
                    x += 1;
                    continue;
                }
                queue!(out, MoveTo(ox + x as u16, oy + y as u16))?;
                if fg != cell.fg {
                    queue!(out, SetForegroundColor(Color::AnsiValue(cell.fg)))?;
                    fg = cell.fg;
                }
                if bg != cell.bg {
                    queue!(out, SetBackgroundColor(Color::AnsiValue(cell.bg)))?;
                    bg = cell.bg;
                }
                let mut run = String::new();
                while x < WIDTH {
                    let j = y * WIDTH + x;
                    let n = c.cells[j];
                    if n.fg != fg || n.bg != bg
                        || (self.previous.get(j) == Some(&n) && !run.is_empty())
                    {
                        break;
                    }
                    run.push(n.ch);
                    x += 1;
                }
                queue!(out, Print(run))?;
            }
        }
        out.flush()?;
        self.previous.clone_from(&c.cells);
        Ok(())
    }
}
pub fn compose(g: &Game) -> Canvas {
    let mut c = Canvas::new();
    if g.mode == Mode::Title {
        title(&mut c, g);
        return c;
    }
    c.text(2, 1, "V E S P E R", CYAN);
    c.text(17, 1, "/  a passage through the dark", DIM);
    c.text(93, 1, &format!("FLIGHT {:02}  /  03", g.sector), GOLD);
    c.text(2, 4, "HULL", DIM);
    c.bar(8, 4, 23, g.hull / g.max_hull, if g.hull < 35. { RED } else { CYAN });
    c.text(
        33,
        4,
        &format!("{:03}/{:03}", g.hull.ceil() as u32, g.max_hull as u32),
        WHITE,
    );
    c.text(48, 4, "PHASE", DIM);
    c.bar(55, 4, 12, 1. - g.phase_cd / (4.2 - g.engine as f32 * 0.7).max(2.1), CYAN);
    c.text(
        69,
        4,
        if g.phase_cd <= 0. { "READY" } else { "CYCLE" },
        if g.phase_cd <= 0. { CYAN } else { DIM },
    );
    c.text(83, 4, "NOVA", DIM);
    c.bar(89, 4, 16, g.nova / 100., GOLD);
    c.text(108, 4, &format!("{:3}%", g.nova as u32), GOLD);
    c.rect(1, 8, 88, 52, if g.flash > 0. { RED } else { 60 });
    c.fill(2, 9, 86, 50, FIELD);
    c.text(4, 8, &format!(" 0{} / {} ", g.sector, g.sector_name()), CYAN);
    c.text(69, 8, " FORWARD SCAN ", DIM);
    c.rect(91, 8, 28, 52, 60);
    c.text(94, 8, " FLIGHT TELEMETRY ", DIM);
    field(&mut c, g);
    sidebar(&mut c, g);
    c.text(3, 61, "WASD / ↑↓←→  Move", WHITE);
    c.text(29, 61, "SPACE  Phase", CYAN);
    c.text(51, 61, "X  Nova", GOLD);
    c.text(68, 61, "P / ESC  Pause", DIM);
    c.text(96, 61, "AUTO-FIRE  ON", CYAN);
    c.text(3, 63, "◇  salvage     +  repair     •  hostile fire", DIM);
    c.text(76, 63, "VESPER REACH  /  DEEP SPACE TRANSIT", DIM);
    match g.mode {
        Mode::Paused => pause(&mut c),
        Mode::Upgrade => upgrade(&mut c, g),
        Mode::Lost => ending(&mut c, g, false),
        Mode::Won => ending(&mut c, g, true),
        _ => {}
    }
    c
}
fn field(c: &mut Canvas, g: &Game) {
    for s in &g.stars {
        c.field(
            s.x as i32,
            s.y as i32,
            if s.layer == 2 { '·' } else { '.' },
            match s.layer {
                0 => 235,
                1 => 238,
                _ => 60,
            },
        );
    }
    for y in (3..50).step_by(8) {
        c.field(0, y, '╴', 238);
        c.field(85, y, '╶', 238);
    }
    if let Some(b) = g.boss {
        if b.warning > 0. {
            for y in b.y as i32 + 3..H as i32 {
                if y % 2 == 0 {
                    c.field(
                        b.target as i32,
                        y,
                        '┆',
                        if (b.warning * 8.) as i32 % 2 == 0 { GOLD } else { 130 },
                    );
                }
            }
            c.sprite(b.target, H - 2., &["‹ MOVE ›"], GOLD);
        }
        if b.beam > 0. {
            for y in b.y as i32 + 3..H as i32 {
                for dx in -3..=3 {
                    c.field(
                        b.target as i32 + dx,
                        y,
                        if dx.abs() < 2 { '█' } else { '│' },
                        if dx.abs() < 2 { 223 } else { 208 },
                    );
                }
            }
        }
    }
    for p in &g.particles {
        c.field(
            p.x.round() as i32,
            p.y.round() as i32,
            if p.life / p.max > 0.6 { '✦' } else { '·' },
            if p.life / p.max < 0.3 { 60 } else { p.color },
        );
    }
    for l in &g.loot {
        c.field(
            l.x.round() as i32,
            l.y.round() as i32,
            if l.kind == LootKind::Core { '◇' } else { '+' },
            if l.kind == LootKind::Core { GOLD } else { 121 },
        );
    }
    for e in &g.enemies {
        match e.kind {
            Kind::Scout => c.sprite(e.x, e.y, &["╲◆╱", " ▾ "], 203),
            Kind::Striker => {
                c.sprite(e.x, e.y, &["╔╧╧╧╗", "╚═▼═╝"], 215)
            }
            Kind::Rock => {
                c.sprite(e.x, e.y, &["╭▒╮", "(▓▒▓)", "╰▒╯"], 103)
            }
        }
    }
    if let Some(b) = g.boss {
        c.sprite(
            b.x,
            b.y,
            &[
                "   ╭──╮ ╭──╮   ",
                "╭══╧══╧═╧══╧══╮",
                "╰◀═══╣◆╠═══▶╯",
                "   ╲╲ ▾ ╱╱   ",
            ],
            if b.hp < b.max_hp * 0.4 { RED } else { GOLD },
        );
        c.text(7, 10, g.boss_name(), GOLD);
        c.bar(25, 10, 55, b.hp / b.max_hp, RED);
    }
    for s in &g.shots {
        c.field(
            s.x.round() as i32,
            s.y.round() as i32,
            if s.friendly { '│' } else { '●' },
            if s.friendly { 117 } else { 209 },
        );
    }
    if g.mode != Mode::Lost {
        let color = if g.phase > 0. {
            159
        } else if g.invuln > 0. && (g.time * 12.) as i32 % 2 == 0 {
            60
        } else {
            195
        };
        c.sprite(g.x, g.y, &["  ▲  ", "◁╾█╼▷", " ╵ ╵ "], color);
        if g.phase > 0. {
            c.sprite(g.x, g.y, &["╭     ╮", "       ", "╰     ╯"], 81);
        }
    }
    if g.nova_ring > 0. {
        let r = (0.9 - g.nova_ring) * 100.;
        for i in 0..160 {
            let a = i as f32 * std::f32::consts::TAU / 160.;
            c.field(
                (g.x + a.cos() * r) as i32,
                (g.y + a.sin() * r * 0.5) as i32,
                '·',
                159,
            );
        }
    }
    if g.banner_time > 0. {
        let s = &g.banner;
        let x = 45 - s.chars().count() as i32 / 2;
        c.fill(x - 2, 19, s.chars().count() as i32 + 4, 3, FIELD);
        c.text(x, 20, s, if g.guardian_spawned { GOLD } else { CYAN });
    }
}
fn sidebar(c: &mut Canvas, g: &Game) {
    c.panel(11, "SIGNAL / SCORE", &format!("{:09}", g.score), WHITE);
    c.panel(15, "PERSONAL BEST", &format!("{:09}", g.best.max(g.score)), DIM);
    c.panel(
        19,
        "CHAIN MULTIPLIER",
        &format!("×{}   /   {:02} contacts", 1 + (g.combo / 5).min(4), g.combo),
        GOLD,
    );
    c.bar(94, 22, 21, g.combo_time / 4., GOLD);
    c.text(94, 25, "TRANSIT", DIM);
    for i in 1..=3 {
        let col = if i <= g.sector { CYAN } else { 239 };
        c.text(
            94 + (i as i32 - 1) * 8,
            27,
            &format!("{} {:02}", if i < g.sector { "◆" } else { "◇" }, i),
            col,
        );
    }
    c.bar(94, 29, 21, g.sector_time / SECTOR_TIME, CYAN);
    c.text(
        94,
        31,
        if g.boss.is_some() {
            "Neutralize the guardian."
        } else {
            "Survive the crossing."
        },
        WHITE,
    );
    c.text(
        94,
        33,
        if g.boss.is_some() {
            "Gold line? Phase away."
        } else {
            "Next: guardian contact"
        },
        DIM,
    );
    c.text(94, 36, "SHIP SYSTEMS", DIM);
    c.text(94, 38, &format!("LANCE     MK {}", g.weapon + 1), CYAN);
    c.text(94, 40, &format!("DRIVE     MK {}", g.engine + 1), CYAN);
    c.text(94, 43, "FLIGHT NOTES", DIM);
    let notes = if g.nova >= 100. {
        ["Nova capacitor full.", "Press X to clear fire."]
    } else if g.hull < 35. {
        ["Hull integrity low.", "Seek green + repairs."]
    } else {
        ["Phase ignores damage.", "Salvage pulls nearby."]
    };
    for (i, n) in notes.iter().enumerate() {
        c.text(94, 45 + i as i32, n, if g.hull < 35. { RED } else { WHITE });
    }
    c.text(94, 50, "COMMS / LATEST", DIM);
    if let Some(line) = g.feed.last() {
        let mut row = 52;
        let mut s = String::new();
        for word in line.split_whitespace() {
            if s.len() + word.len() + 1 > 22 {
                c.text(94, row, &s, 67);
                row += 1;
                s.clear();
            }
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(word);
        }
        c.text(94, row, &s, 67);
    }
    c.text(94, 57, "●  UPLINK ESTABLISHED", 60);
}
fn title(c: &mut Canvas, g: &Game) {
    for s in &g.stars {
        let x = (s.x / W * 116.) as i32 + 2;
        let y = (s.y / H * 61.) as i32 + 2;
        c.put(x, y, '·', if s.layer == 2 { 60 } else { 237 });
    }
    c.rect(0, 0, 120, 65, 60);
    c.text(4, 2, "DEEP SPACE TRANSIT AUTHORITY", DIM);
    c.text(93, 2, "FLIGHT LOG / 001", DIM);
    c.center(8, "T H E   L A S T   R O U T E   H O M E", GOLD);
    let logo = [
        "██╗   ██╗███████╗███████╗██████╗ ███████╗██████╗ ",
        "██║   ██║██╔════╝██╔════╝██╔══██╗██╔════╝██╔══██╗",
        "██║   ██║█████╗  ███████╗██████╔╝█████╗  ██████╔╝",
        "╚██╗ ██╔╝██╔══╝  ╚════██║██╔═══╝ ██╔══╝  ██╔══██╗",
        " ╚████╔╝ ███████╗███████║██║     ███████╗██║  ██║",
        "  ╚═══╝  ╚══════╝╚══════╝╚═╝     ╚══════╝╚═╝  ╚═╝",
    ];
    for (i, line) in logo.iter().enumerate() {
        c.center(12 + i as i32, line, if i < 3 { 153 } else { 67 });
    }
    c.center(21, "A small ship. Three guardians. One way through.", WHITE);
    let ship = [
        "             ▲             ",
        "            ╱█╲            ",
        "           ╱███╲           ",
        "       ╱╲ ╱██╬██╲ ╱╲       ",
        "      ╱██╳███╬███╳██╲      ",
        "     ╱═══════╬═══════╲     ",
        "    ◁████╱╲█████╱╲████▷    ",
        "     ╲__╱  ╲███╱  ╲__╱     ",
        "       ╵    ╲█╱    ╵       ",
        "             ▒             ",
        "             ░             ",
    ];
    for (i, line) in ship.iter().enumerate() {
        c.center(25 + i as i32, line, if i > 8 { 67 } else { 117 });
    }
    c.center(39, "Cross the shoals. Break the blockade. Bring the light home.", DIM);
    c.fill(43, 43, 34, 3, 235);
    c.text(
        47,
        44,
        "[ ENTER ]  BEGIN THE VOYAGE",
        if (g.time * 1.5) as i32 % 2 == 0 { 195 } else { 117 },
    );
    c.center(49, "WASD / ARROWS   steer       SPACE   phase       X   nova", WHITE);
    c.center(
        51,
        "Your lances fire automatically. Keep moving; collect the fragments.",
        DIM,
    );
    c.center(55, "3 SECTORS  ·  SHIP UPGRADES  ·  GUARDIAN ENCOUNTERS", 67);
    if g.best > 0 {
        c.center(58, &format!("PERSONAL BEST   {:09}", g.best), GOLD);
    }
    c.text(4, 62, "HEADPHONES OPTIONAL. IMAGINATION REQUIRED.", DIM);
    c.text(104, 62, "Q  leave", DIM);
}
fn pause(c: &mut Canvas) {
    c.modal(31, 21, 58, 23);
    c.center(24, "F L I G H T   S U S P E N D E D", CYAN);
    c.center(28, "The dark can wait.", WHITE);
    c.center(31, "WASD / ARROWS     steer", WHITE);
    c.center(33, "SPACE     phase through danger", CYAN);
    c.center(35, "X     release a fully charged nova", GOLD);
    c.center(39, "P / ESC / ENTER  resume     Q  leave", DIM);
}
fn upgrade(c: &mut Canvas, g: &Game) {
    c.modal(11, 15, 98, 36);
    c.center(18, "G U A R D I A N   S I L E N C E D", GOLD);
    c.center(21, &format!("{} falls quiet. The route opens.", g.boss_name()), WHITE);
    c.center(23, "Choose a refit for the crossing ahead.", DIM);
    let cards = [
        ("1", "LANCE ARRAY", ["Add spread fire.", "Increase lance damage."]),
        ("2", "HULL PLATING", ["+30 maximum hull.", "More room for mistakes."]),
        ("3", "PHASE ENGINE", ["Shorter phase cooldown.", "Faster cruising speed."]),
    ];
    for (i, (key, name, lines)) in cards.iter().enumerate() {
        let x = 15 + i as i32 * 31;
        c.rect(x, 27, 28, 13, 60);
        c.text(x + 3, 29, &format!("[ {} ]", key), GOLD);
        c.text(x + 3, 32, name, CYAN);
        for (j, s) in lines.iter().enumerate() {
            c.text(x + 3, 35 + j as i32, s, WHITE);
        }
    }
    c.center(43, "Every refit restores 35 hull and adds 30% nova charge.", 117);
    c.center(47, "PRESS 1, 2 OR 3 TO REFIT & CONTINUE", GOLD);
}
fn ending(c: &mut Canvas, g: &Game, won: bool) {
    c.modal(24, 17, 72, 33);
    c.center(
        20,
        if won { "T H E   L I G H T   I S   H O M E" } else { "S I G N A L   L O S T" },
        if won { GOLD } else { RED },
    );
    c.center(
        24,
        if won {
            "Beyond the gate, the stars look familiar."
        } else {
            "The Reach keeps another little light."
        },
        WHITE,
    );
    c.center(
        26,
        if won {
            "You made a path through the dark."
        } else {
            "There is always another ship. Another crossing."
        },
        DIM,
    );
    c.center(30, &format!("FINAL SCORE     {:09}", g.score), GOLD);
    c.center(33, &format!("CONTACTS CLEARED    {:03}", g.kills), WHITE);
    c.center(35, &format!("SECTORS REACHED     {} / 3", g.sector), WHITE);
    c.center(38, &format!("PERSONAL BEST    {:09}", g.best.max(g.score)), CYAN);
    c.center(43, "[ ENTER / R ]  FLY AGAIN", WHITE);
    c.center(46, "Q  leave the Reach", DIM);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_screen_fits_the_canvas() {
        let mut g = Game::new(1, 500);
        for mode in [
            Mode::Title,
            Mode::Playing,
            Mode::Paused,
            Mode::Upgrade,
            Mode::Lost,
            Mode::Won,
        ] {
            g.mode = mode;
            let c = compose(&g);
            assert_eq!(c.cells.len(), WIDTH * HEIGHT);
            assert!(c.cells.iter().all(| c |! c.ch.is_control()));
        }
    }
    #[test]
    fn field_clips_entities_at_the_border() {
        let mut c = Canvas::new();
        c.sprite(-1., -1., &["ABCDE", "FGHIJ", "KLMNO"], 117);
        assert_eq!(c.cells[8 * WIDTH + 1].ch,' ');
        assert_eq!(c.cells[9 * WIDTH + 2].ch,'N');
    }
}
