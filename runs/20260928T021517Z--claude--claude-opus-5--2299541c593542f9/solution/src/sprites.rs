//! Pixel art. Small craft are hand-authored as character grids; the capital
//! ship is generated from overlapping shapes so it can be large without
//! thousands of hand-placed pixels. Both bake down to the same `Sprite` form.

use crate::canvas::{scale, Canvas, Rgb};

/// One baked pixel: base colour, coverage, and how much light it emits.
#[derive(Clone, Copy)]
pub struct Px {
    pub c: Rgb,
    pub a: f32,
    pub em: f32,
}

pub struct Sprite {
    pub w: i32,
    pub h: i32,
    /// Anchor offset from the top-left, in pixels.
    pub ax: i32,
    pub ay: i32,
    pub px: Vec<Px>,
}

impl Sprite {
    #[inline]
    fn at(&self, x: i32, y: i32) -> Px {
        self.px[(y * self.w + x) as usize]
    }
}

/// Shading ramp shared by every character-grid sprite.
#[derive(Clone, Copy)]
pub struct Pal {
    /// outline
    pub k: Rgb,
    /// dark hull
    pub d: Rgb,
    /// mid hull
    pub m: Rgb,
    /// highlight
    pub l: Rgb,
    /// accent trim
    pub a: Rgb,
    /// emissive core
    pub c: Rgb,
    /// engine port (emissive, usually overdrawn by the flame)
    pub e: Rgb,
}

const TRANSPARENT: Px = Px {
    c: [0.0, 0.0, 0.0],
    a: 0.0,
    em: 0.0,
};

fn bake(rows: &[&str], pal: &Pal) -> Sprite {
    let h = rows.len() as i32;
    let w = rows[0].chars().count() as i32;
    for (i, r) in rows.iter().enumerate() {
        debug_assert_eq!(
            r.chars().count() as i32,
            w,
            "sprite row {i} has the wrong width"
        );
    }
    let mut px = Vec::with_capacity((w * h) as usize);
    for r in rows {
        for ch in r.chars() {
            px.push(match ch {
                'k' => Px { c: pal.k, a: 1.0, em: 0.0 },
                'd' => Px { c: pal.d, a: 1.0, em: 0.0 },
                'm' => Px { c: pal.m, a: 1.0, em: 0.0 },
                'l' => Px { c: pal.l, a: 1.0, em: 0.0 },
                'a' => Px { c: pal.a, a: 1.0, em: 0.0 },
                'c' => Px { c: pal.c, a: 1.0, em: 0.65 },
                'e' => Px { c: pal.e, a: 1.0, em: 1.10 },
                _ => TRANSPARENT,
            });
        }
    }
    Sprite {
        w,
        h,
        ax: w / 2,
        ay: h / 2,
        px,
    }
}

// ---------------------------------------------------------------------------
// Character art
// ---------------------------------------------------------------------------

#[rustfmt::skip]
const PLAYER_ART: &[&str] = &[
    ".....c.....",
    ".....l.....",
    "....lml....",
    "....lcl....",
    "..adlclda..",
    ".aadmcmdaa.",
    "aaadmmmdaaa",
    "aa.dmkmd.aa",
    "....e.e....",
];

#[rustfmt::skip]
const DRIFTER_ART: &[&str] = &[
    "..ddddd..",
    ".dmlclmd.",
    "dmmmcmmmd",
    "dmmmcmmmd",
    "admm.mmda",
    ".a.d.d.a.",
];

#[rustfmt::skip]
const DART_ART: &[&str] = &[
    "a.ddd.a",
    "admmmda",
    "admcmda",
    ".dmcmd.",
    ".dmcmd.",
    "..dcd..",
    "...k...",
];

#[rustfmt::skip]
const SENTINEL_ART: &[&str] = &[
    "...ddddddd...",
    "..dmmmmmmmd..",
    ".dmmmlllmmmd.",
    "dmmmlccclmmmd",
    "dmmmlccclmmmd",
    ".dmmmlllmmmd.",
    "a.dmmmmmmmd.a",
    "aa.d.....d.aa",
];

#[rustfmt::skip]
const WEAVER_ART: &[&str] = &[
    ".....ddd.....",
    "...ddmmmdd...",
    ".ddmmlclmmdd.",
    "ddmmmmcmmmmdd",
    "dmmmdd.ddmmmd",
    "a.d.......d.a",
    "...a.....a...",
];

#[rustfmt::skip]
const MINE_ART: &[&str] = &[
    "..aaa..",
    ".dmmmd.",
    "admcmda",
    "amcccma",
    "admcmda",
    ".dmmmd.",
    "..aaa..",
];

#[rustfmt::skip]
const WARDEN_ART: &[&str] = &[
    ".....ddddddddd.....",
    "...ddmmmmmmmmmdd...",
    "..dmmmmlllllmmmmd..",
    ".dmmmlccccccclmmmd.",
    "dmmmmlccccccclmmmmd",
    "dmmmmlccccccclmmmmd",
    ".dmmmlccccccclmmmd.",
    "..dmmmmlllllmmmmd..",
    "a..ddmmmmmmmmmdd..a",
    "aa...ddmmmmmdd...aa",
    ".....e.......e.....",
];

pub const PLAYER_PAL: Pal = Pal {
    k: [0.04, 0.06, 0.10],
    d: [0.13, 0.21, 0.34],
    m: [0.36, 0.54, 0.74],
    l: [0.76, 0.90, 1.00],
    a: [0.14, 0.50, 0.70],
    c: [0.55, 0.95, 1.00],
    e: [0.60, 0.85, 1.00],
};

pub const DRIFTER_PAL: Pal = Pal {
    k: [0.06, 0.03, 0.02],
    d: [0.30, 0.13, 0.06],
    m: [0.64, 0.31, 0.12],
    l: [0.97, 0.66, 0.30],
    a: [0.46, 0.17, 0.08],
    c: [1.00, 0.56, 0.16],
    e: [1.00, 0.70, 0.30],
};

pub const DART_PAL: Pal = Pal {
    k: [0.05, 0.02, 0.07],
    d: [0.27, 0.09, 0.31],
    m: [0.59, 0.21, 0.63],
    l: [0.94, 0.63, 0.97],
    a: [0.40, 0.10, 0.46],
    c: [1.00, 0.36, 0.86],
    e: [1.00, 0.50, 0.90],
};

pub const SENTINEL_PAL: Pal = Pal {
    k: [0.03, 0.06, 0.05],
    d: [0.12, 0.27, 0.21],
    m: [0.31, 0.57, 0.43],
    l: [0.73, 0.94, 0.82],
    a: [0.18, 0.39, 0.29],
    c: [0.48, 1.00, 0.62],
    e: [0.60, 1.00, 0.70],
};

pub const WEAVER_PAL: Pal = Pal {
    k: [0.07, 0.06, 0.02],
    d: [0.31, 0.27, 0.08],
    m: [0.67, 0.59, 0.18],
    l: [1.00, 0.95, 0.56],
    a: [0.46, 0.39, 0.10],
    c: [1.00, 0.90, 0.32],
    e: [1.00, 0.95, 0.50],
};

pub const MINE_PAL: Pal = Pal {
    k: [0.07, 0.02, 0.02],
    d: [0.33, 0.07, 0.06],
    m: [0.70, 0.17, 0.14],
    l: [1.00, 0.56, 0.46],
    a: [0.52, 0.11, 0.08],
    c: [1.00, 0.26, 0.16],
    e: [1.00, 0.40, 0.25],
};

pub const WARDEN_PAL: Pal = Pal {
    k: [0.05, 0.04, 0.08],
    d: [0.20, 0.17, 0.30],
    m: [0.44, 0.38, 0.60],
    l: [0.85, 0.80, 1.00],
    a: [0.30, 0.22, 0.45],
    c: [0.80, 0.45, 1.00],
    e: [0.90, 0.60, 1.00],
};

// ---------------------------------------------------------------------------
// Generated capital ship
// ---------------------------------------------------------------------------

/// Signed coverage of a rounded trapezoid, used as a building block.
fn trapezoid(x: f32, y: f32, cy: f32, half_h: f32, top_w: f32, bot_w: f32) -> f32 {
    let t = ((y - (cy - half_h)) / (2.0 * half_h)).clamp(0.0, 1.0);
    let w = top_w + (bot_w - top_w) * t;
    if y < cy - half_h - 0.5 || y > cy + half_h + 0.5 {
        return 0.0;
    }
    let edge = w - x.abs();
    edge.clamp(0.0, 1.0)
}

fn ellipse(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let dx = (x - cx) / rx;
    let dy = (y - cy) / ry;
    let d = (dx * dx + dy * dy).sqrt();
    // Convert the normalised distance back to pixels for a 1px soft edge.
    let px = (1.0 - d) * rx.min(ry);
    px.clamp(0.0, 1.0)
}

/// Build the wave boss: a broad carrier hull with two wing pods and a core.
fn build_dreadnought(pal: &Pal) -> Sprite {
    let w: i32 = 45;
    let h: i32 = 23;
    let cx = (w / 2) as f32;
    let mut px = vec![TRANSPARENT; (w * h) as usize];

    // Coverage field first, so shading can be derived from it.
    let mut cov = vec![0.0f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 + 0.5 - cx;
            let fy = y as f32 + 0.5;
            let mut c: f32 = 0.0;
            // Central spine, widening towards the stern.
            c = c.max(trapezoid(fx, fy, 11.0, 11.0, 3.5, 7.0));
            // Main hull body.
            c = c.max(ellipse(fx, fy, 0.0, 11.0, 12.5, 9.5));
            // Forward prow.
            c = c.max(trapezoid(fx, fy, 6.0, 6.0, 2.0, 9.0));
            // Wing pods.
            c = c.max(ellipse(fx, fy, 16.0, 12.0, 6.5, 4.5));
            c = c.max(ellipse(fx, fy, -16.0, 12.0, 6.5, 4.5));
            // Outer pylons tying the pods to the hull.
            if fy > 9.0 && fy < 14.0 {
                c = c.max(((22.0 - fx.abs()) * 0.5).clamp(0.0, 1.0).min(1.0));
            }
            cov[(y * w + x) as usize] = c;
        }
    }

    // Shade: rim highlight near the silhouette edge, dark towards the stern,
    // and a bright emissive core amidships.
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let c = cov[i];
            if c <= 0.0 {
                continue;
            }
            let fx = x as f32 + 0.5 - cx;
            let fy = y as f32 + 0.5;

            // Distance to the nearest empty pixel, capped at 3.
            let mut edge = 3.0f32;
            'probe: for r in 1..=3 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy > r * r {
                            continue;
                        }
                        let nx = x + dx;
                        let ny = y + dy;
                        let outside = nx < 0
                            || ny < 0
                            || nx >= w
                            || ny >= h
                            || cov[(ny * w + nx) as usize] < 0.35;
                        if outside {
                            edge = r as f32;
                            break 'probe;
                        }
                    }
                }
            }

            // Vertical light falloff: lit from the prow.
            let lit = 1.0 - (fy / h as f32) * 0.55;
            let base = if edge <= 1.0 {
                pal.d
            } else if edge <= 2.0 {
                crate::canvas::lerp_rgb(pal.d, pal.m, 0.6)
            } else {
                pal.m
            };
            let mut col = scale(base, lit);

            // Hull plating: faint darker seams along the length.
            if (fx.abs() as i32) % 7 == 3 && edge > 1.0 {
                col = scale(col, 0.82);
            }

            // Trim stripes down the wings.
            if fy > 10.5 && fy < 12.5 && fx.abs() > 9.0 {
                col = pal.a;
            }

            // Emissive core.
            let core = 1.0 - ((fx / 5.0).powi(2) + ((fy - 10.0) / 4.0).powi(2)).sqrt();
            let mut em = 0.0;
            if core > 0.0 {
                let t = core.clamp(0.0, 1.0);
                col = crate::canvas::lerp_rgb(col, pal.c, t.powf(0.6));
                em = t * 0.9;
            }
            // Pod lights.
            for side in [-1.0f32, 1.0] {
                let d = (((fx - side * 16.0) / 2.6).powi(2) + ((fy - 12.0) / 2.0).powi(2)).sqrt();
                if d < 1.0 {
                    let t = 1.0 - d;
                    col = crate::canvas::lerp_rgb(col, pal.e, t);
                    em = em.max(t * 1.0);
                }
            }

            px[i] = Px { c: col, a: c, em };
        }
    }

    Sprite {
        w,
        h,
        ax: w / 2,
        ay: h / 2,
        px,
    }
}

// ---------------------------------------------------------------------------

pub struct SpriteSet {
    pub player: Sprite,
    pub drifter: Sprite,
    pub dart: Sprite,
    pub sentinel: Sprite,
    pub weaver: Sprite,
    pub mine: Sprite,
    pub warden: Sprite,
    pub dreadnought: Sprite,
}

impl SpriteSet {
    pub fn new() -> Self {
        SpriteSet {
            player: bake(PLAYER_ART, &PLAYER_PAL),
            drifter: bake(DRIFTER_ART, &DRIFTER_PAL),
            dart: bake(DART_ART, &DART_PAL),
            sentinel: bake(SENTINEL_ART, &SENTINEL_PAL),
            weaver: bake(WEAVER_ART, &WEAVER_PAL),
            mine: bake(MINE_ART, &MINE_PAL),
            warden: bake(WARDEN_ART, &WARDEN_PAL),
            dreadnought: build_dreadnought(&WARDEN_PAL),
        }
    }
}

/// How a sprite should be drawn this frame.
#[derive(Clone, Copy)]
pub struct DrawOpts {
    /// Overall opacity.
    pub alpha: f32,
    /// 0 = normal, 1 = fully blown out to white (damage flash).
    pub flash: f32,
    /// Vertical shear per horizontal pixel, for banking.
    pub shear: f32,
    /// Extra multiplier on emissive pixels.
    pub emissive: f32,
}

impl Default for DrawOpts {
    fn default() -> Self {
        DrawOpts {
            alpha: 1.0,
            flash: 0.0,
            shear: 0.0,
            emissive: 1.0,
        }
    }
}

/// Blit a baked sprite with its anchor at (x, y), rounded to the pixel grid so
/// the art stays crisp.
pub fn draw(canvas: &mut Canvas, s: &Sprite, x: f32, y: f32, o: DrawOpts) {
    if o.alpha <= 0.003 {
        return;
    }
    let ox = x.round() as i32 - s.ax;
    let oy = y.round() as i32 - s.ay;
    let white: Rgb = [1.0, 1.0, 1.0];
    for sy in 0..s.h {
        for sx in 0..s.w {
            let p = s.at(sx, sy);
            if p.a <= 0.0 {
                continue;
            }
            let shear = if o.shear != 0.0 {
                (o.shear * (sx - s.ax) as f32).round() as i32
            } else {
                0
            };
            let dx = ox + sx;
            let dy = oy + sy + shear;
            let col = if o.flash > 0.0 {
                crate::canvas::lerp_rgb(p.c, white, o.flash)
            } else {
                p.c
            };
            canvas.over(dx, dy, col, p.a * o.alpha);
            if p.em > 0.0 {
                canvas.add(dx, dy, scale(col, p.em * o.emissive * o.alpha));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3x5 pixel font, used for the letters stamped on pickups.
// ---------------------------------------------------------------------------

/// Rows are 3 bits wide, most significant bit on the left, 5 rows per glyph.
fn glyph(ch: char) -> Option<[u8; 5]> {
    Some(match ch {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        'W' => [0b101, 0b101, 0b101, 0b111, 0b101],
        'S' => [0b111, 0b100, 0b111, 0b001, 0b111],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        '!' => [0b010, 0b010, 0b010, 0b000, 0b010],
        ' ' => [0, 0, 0, 0, 0],
        _ => return None,
    })
}

/// Width in pixels of a string rendered with `stamp_text`.
pub fn text_width(s: &str) -> i32 {
    let n = s.chars().count() as i32;
    (n * 4 - 1).max(0)
}

/// Stamp a 3x5 glyph with its centre at (x, y).
///
/// Strokes here are one pixel wide with one-pixel gaps, so this deliberately
/// adds no emissive light: a bloom halo would close those gaps and turn the
/// text into a smear.
pub fn stamp_letter(canvas: &mut Canvas, ch: char, x: f32, y: f32, c: Rgb, a: f32) {
    let Some(g) = glyph(ch) else { return };
    let ox = x.round() as i32 - 1;
    let oy = y.round() as i32 - 2;
    for (ry, bits) in g.iter().enumerate() {
        for rx in 0..3 {
            if bits & (0b100 >> rx) != 0 {
                canvas.over(ox + rx as i32, oy + ry as i32, c, a);
            }
        }
    }
}

/// Stamp a short string in the 3x5 font, centred horizontally on `x`.
pub fn stamp_text(canvas: &mut Canvas, s: &str, x: f32, y: f32, c: Rgb, a: f32) {
    if a <= 0.01 {
        return;
    }
    let w = text_width(s) as f32;
    let mut cx = x - w * 0.5 + 1.0;
    for ch in s.chars() {
        stamp_letter(canvas, ch, cx, y, c, a);
        cx += 4.0;
    }
}

// ---------------------------------------------------------------------------
// 5x7 display font, stamped into the canvas for titles and banners.
// ---------------------------------------------------------------------------

/// Each glyph is seven rows of five bits, most significant bit leftmost.
#[rustfmt::skip]
fn glyph57(ch: char) -> Option<[u8; 7]> {
    Some(match ch.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10101, 0b10011, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        '\'' => [0b00100, 0b00100, 0, 0, 0, 0, 0],
        _ => return None,
    })
}

/// Pixel width of `s` drawn by `stamp_big` at the given scale.
pub fn big_width(s: &str, sc: i32) -> i32 {
    let n = s.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * 6 * sc - sc
    }
}

/// Stamp a string in the 5x7 font, centred on `x`, top at `y`.
///
/// `glow` adds emissive light so the bloom pass gives large text a halo. It is
/// only ever useful on the pre-bloom pass, and only at scale 2 and above: a
/// one-pixel stroke would be smeared shut by its own halo.
pub fn stamp_big(
    canvas: &mut Canvas,
    s: &str,
    x: f32,
    y: f32,
    sc: i32,
    c: Rgb,
    a: f32,
    glow: bool,
) {
    if a <= 0.01 || sc < 1 {
        return;
    }
    let total = big_width(s, sc);
    let mut ox = (x - total as f32 * 0.5).round() as i32;
    let oy = y.round() as i32;
    for ch in s.chars() {
        if let Some(g) = glyph57(ch) {
            for (ry, bits) in g.iter().enumerate() {
                for rx in 0..5 {
                    if bits & (0b10000 >> rx) == 0 {
                        continue;
                    }
                    for sy in 0..sc {
                        for sx in 0..sc {
                            let dx = ox + rx * sc + sx;
                            let dy = oy + ry as i32 * sc + sy;
                            canvas.over(dx, dy, c, a);
                            if glow && sc > 1 {
                                // Enough extra light for the bloom pass to
                                // give the text a halo, but not so much that
                                // every colour blows out to white.
                                canvas.add(dx, dy, scale(c, 0.35 * a));
                            }
                        }
                    }
                }
            }
        }
        ox += 6 * sc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_font_covers_the_strings_the_game_draws() {
        let used = "NEBULA DRIFT WAVE CLEAR GAME OVER PAUSED 0123456789 \
                    READY RUN ENDED NEW BEST!-.'";
        for ch in used.chars() {
            assert!(glyph57(ch).is_some(), "missing 5x7 glyph for {ch:?}");
        }
    }

    #[test]
    fn micro_font_covers_pickup_letters_and_digits() {
        for ch in "WSHBPX0123456789+- !".chars() {
            assert!(glyph(ch).is_some(), "missing 3x5 glyph for {ch:?}");
        }
    }

    #[test]
    fn all_character_art_is_rectangular() {
        for art in [
            PLAYER_ART,
            DRIFTER_ART,
            DART_ART,
            SENTINEL_ART,
            WEAVER_ART,
            MINE_ART,
            WARDEN_ART,
        ] {
            let w = art[0].chars().count();
            for (i, row) in art.iter().enumerate() {
                assert_eq!(row.chars().count(), w, "row {i} of {art:?}");
            }
        }
    }

    #[test]
    fn sprites_bake_with_visible_pixels() {
        let set = SpriteSet::new();
        for (name, s) in [
            ("player", &set.player),
            ("drifter", &set.drifter),
            ("dart", &set.dart),
            ("sentinel", &set.sentinel),
            ("weaver", &set.weaver),
            ("mine", &set.mine),
            ("warden", &set.warden),
            ("dreadnought", &set.dreadnought),
        ] {
            let solid = s.px.iter().filter(|p| p.a > 0.5).count();
            assert!(solid > 10, "{name} baked almost empty ({solid} px)");
            assert_eq!(s.px.len(), (s.w * s.h) as usize, "{name} size mismatch");
        }
    }
}
