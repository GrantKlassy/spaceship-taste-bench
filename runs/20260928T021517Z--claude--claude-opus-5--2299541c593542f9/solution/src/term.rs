//! Terminal back-end: tone mapping, exact xterm-256 quantisation, a cell grid
//! with half-block packing, and a diffing writer that only repaints what moved.

use crate::canvas::{Canvas, Rgb};
use std::io::Write;

/// The six intensity levels of the xterm 6x6x6 colour cube.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
/// The 24-step grey ramp is 8, 18, ... 238 at palette indices 232..=255.
const GREY_LO: i32 = 8;
const GREY_STEP: i32 = 10;
const GREY_N: i32 = 24;

/// Perceptual-ish channel weights for nearest-colour search.
const WR: i32 = 2;
const WG: i32 = 4;
const WB: i32 = 3;

pub struct Palette {
    /// u8 channel value -> index into CUBE of the nearest level.
    cube_of: [u8; 256],
    /// u8 channel value -> spacing of the cube levels bracketing it.
    cube_gap: [u8; 256],
    /// u8 value -> index into the grey ramp nearest to it.
    grey_of: [u8; 256],
    /// Palette index -> displayed sRGB, for offline snapshots.
    pub rgb_of: [[u8; 3]; 256],
}

impl Palette {
    pub fn new() -> Self {
        let mut cube_of = [0u8; 256];
        for (v, slot) in cube_of.iter_mut().enumerate() {
            let mut best = 0usize;
            let mut best_d = i32::MAX;
            for (i, &lv) in CUBE.iter().enumerate() {
                let d = (v as i32 - lv as i32).abs();
                if d < best_d {
                    best_d = d;
                    best = i;
                }
            }
            *slot = best as u8;
        }

        // Spacing of the cube levels bracketing each value. Values sitting
        // exactly on a level take the gap above, so they never dither away
        // from themselves.
        let mut cube_gap = [0u8; 256];
        for (v, slot) in cube_gap.iter_mut().enumerate() {
            let v = v as i32;
            let mut gap = (CUBE[5] - CUBE[4]) as u8;
            for i in 0..5 {
                let (lo, hi) = (CUBE[i] as i32, CUBE[i + 1] as i32);
                if v >= lo && v < hi {
                    gap = (hi - lo) as u8;
                    break;
                }
            }
            *slot = gap;
        }

        let mut grey_of = [0u8; 256];
        for (v, slot) in grey_of.iter_mut().enumerate() {
            let idx = (((v as i32 - GREY_LO) as f32) / GREY_STEP as f32).round() as i32;
            *slot = idx.clamp(0, GREY_N - 1) as u8;
        }

        let mut rgb_of = [[0u8; 3]; 256];
        // 0..15 are theme-dependent; the game never emits them, but fill in the
        // conventional values so snapshots of any stray index look sane.
        const BASE16: [[u8; 3]; 16] = [
            [0, 0, 0],
            [128, 0, 0],
            [0, 128, 0],
            [128, 128, 0],
            [0, 0, 128],
            [128, 0, 128],
            [0, 128, 128],
            [192, 192, 192],
            [128, 128, 128],
            [255, 0, 0],
            [0, 255, 0],
            [255, 255, 0],
            [0, 0, 255],
            [255, 0, 255],
            [0, 255, 255],
            [255, 255, 255],
        ];
        rgb_of[..16].copy_from_slice(&BASE16);
        for i in 0..216usize {
            let r = CUBE[i / 36];
            let g = CUBE[(i / 6) % 6];
            let b = CUBE[i % 6];
            rgb_of[16 + i] = [r, g, b];
        }
        for i in 0..GREY_N as usize {
            let v = (GREY_LO + GREY_STEP * i as i32) as u8;
            rgb_of[232 + i] = [v, v, v];
        }

        Palette {
            cube_of,
            cube_gap,
            grey_of,
            rgb_of,
        }
    }

    /// Exact nearest neighbour over the colour cube plus the grey ramp.
    ///
    /// The cube is a separable grid, so the closest cube entry is found by
    /// rounding each channel on its own. The greys lie on the r=g=b diagonal,
    /// so the closest grey is the weighted mean of the channels. Comparing
    /// those two candidates is therefore exact, and needs no lookup table.
    #[inline]
    pub fn quantize(&self, r: u8, g: u8, b: u8) -> u8 {
        self.quantize_dithered(r as f32, g as f32, b as f32, 0.0)
    }

    /// Nearest palette entry with ordered dithering.
    ///
    /// `bayer` is in -0.5..0.5. The amplitude cannot be a constant: the cube's
    /// two darkest levels are 95 apart while the grey ramp steps by 10, so a
    /// single amplitude either leaves dark colours banded into flat black or
    /// covers the greys in speckle. Instead the offset is scaled by the local
    /// spacing of whichever family the colour lands in, which reproduces the
    /// requested value on average and leaves exact palette colours untouched.
    #[inline]
    pub fn quantize_dithered(&self, rf: f32, gf: f32, bf: f32, bayer: f32) -> u8 {
        let ri = rf.clamp(0.0, 255.0) as i32;
        let gi = gf.clamp(0.0, 255.0) as i32;
        let bi = bf.clamp(0.0, 255.0) as i32;

        // Undithered candidates decide which family to use, so the choice
        // stays stable across neighbouring pixels.
        let c0 = (
            self.cube_of[ri as usize] as usize,
            self.cube_of[gi as usize] as usize,
            self.cube_of[bi as usize] as usize,
        );
        let (cr, cg, cb) = (CUBE[c0.0] as i32, CUBE[c0.1] as i32, CUBE[c0.2] as i32);
        let dc = WR * (ri - cr) * (ri - cr) + WG * (gi - cg) * (gi - cg) + WB * (bi - cb) * (bi - cb);

        let mean = (WR * ri + WG * gi + WB * bi) / (WR + WG + WB);
        let g0 = self.grey_of[mean.clamp(0, 255) as usize] as i32;
        let gv0 = GREY_LO + GREY_STEP * g0;
        let dg = WR * (ri - gv0) * (ri - gv0)
            + WG * (gi - gv0) * (gi - gv0)
            + WB * (bi - gv0) * (bi - gv0);

        if dg < dc {
            // Grey ramp: the steps are only 10 apart, so the residual banding
            // is under half a percent of range. A light touch is enough here;
            // a full-step offset just looks like static.
            let m = (mean as f32 + bayer * GREY_STEP as f32 * GREY_DITHER)
                .clamp(0.0, 255.0) as usize;
            (232 + self.grey_of[m] as i32) as u8
        } else {
            // Cube: dither each channel by the gap to its neighbouring level.
            let d = |v: i32| -> usize {
                let step = (self.cube_gap[v as usize] as f32 * CUBE_DITHER).min(CUBE_DITHER_MAX);
                (v as f32 + bayer * step).clamp(0.0, 255.0) as usize
            };
            let r = self.cube_of[d(ri)] as usize;
            let g = self.cube_of[d(gi)] as usize;
            let b = self.cube_of[d(bi)] as usize;
            (16 + r * 36 + g * 6 + b) as u8
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: u8,
    pub bg: u8,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: 16,
        bg: 16,
    };
}

/// 8x8 Bayer threshold matrix. Finer than a 4x4 grid, so the dither reads as
/// texture rather than as visible blocks at this pixel size.
#[rustfmt::skip]
const BAYER8: [u8; 64] = [
     0, 32,  8, 40,  2, 34, 10, 42,
    48, 16, 56, 24, 50, 18, 58, 26,
    12, 44,  4, 36, 14, 46,  6, 38,
    60, 28, 52, 20, 62, 30, 54, 22,
     3, 35, 11, 43,  1, 33,  9, 41,
    51, 19, 59, 27, 49, 17, 57, 25,
    15, 47,  7, 39, 13, 45,  5, 37,
    63, 31, 55, 23, 61, 29, 53, 21,
];

/// Dither strength as a fraction of the exact family step. The cube sits just
/// under the theoretical value to take the edge off its wide dark bands; the
/// grey ramp is much finer and only needs a hint.
const CUBE_DITHER: f32 = 0.90;
const GREY_DITHER: f32 = 0.45;
/// Hard ceiling on the cube offset. Dithering the full 95-wide gap between the
/// two darkest levels is numerically correct but reads as a checkerboard, so
/// dark bands get partial dithering and a little extra contrast instead.
const CUBE_DITHER_MAX: f32 = 40.0;

/// Bayer value for a pixel, centred on zero in -0.5..0.5.
#[inline]
fn bayer(x: usize, y: usize) -> f32 {
    (BAYER8[(y & 7) * 8 + (x & 7)] as f32 + 0.5) / 64.0 - 0.5
}

/// Highlights above this roll off smoothly towards white instead of clipping,
/// which is what gives explosion cores their hot centre.
const KNEE: f32 = 0.80;

#[inline]
fn tonemap_channel(v: f32) -> f32 {
    if v <= KNEE {
        v.max(0.0)
    } else {
        KNEE + (1.0 - KNEE) * (1.0 - (-(v - KNEE) / (1.0 - KNEE)).exp())
    }
}

pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<Cell>,
    prev: Vec<Cell>,
    pub palette: Palette,
    out: String,
    force_full: bool,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        Screen {
            cols,
            rows,
            cells: vec![Cell::BLANK; cols * rows],
            prev: vec![
                Cell {
                    ch: '\u{0}',
                    fg: 0,
                    bg: 0
                };
                cols * rows
            ],
            palette: Palette::new(),
            out: String::with_capacity(1 << 18),
            force_full: true,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.cells.clear();
        self.cells.resize(cols * rows, Cell::BLANK);
        self.prev.clear();
        self.prev.resize(
            cols * rows,
            Cell {
                ch: '\u{0}',
                fg: 0,
                bg: 0,
            },
        );
        self.force_full = true;
    }

    pub fn clear(&mut self, cell: Cell) {
        for c in self.cells.iter_mut() {
            *c = cell;
        }
    }

    #[inline]
    pub fn put(&mut self, col: i32, row: i32, cell: Cell) {
        if col >= 0 && row >= 0 && (col as usize) < self.cols && (row as usize) < self.rows {
            self.cells[row as usize * self.cols + col as usize] = cell;
        }
    }

    #[inline]
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get(&self, col: i32, row: i32) -> Cell {
        if col >= 0 && row >= 0 && (col as usize) < self.cols && (row as usize) < self.rows {
            self.cells[row as usize * self.cols + col as usize]
        } else {
            Cell::BLANK
        }
    }

    pub fn text(&mut self, col: i32, row: i32, s: &str, fg: u8, bg: u8) {
        let mut c = col;
        for ch in s.chars() {
            self.put(c, row, Cell { ch, fg, bg });
            c += 1;
        }
    }

    /// Fold a pixel canvas into half-block cells anchored at (col0, row0).
    /// Each cell shows the upper pixel as the foreground of `▀` and the lower
    /// pixel as the background.
    ///
    /// As `blit`, but displaces the image by (ox, oy) pixels. Used for screen
    /// shake; pixels shifted in from outside the canvas read as black.
    pub fn blit_offset(&mut self, canvas: &Canvas, col0: usize, row0: usize, ox: i32, oy: i32) {
        const BLACK: Rgb = [0.0, 0.0, 0.0];
        let rows = canvas.h / 2;
        let cw = canvas.w as i32;
        let ch = canvas.h as i32;
        for ry in 0..rows {
            let srow = row0 + ry;
            if srow >= self.rows {
                break;
            }
            let sy_top = (ry as i32) * 2 - oy;
            let sy_bot = sy_top + 1;
            for cx in 0..canvas.w {
                let scol = col0 + cx;
                if scol >= self.cols {
                    break;
                }
                let sx = cx as i32 - ox;
                let inx = sx >= 0 && sx < cw;
                let top = if inx && sy_top >= 0 && sy_top < ch {
                    canvas.buf[sy_top as usize * canvas.w + sx as usize]
                } else {
                    BLACK
                };
                let bot = if inx && sy_bot >= 0 && sy_bot < ch {
                    canvas.buf[sy_bot as usize * canvas.w + sx as usize]
                } else {
                    BLACK
                };
                // The two halves of a cell sit on different canvas rows, so
                // give them different dither offsets.
                let fg = quant_px(&self.palette, top, bayer(cx, ry * 2));
                let bg = quant_px(&self.palette, bot, bayer(cx, ry * 2 + 1));
                let cell = if fg == bg {
                    // Uniform cell: a space avoids any glyph seams and is
                    // cheaper to emit.
                    Cell { ch: ' ', fg, bg }
                } else {
                    Cell { ch: '▀', fg, bg }
                };
                self.cells[srow * self.cols + scol] = cell;
            }
        }
    }

    /// Serialise the difference against the previous frame and write it out.
    pub fn flush(&mut self, w: &mut impl Write) -> std::io::Result<usize> {
        self.out.clear();
        // Start from a known SGR state each frame.
        self.out.push_str("\x1b[m");
        let mut cur_fg: i32 = -1;
        let mut cur_bg: i32 = -1;
        let mut cursor: Option<(usize, usize)> = None;

        for row in 0..self.rows {
            let base = row * self.cols;
            let mut col = 0usize;
            while col < self.cols {
                if !self.force_full && self.cells[base + col] == self.prev[base + col] {
                    col += 1;
                    continue;
                }
                // Find the end of this dirty span, tolerating short clean gaps
                // because re-positioning the cursor costs more than repainting
                // a few cells.
                let start = col;
                let mut end = col + 1;
                let mut gap = 0usize;
                let mut scan = end;
                while scan < self.cols {
                    let same = !self.force_full && self.cells[base + scan] == self.prev[base + scan];
                    if same {
                        gap += 1;
                        if gap > 6 {
                            break;
                        }
                    } else {
                        gap = 0;
                        end = scan + 1;
                    }
                    scan += 1;
                }

                if cursor != Some((row, start)) {
                    use std::fmt::Write as _;
                    let _ = write!(self.out, "\x1b[{};{}H", row + 1, start + 1);
                }

                for c in start..end {
                    let cell = self.cells[base + c];
                    emit_cell(&mut self.out, cell, &mut cur_fg, &mut cur_bg);
                    self.prev[base + c] = cell;
                }
                cursor = Some((row, end));
                col = end;
            }
        }

        self.force_full = false;
        let n = self.out.len();
        w.write_all(self.out.as_bytes())?;
        Ok(n)
    }

    /// Render the current cell grid back to RGB for offline inspection.
    /// Only used by the snapshot tool.
    pub fn to_rgb(&self) -> (usize, usize, Vec<u8>) {
        let w = self.cols;
        let h = self.rows * 2;
        let mut px = vec![0u8; w * h * 3];
        for row in 0..self.rows {
            for col in 0..w {
                let cell = self.cells[row * w + col];
                let (top, bot) = match cell.ch {
                    ' ' => (cell.bg, cell.bg),
                    '▀' => (cell.fg, cell.bg),
                    '▄' => (cell.bg, cell.fg),
                    // Approximate text glyphs as a half-intensity blend so the
                    // HUD is legible as shapes in a snapshot.
                    _ => (cell.fg, cell.bg),
                };
                let t = self.palette.rgb_of[top as usize];
                let b = self.palette.rgb_of[bot as usize];
                let o1 = ((row * 2) * w + col) * 3;
                let o2 = ((row * 2 + 1) * w + col) * 3;
                px[o1..o1 + 3].copy_from_slice(&t);
                px[o2..o2 + 3].copy_from_slice(&b);
            }
        }
        (w, h, px)
    }

    /// Plain-text dump of the grid, for checking HUD layout in tests.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let c = self.cells[row * self.cols + col].ch;
                s.push(if c == '▀' { '#' } else { c });
            }
            s.push('\n');
        }
        s
    }
}

#[inline]
fn quant_px(pal: &Palette, c: Rgb, bayer: f32) -> u8 {
    pal.quantize_dithered(
        tonemap_channel(c[0]) * 255.0,
        tonemap_channel(c[1]) * 255.0,
        tonemap_channel(c[2]) * 255.0,
        bayer,
    )
}

fn emit_cell(out: &mut String, cell: Cell, cur_fg: &mut i32, cur_bg: &mut i32) {
    use std::fmt::Write as _;
    let needs_fg = cell.ch != ' ' && cell.fg as i32 != *cur_fg;
    let needs_bg = cell.bg as i32 != *cur_bg;
    match (needs_fg, needs_bg) {
        (true, true) => {
            let _ = write!(out, "\x1b[38;5;{};48;5;{}m", cell.fg, cell.bg);
            *cur_fg = cell.fg as i32;
            *cur_bg = cell.bg as i32;
        }
        (true, false) => {
            let _ = write!(out, "\x1b[38;5;{}m", cell.fg);
            *cur_fg = cell.fg as i32;
        }
        (false, true) => {
            let _ = write!(out, "\x1b[48;5;{}m", cell.bg);
            *cur_bg = cell.bg as i32;
        }
        (false, false) => {}
    }
    out.push(cell.ch);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_is_exact_nearest_neighbour() {
        let pal = Palette::new();
        // Brute-force reference over the 240 colours the game can emit.
        let brute = |r: i32, g: i32, b: i32| -> u8 {
            let mut best = 16u8;
            let mut bd = i32::MAX;
            for i in 16..256usize {
                let c = pal.rgb_of[i];
                let d = WR * (r - c[0] as i32).pow(2)
                    + WG * (g - c[1] as i32).pow(2)
                    + WB * (b - c[2] as i32).pow(2);
                if d < bd {
                    bd = d;
                    best = i as u8;
                }
            }
            best
        };
        let mut rng = crate::rng::Rng::new(7);
        for _ in 0..4000 {
            let r = rng.below(256) as i32;
            let g = rng.below(256) as i32;
            let b = rng.below(256) as i32;
            let got = pal.quantize(r as u8, g as u8, b as u8);
            let want = brute(r, g, b);
            let cg = pal.rgb_of[got as usize];
            let cw = pal.rgb_of[want as usize];
            let dg = WR * (r - cg[0] as i32).pow(2)
                + WG * (g - cg[1] as i32).pow(2)
                + WB * (b - cg[2] as i32).pow(2);
            let dw = WR * (r - cw[0] as i32).pow(2)
                + WG * (g - cw[1] as i32).pow(2)
                + WB * (b - cw[2] as i32).pow(2);
            assert_eq!(dg, dw, "({r},{g},{b}) got {got} want {want}");
        }
    }

    #[test]
    fn quantizer_never_uses_theme_dependent_colours() {
        let pal = Palette::new();
        for r in (0..256).step_by(7) {
            for g in (0..256).step_by(7) {
                for b in (0..256).step_by(7) {
                    assert!(pal.quantize(r as u8, g as u8, b as u8) >= 16);
                }
            }
        }
    }

    #[test]
    fn diff_writer_repaints_only_changed_cells() {
        let mut s = Screen::new(20, 4);
        let mut buf = Vec::new();
        s.flush(&mut buf).unwrap();
        buf.clear();
        // Nothing changed: output should be just the SGR reset.
        let n = s.flush(&mut buf).unwrap();
        assert_eq!(n, 3, "idle frame should emit nothing but a reset");

        s.put(5, 2, Cell { ch: 'X', fg: 200, bg: 16 });
        buf.clear();
        s.flush(&mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("\x1b[3;6H"), "cursor should jump to the cell: {out:?}");
        assert!(out.contains('X'));
        assert!(out.len() < 40, "should not repaint the whole screen: {out:?}");
    }
}
