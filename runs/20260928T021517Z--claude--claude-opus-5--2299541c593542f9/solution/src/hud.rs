//! Terminal chrome: the frame around the playfield, the status bars and the
//! boss health readout. Everything here writes cells, not pixels — the glowing
//! banner text lives in the canvas instead.

use crate::canvas::Rgb;
use crate::entities::Kind;
use crate::game::{Game, Mode};
use crate::term::{Cell, Screen};

/// Where the playfield sits inside the terminal.
#[derive(Clone, Copy)]
pub struct Layout {
    pub cols: usize,
    pub rows: usize,
    pub pf_col: usize,
    pub pf_row: usize,
    pub pf_w: usize,
    pub pf_rows: usize,
}

pub const MIN_COLS: usize = 60;
pub const MIN_ROWS: usize = 22;

impl Layout {
    pub fn new(cols: usize, rows: usize) -> Self {
        Layout {
            cols,
            rows,
            pf_col: 1,
            pf_row: 2,
            pf_w: cols.saturating_sub(2),
            pf_rows: rows.saturating_sub(4),
        }
    }

    /// Canvas height in pixels (two per terminal row).
    pub fn pf_h(&self) -> usize {
        self.pf_rows * 2
    }

    pub fn fits(cols: usize, rows: usize) -> bool {
        cols >= MIN_COLS && rows >= MIN_ROWS
    }
}

const BG: Rgb = [0.020, 0.025, 0.042];
const FRAME: Rgb = [0.16, 0.22, 0.34];
const FRAME_HI: Rgb = [0.34, 0.48, 0.66];
const LABEL: Rgb = [0.38, 0.47, 0.60];
const VALUE: Rgb = [0.80, 0.90, 1.00];
const DIM: Rgb = [0.26, 0.32, 0.42];

fn q(s: &Screen, c: Rgb) -> u8 {
    s.palette.quantize(
        (c[0] * 255.0).clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).clamp(0.0, 255.0) as u8,
    )
}

/// A left-to-right cursor for laying out a status bar.
struct Bar<'a> {
    s: &'a mut Screen,
    row: i32,
    col: i32,
    bg: u8,
}

impl Bar<'_> {
    fn put(&mut self, text: &str, fg: Rgb) {
        let fg = q(self.s, fg);
        self.s.text(self.col, self.row, text, fg, self.bg);
        self.col += text.chars().count() as i32;
    }

    fn gauge(&mut self, filled: i32, total: i32, on: Rgb, off: Rgb) {
        let on = q(self.s, on);
        let off = q(self.s, off);
        for i in 0..total {
            let (ch, fg) = if i < filled { ('█', on) } else { ('░', off) };
            self.s.put(self.col, self.row, Cell { ch, fg, bg: self.bg });
            self.col += 1;
        }
    }

    fn pips(&mut self, filled: i32, total: i32, on: Rgb, off: Rgb) {
        let on = q(self.s, on);
        let off = q(self.s, off);
        for i in 0..total {
            let (ch, fg) = if i < filled { ('◆', on) } else { ('◇', off) };
            self.s.put(self.col, self.row, Cell { ch, fg, bg: self.bg });
            self.col += 1;
        }
    }

    fn space(&mut self, n: i32) {
        self.col += n;
    }
}

pub fn draw_chrome(s: &mut Screen, g: &Game, lay: Layout) {
    let bg = q(s, BG);
    let frame = q(s, FRAME);
    let frame_hi = q(s, FRAME_HI);

    // Wipe the four chrome rows (the playfield itself is fully overwritten by
    // the canvas blit, so it never needs clearing).
    for row in [0i32, 1, lay.rows as i32 - 2, lay.rows as i32 - 1] {
        for col in 0..lay.cols as i32 {
            s.put(col, row, Cell { ch: ' ', fg: bg, bg });
        }
    }
    // Side rails.
    for row in lay.pf_row..lay.pf_row + lay.pf_rows {
        s.put(0, row as i32, Cell { ch: '│', fg: frame, bg });
        s.put(
            lay.cols as i32 - 1,
            row as i32,
            Cell { ch: '│', fg: frame, bg },
        );
    }
    let top = 1i32;
    let bot = lay.rows as i32 - 2;
    for col in 1..lay.cols as i32 - 1 {
        s.put(col, top, Cell { ch: '─', fg: frame, bg });
        s.put(col, bot, Cell { ch: '─', fg: frame, bg });
    }
    s.put(0, top, Cell { ch: '┌', fg: frame_hi, bg });
    s.put(lay.cols as i32 - 1, top, Cell { ch: '┐', fg: frame_hi, bg });
    s.put(0, bot, Cell { ch: '└', fg: frame_hi, bg });
    s.put(lay.cols as i32 - 1, bot, Cell { ch: '┘', fg: frame_hi, bg });

    draw_top_bar(s, g, lay, bg);
    draw_bottom_bar(s, g, lay, bg);

    if let Some((kind, hp, max_hp)) = g.boss_ref {
        if g.mode == Mode::Play {
            draw_boss_bar(s, lay, kind, hp / max_hp, bg);
        }
    }
}

fn draw_top_bar(s: &mut Screen, g: &Game, lay: Layout, bg: u8) {
    let mut b = Bar {
        s,
        row: 0,
        col: 1,
        bg,
    };
    b.put("SCORE ", LABEL);
    b.put(&format!("{:07}", g.score.min(9_999_999)), VALUE);

    if g.combo >= 5 && g.mode == Mode::Play {
        b.space(2);
        let mult = (1 + g.combo / 5).min(8);
        let heat = ((mult - 1) as f32 / 7.0).clamp(0.0, 1.0);
        let c = [1.0, 0.95 - heat * 0.55, 0.35 - heat * 0.30];
        b.put(&format!("x{mult} CHAIN {}", g.combo), c);
    }

    // Centre: the game title, or the wave number once a run is underway.
    let centre = if g.mode == Mode::Title {
        "N E B U L A   D R I F T".to_string()
    } else {
        format!("WAVE {:02}", g.wave.number)
    };
    let cx = (lay.cols as i32 - centre.chars().count() as i32) / 2;
    let fg = q(s, if g.mode == Mode::Title { VALUE } else { FRAME_HI });
    s.text(cx, 0, &centre, fg, bg);

    let right = format!("BEST {:07}", g.high.min(9_999_999));
    let rx = lay.cols as i32 - 1 - right.chars().count() as i32;
    let lab = q(s, LABEL);
    let val = q(s, if g.new_high { [1.0, 0.85, 0.35] } else { VALUE });
    s.text(rx, 0, "BEST ", lab, bg);
    s.text(rx + 5, 0, &format!("{:07}", g.high.min(9_999_999)), val, bg);
}

fn draw_bottom_bar(s: &mut Screen, g: &Game, lay: Layout, bg: u8) {
    let row = lay.rows as i32 - 1;
    let p = &g.player;

    let hull_frac = p.hull as f32 / p.max_hull.max(1) as f32;
    let hull_col: Rgb = if hull_frac > 0.6 {
        [0.35, 0.95, 0.55]
    } else if hull_frac > 0.3 {
        [1.00, 0.80, 0.25]
    } else {
        [1.00, 0.32, 0.28]
    };

    let mut b = Bar {
        s,
        row,
        col: 1,
        bg,
    };
    b.put("HULL ", LABEL);
    b.gauge(p.hull, p.max_hull, hull_col, DIM);
    b.space(2);
    b.put("SHIELD ", LABEL);
    b.gauge(
        p.shield.floor() as i32,
        p.max_shield as i32,
        [0.35, 0.78, 1.00],
        DIM,
    );
    b.space(2);
    b.put("WEAPON ", LABEL);
    b.gauge(
        p.weapon as i32,
        crate::entities::MAX_WEAPON as i32,
        [1.00, 0.85, 0.30],
        DIM,
    );
    b.space(2);
    b.put("BOMB ", LABEL);
    b.pips(p.bombs as i32, p.max_bombs as i32, [1.00, 0.55, 0.35], DIM);

    // Right-aligned control reminder, trimmed to whatever space is left.
    let hint = if g.autofire {
        "WASD/ARROWS move   X bomb   F autofire:ON   P pause   Q quit"
    } else {
        "WASD/ARROWS move   SPACE fire   X bomb   F autofire:OFF   P pause   Q quit"
    };
    let hx = lay.cols as i32 - 1 - hint.chars().count() as i32;
    if hx > b.col + 2 {
        let fg = q(s, DIM);
        s.text(hx, row, hint, fg, bg);
    }
}

fn draw_boss_bar(s: &mut Screen, lay: Layout, kind: Kind, frac: f32, bg: u8) {
    let name = kind.name();
    let width = ((lay.cols as i32 - 20).clamp(10, 46)) as usize;
    // Leading and trailing spaces lift the readout off the border rule.
    let total = name.chars().count() + 5 + width;
    let x0 = (lay.cols as i32 - total as i32) / 2;
    let row = 1;

    let tint = kind.tint();
    let name_fg = q(s, [tint[0], tint[1] * 0.9, tint[2]]);
    s.put(x0, row, Cell { ch: ' ', fg: bg, bg });
    s.text(x0 + 1, row, name, name_fg, bg);

    let mut col = x0 + 1 + name.chars().count() as i32;
    s.put(col, row, Cell { ch: ' ', fg: bg, bg });
    col += 1;
    let filled = (frac.clamp(0.0, 1.0) * width as f32).round() as usize;
    // The bar shifts from violet to red as the hull gives way.
    let hot: Rgb = [1.0, 0.30, 0.25];
    let cool: Rgb = tint;
    let on = q(s, crate::canvas::lerp_rgb(hot, cool, frac.clamp(0.0, 1.0)));
    let off = q(s, [0.18, 0.12, 0.24]);
    let edge = q(s, [0.35, 0.28, 0.45]);
    s.put(col, row, Cell { ch: '┤', fg: edge, bg });
    col += 1;
    for i in 0..width {
        let (ch, fg) = if i < filled { ('█', on) } else { ('░', off) };
        s.put(col, row, Cell { ch, fg, bg });
        col += 1;
    }
    s.put(col, row, Cell { ch: '├', fg: edge, bg });
    s.put(col + 1, row, Cell { ch: ' ', fg: bg, bg });
}

/// Shown when the terminal is too small to lay the game out.
pub fn draw_too_small(s: &mut Screen, cols: usize, rows: usize) {
    let bg = s.palette.quantize(8, 8, 14);
    let fg = s.palette.quantize(220, 230, 255);
    let dim = s.palette.quantize(110, 125, 150);
    s.clear(Cell { ch: ' ', fg: bg, bg });
    let lines = [
        "NEBULA DRIFT".to_string(),
        String::new(),
        format!("terminal is {cols}x{rows}"),
        format!("please resize to at least {MIN_COLS}x{MIN_ROWS}"),
        "(124x69 recommended)".to_string(),
        String::new(),
        "Q to quit".to_string(),
    ];
    let top = (rows as i32 - lines.len() as i32) / 2;
    for (i, l) in lines.iter().enumerate() {
        let x = (cols as i32 - l.chars().count() as i32) / 2;
        s.text(
            x,
            top + i as i32,
            l,
            if i == 0 { fg } else { dim },
            bg,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_target_terminal() {
        let l = Layout::new(124, 69);
        assert_eq!(l.pf_w, 122);
        assert_eq!(l.pf_rows, 65);
        assert_eq!(l.pf_h(), 130);
        // Two chrome rows on top, two below.
        assert_eq!(l.pf_row + l.pf_rows + 2, l.rows);
    }

    #[test]
    fn chrome_draws_a_closed_frame() {
        let mut s = Screen::new(124, 69);
        let g = Game::new(122, 130, 0);
        let lay = Layout::new(124, 69);
        draw_chrome(&mut s, &g, lay);
        assert_eq!(s.get(0, 1).ch, '┌');
        assert_eq!(s.get(123, 1).ch, '┐');
        assert_eq!(s.get(0, 67).ch, '└');
        assert_eq!(s.get(123, 67).ch, '┘');
        assert_eq!(s.get(0, 30).ch, '│');
        assert_eq!(s.get(123, 30).ch, '│');
        assert_eq!(s.get(40, 1).ch, '─');
    }

    #[test]
    fn status_bars_render_within_the_terminal() {
        let mut s = Screen::new(124, 69);
        let mut g = Game::new(122, 130, 0);
        g.start_run();
        g.score = 1234;
        g.combo = 25;
        let lay = Layout::new(124, 69);
        draw_chrome(&mut s, &g, lay);
        let text = s.to_text();
        let first = text.lines().next().unwrap();
        assert!(first.contains("SCORE 0001234"), "{first}");
        assert!(first.contains("BEST"), "{first}");
        let last = text.lines().last().unwrap();
        assert!(last.contains("HULL"), "{last}");
        assert!(last.contains("BOMB"), "{last}");
        assert!(last.contains("quit"), "{last}");
    }

    #[test]
    fn small_but_valid_terminals_still_lay_out() {
        for (c, r) in [(60usize, 22usize), (80, 24), (100, 40), (124, 69), (200, 90)] {
            assert!(Layout::fits(c, r));
            let lay = Layout::new(c, r);
            assert!(lay.pf_w > 0 && lay.pf_rows > 0);
            let mut s = Screen::new(c, r);
            let g = Game::new(lay.pf_w, lay.pf_h(), 0);
            draw_chrome(&mut s, &g, lay);
        }
    }
}
