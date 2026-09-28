//! A linear-light RGB pixel buffer with additive blending, sub-pixel splats
//! and a small separable bloom pass.
//!
//! The game draws into this at twice the terminal's row count; `term.rs` then
//! folds pairs of rows into half-block characters.

pub type Rgb = [f32; 3];

#[inline]
pub fn scale(c: Rgb, k: f32) -> Rgb {
    [c[0] * k, c[1] * k, c[2] * k]
}

#[inline]
pub fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<Rgb>,
    // Scratch buffers for the bloom pass, kept around to avoid per-frame allocation.
    bright: Vec<Rgb>,
    blur: Vec<Rgb>,
    bw: usize,
    bh: usize,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        let bw = w.div_ceil(2);
        let bh = h.div_ceil(2);
        Canvas {
            w,
            h,
            buf: vec![[0.0; 3]; w * h],
            bright: vec![[0.0; 3]; bw * bh],
            blur: vec![[0.0; 3]; bw * bh],
            bw,
            bh,
        }
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.bw = w.div_ceil(2);
        self.bh = h.div_ceil(2);
        self.buf.clear();
        self.buf.resize(w * h, [0.0; 3]);
        self.bright.clear();
        self.bright.resize(self.bw * self.bh, [0.0; 3]);
        self.blur.clear();
        self.blur.resize(self.bw * self.bh, [0.0; 3]);
    }

    pub fn clear(&mut self) {
        for p in self.buf.iter_mut() {
            *p = [0.0, 0.0, 0.0];
        }
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    /// Additive light accumulation at an integer pixel.
    #[inline]
    pub fn add(&mut self, x: i32, y: i32, c: Rgb) {
        if self.in_bounds(x, y) {
            let p = &mut self.buf[y as usize * self.w + x as usize];
            p[0] += c[0];
            p[1] += c[1];
            p[2] += c[2];
        }
    }

    /// Alpha-over composite at an integer pixel.
    #[inline]
    pub fn over(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        if self.in_bounds(x, y) {
            let p = &mut self.buf[y as usize * self.w + x as usize];
            p[0] += (c[0] - p[0]) * a;
            p[1] += (c[1] - p[1]) * a;
            p[2] += (c[2] - p[2]) * a;
        }
    }

    /// Additive splat with bilinear weights, so motion is smooth below the
    /// pixel grid. This is what keeps stars and bullets from looking steppy.
    pub fn add_sub(&mut self, x: f32, y: f32, c: Rgb) {
        let fx = x.floor();
        let fy = y.floor();
        let tx = x - fx;
        let ty = y - fy;
        let ix = fx as i32;
        let iy = fy as i32;
        self.add(ix, iy, scale(c, (1.0 - tx) * (1.0 - ty)));
        self.add(ix + 1, iy, scale(c, tx * (1.0 - ty)));
        self.add(ix, iy + 1, scale(c, (1.0 - tx) * ty));
        self.add(ix + 1, iy + 1, scale(c, tx * ty));
    }

    /// Soft additive glow: intensity falls off smoothly to zero at `r`.
    pub fn glow(&mut self, cx: f32, cy: f32, r: f32, c: Rgb) {
        if r <= 0.0 {
            return;
        }
        let x0 = (cx - r).floor().max(0.0) as i32;
        let x1 = (cx + r).ceil().min(self.w as f32 - 1.0) as i32;
        let y0 = (cy - r).floor().max(0.0) as i32;
        let y1 = (cy + r).ceil().min(self.h as f32 - 1.0) as i32;
        let inv = 1.0 / (r * r);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d2 = (dx * dx + dy * dy) * inv;
                if d2 < 1.0 {
                    // smooth (1 - d^2)^2 falloff
                    let f = 1.0 - d2;
                    self.add(x, y, scale(c, f * f));
                }
            }
        }
    }

    /// Filled disc with a soft one-pixel edge, composited over.
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, alpha: f32) {
        if r <= 0.0 {
            return;
        }
        let x0 = (cx - r - 1.0).floor().max(0.0) as i32;
        let x1 = (cx + r + 1.0).ceil().min(self.w as f32 - 1.0) as i32;
        let y0 = (cy - r - 1.0).floor().max(0.0) as i32;
        let y1 = (cy + r + 1.0).ceil().min(self.h as f32 - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.over(x, y, c, cov * alpha);
                }
            }
        }
    }

    /// Additive ring, used for shockwaves.
    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, thick: f32, c: Rgb) {
        let outer = r + thick;
        let x0 = (cx - outer).floor().max(0.0) as i32;
        let x1 = (cx + outer).ceil().min(self.w as f32 - 1.0) as i32;
        let y0 = (cy - outer).floor().max(0.0) as i32;
        let y1 = (cy + outer).ceil().min(self.h as f32 - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let t = ((d - r).abs() / thick).min(1.0);
                let f = 1.0 - t;
                if f > 0.0 {
                    self.add(x, y, scale(c, f * f));
                }
            }
        }
    }

    /// Additive anti-aliased segment (used for laser beams and debris streaks).
    pub fn streak(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let n = (dx.abs().max(dy.abs()).ceil() as i32).clamp(1, 512);
        let inv = 1.0 / n as f32;
        let step = scale(c, inv.sqrt().max(inv));
        for i in 0..=n {
            let t = i as f32 * inv;
            self.add_sub(x0 + dx * t, y0 + dy * t, step);
        }
    }

    // ---- bloom -------------------------------------------------------

    /// Extract highlights at half resolution, blur them, and add back.
    /// `threshold` is in linear light; `strength` scales the result.
    pub fn bloom(&mut self, threshold: f32, strength: f32) {
        let (w, h, bw, bh) = (self.w, self.h, self.bw, self.bh);

        // Downsample 2x2 boxes, keeping only the energy above the threshold.
        for by in 0..bh {
            for bx in 0..bw {
                let mut acc = [0.0f32; 3];
                let mut n = 0.0f32;
                for oy in 0..2 {
                    let y = by * 2 + oy;
                    if y >= h {
                        continue;
                    }
                    for ox in 0..2 {
                        let x = bx * 2 + ox;
                        if x >= w {
                            continue;
                        }
                        let p = self.buf[y * w + x];
                        acc[0] += p[0];
                        acc[1] += p[1];
                        acc[2] += p[2];
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    let inv = 1.0 / n;
                    let mut p = [acc[0] * inv, acc[1] * inv, acc[2] * inv];
                    let lum = p[0] * 0.299 + p[1] * 0.587 + p[2] * 0.114;
                    let k = if lum > threshold {
                        (lum - threshold) / lum.max(1e-4)
                    } else {
                        0.0
                    };
                    p = [p[0] * k, p[1] * k, p[2] * k];
                    self.bright[by * bw + bx] = p;
                }
            }
        }

        // Two separable box passes approximate a gaussian well enough here.
        for _ in 0..2 {
            // horizontal
            for y in 0..bh {
                let row = y * bw;
                for x in 0..bw {
                    let mut acc = [0.0f32; 3];
                    let mut n = 0.0f32;
                    for d in -2i32..=2 {
                        let sx = x as i32 + d;
                        if sx < 0 || sx >= bw as i32 {
                            continue;
                        }
                        let p = self.bright[row + sx as usize];
                        let wgt = KERNEL5[(d + 2) as usize];
                        acc[0] += p[0] * wgt;
                        acc[1] += p[1] * wgt;
                        acc[2] += p[2] * wgt;
                        n += wgt;
                    }
                    let inv = 1.0 / n.max(1e-4);
                    self.blur[row + x] = [acc[0] * inv, acc[1] * inv, acc[2] * inv];
                }
            }
            // vertical
            for y in 0..bh {
                for x in 0..bw {
                    let mut acc = [0.0f32; 3];
                    let mut n = 0.0f32;
                    for d in -2i32..=2 {
                        let sy = y as i32 + d;
                        if sy < 0 || sy >= bh as i32 {
                            continue;
                        }
                        let p = self.blur[sy as usize * bw + x];
                        let wgt = KERNEL5[(d + 2) as usize];
                        acc[0] += p[0] * wgt;
                        acc[1] += p[1] * wgt;
                        acc[2] += p[2] * wgt;
                        n += wgt;
                    }
                    let inv = 1.0 / n.max(1e-4);
                    self.bright[y * bw + x] = [acc[0] * inv, acc[1] * inv, acc[2] * inv];
                }
            }
        }

        // Add back with bilinear upsampling.
        for y in 0..h {
            let gy = (y as f32 - 0.5) * 0.5;
            let y0 = gy.floor();
            let ty = gy - y0;
            let iy0 = (y0 as i32).clamp(0, bh as i32 - 1) as usize;
            let iy1 = (y0 as i32 + 1).clamp(0, bh as i32 - 1) as usize;
            for x in 0..w {
                let gx = (x as f32 - 0.5) * 0.5;
                let x0 = gx.floor();
                let tx = gx - x0;
                let ix0 = (x0 as i32).clamp(0, bw as i32 - 1) as usize;
                let ix1 = (x0 as i32 + 1).clamp(0, bw as i32 - 1) as usize;
                let a = self.bright[iy0 * bw + ix0];
                let b = self.bright[iy0 * bw + ix1];
                let c = self.bright[iy1 * bw + ix0];
                let d = self.bright[iy1 * bw + ix1];
                let top = lerp_rgb(a, b, tx);
                let bot = lerp_rgb(c, d, tx);
                let v = lerp_rgb(top, bot, ty);
                let p = &mut self.buf[y * w + x];
                p[0] += v[0] * strength;
                p[1] += v[1] * strength;
                p[2] += v[2] * strength;
            }
        }
    }

    /// Multiply the whole frame (screen flashes, fade-in/out).
    pub fn tint(&mut self, k: f32) {
        for p in self.buf.iter_mut() {
            p[0] *= k;
            p[1] *= k;
            p[2] *= k;
        }
    }

    /// Add a uniform colour to the whole frame.
    pub fn wash(&mut self, c: Rgb) {
        for p in self.buf.iter_mut() {
            p[0] += c[0];
            p[1] += c[1];
            p[2] += c[2];
        }
    }
}

const KERNEL5: [f32; 5] = [1.0, 4.0, 7.0, 4.0, 1.0];
