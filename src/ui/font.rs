//! Минимальный движок TrueType: разбор `cmap`/`glyf`/`hmtx` и растеризация
//! глифов с аналитическим сглаживанием при любом размере (поддержка HiDPI).
//!
//! Встроенный шрифт — подмножество DejaVu Sans (латиница, кириллица, символы).

use crate::renderer::canvas::{Coverage, Framebuffer};
use std::cell::RefCell;
use std::collections::HashMap;

/// Встроенный шрифт интерфейса.
pub static UI_FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/ui.ttf");

#[derive(Clone, Copy, Debug)]
enum Seg {
    Line(f32, f32, f32, f32),
    Quad(f32, f32, f32, f32, f32, f32),
}

/// Растеризованный глиф.
#[derive(Clone, Debug, Default)]
pub struct GlyphBitmap {
    pub w: usize,
    pub h: usize,
    /// Смещение левого верхнего угла относительно точки пера (x) и базовой линии (y).
    pub xoff: i32,
    pub yoff: i32,
    pub advance: f32,
    pub alpha: Vec<u8>,
}

/// Шрифт TrueType с кэшем глифов.
pub struct Font {
    data: &'static [u8],
    units_per_em: f32,
    ascent: f32,
    descent: f32,
    line_gap: f32,
    num_hmetrics: u16,
    loca_long: bool,
    off_glyf: usize,
    off_loca: usize,
    off_hmtx: usize,
    cmap: HashMap<u32, u16>,
    cache: RefCell<HashMap<(char, u32), GlyphBitmap>>,
}

fn rd_u16(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0)
}
fn rd_i16(d: &[u8], o: usize) -> i16 {
    rd_u16(d, o) as i16
}
fn rd_u32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}

impl Font {
    /// Загрузка шрифта из байтов TTF.
    pub fn load(data: &'static [u8]) -> Result<Font, String> {
        if data.len() < 12 {
            return Err("слишком короткий файл шрифта".into());
        }
        let num_tables = rd_u16(data, 4) as usize;
        let mut tables: HashMap<[u8; 4], usize> = HashMap::new();
        for i in 0..num_tables {
            let rec = 12 + i * 16;
            let mut tag = [0u8; 4];
            tag.copy_from_slice(&data[rec..rec + 4]);
            tables.insert(tag, rd_u32(data, rec + 8) as usize);
        }
        let t = |name: &[u8; 4]| tables.get(name).copied().ok_or_else(|| format!("нет таблицы {}", String::from_utf8_lossy(name)));
        let head = t(b"head")?;
        let hhea = t(b"hhea")?;
        let cmap_off = t(b"cmap")?;
        let units_per_em = rd_u16(data, head + 18) as f32;
        let loca_long = rd_i16(data, head + 50) != 0;
        let ascent = rd_i16(data, hhea + 4) as f32;
        let descent = rd_i16(data, hhea + 6) as f32;
        let line_gap = rd_i16(data, hhea + 8) as f32;
        let num_hmetrics = rd_u16(data, hhea + 34);

        // cmap: ищем подтаблицу формата 4 (Unicode BMP).
        let mut cmap = HashMap::new();
        let n_sub = rd_u16(data, cmap_off + 2) as usize;
        for i in 0..n_sub {
            let rec = cmap_off + 4 + i * 8;
            let sub = cmap_off + rd_u32(data, rec + 4) as usize;
            if rd_u16(data, sub) != 4 {
                continue;
            }
            let seg_x2 = rd_u16(data, sub + 6) as usize;
            let ends = sub + 14;
            let starts = ends + seg_x2 + 2;
            let deltas = starts + seg_x2;
            let ranges = deltas + seg_x2;
            for s in 0..seg_x2 / 2 {
                let end = rd_u16(data, ends + s * 2) as u32;
                let start = rd_u16(data, starts + s * 2) as u32;
                let delta = rd_u16(data, deltas + s * 2);
                let ro = rd_u16(data, ranges + s * 2) as usize;
                if start == 0xFFFF {
                    continue;
                }
                for c in start..=end {
                    let g = if ro == 0 {
                        (c as u16).wrapping_add(delta)
                    } else {
                        let addr = ranges + s * 2 + ro + (c - start) as usize * 2;
                        let g = rd_u16(data, addr);
                        if g == 0 { 0 } else { g.wrapping_add(delta) }
                    };
                    if g != 0 {
                        cmap.insert(c, g);
                    }
                }
            }
            break;
        }
        Ok(Font {
            data,
            units_per_em,
            ascent,
            descent,
            line_gap,
            num_hmetrics,
            loca_long,
            off_glyf: t(b"glyf")?,
            off_loca: t(b"loca")?,
            off_hmtx: t(b"hmtx")?,
            cmap,
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// Встроенный шрифт интерфейса.
    pub fn builtin() -> Font {
        Font::load(UI_FONT_DATA).expect("встроенный шрифт повреждён")
    }

    fn glyph_index(&self, ch: char) -> u16 {
        self.cmap.get(&(ch as u32)).copied().unwrap_or_else(|| self.cmap.get(&('?' as u32)).copied().unwrap_or(0))
    }

    fn advance_units(&self, g: u16) -> f32 {
        let idx = (g.min(self.num_hmetrics.saturating_sub(1))) as usize;
        rd_u16(self.data, self.off_hmtx + idx * 4) as f32
    }

    fn glyph_range(&self, g: u16) -> Option<(usize, usize)> {
        let g = g as usize;
        let (a, b) = if self.loca_long {
            (rd_u32(self.data, self.off_loca + g * 4) as usize, rd_u32(self.data, self.off_loca + g * 4 + 4) as usize)
        } else {
            (rd_u16(self.data, self.off_loca + g * 2) as usize * 2, rd_u16(self.data, self.off_loca + g * 2 + 2) as usize * 2)
        };
        if b <= a {
            None
        } else {
            Some((self.off_glyf + a, self.off_glyf + b))
        }
    }

    /// Контуры глифа в единицах шрифта (y вверх).
    fn outline(&self, g: u16, depth: u32, out: &mut Vec<Seg>, xf: [f32; 6]) {
        if depth > 6 {
            return;
        }
        let Some((start, _end)) = self.glyph_range(g) else { return };
        let d = self.data;
        let ncont = rd_i16(d, start);
        let apply = |x: f32, y: f32| -> (f32, f32) { (xf[0] * x + xf[2] * y + xf[4], xf[1] * x + xf[3] * y + xf[5]) };
        if ncont >= 0 {
            let ncont = ncont as usize;
            let mut p = start + 10;
            let mut end_pts = Vec::with_capacity(ncont);
            for _ in 0..ncont {
                end_pts.push(rd_u16(d, p) as usize);
                p += 2;
            }
            let npts = end_pts.last().map(|e| e + 1).unwrap_or(0);
            let ins_len = rd_u16(d, p) as usize;
            p += 2 + ins_len;
            let mut flags = Vec::with_capacity(npts);
            while flags.len() < npts {
                let f = d.get(p).copied().unwrap_or(0);
                p += 1;
                flags.push(f);
                if f & 8 != 0 {
                    let r = d.get(p).copied().unwrap_or(0);
                    p += 1;
                    for _ in 0..r {
                        flags.push(f);
                    }
                }
            }
            flags.truncate(npts);
            let mut xs = Vec::with_capacity(npts);
            let mut v = 0i32;
            for &f in &flags {
                if f & 2 != 0 {
                    let dx = d.get(p).copied().unwrap_or(0) as i32;
                    p += 1;
                    v += if f & 16 != 0 { dx } else { -dx };
                } else if f & 16 == 0 {
                    v += rd_i16(d, p) as i32;
                    p += 2;
                }
                xs.push(v);
            }
            let mut ys = Vec::with_capacity(npts);
            v = 0;
            for &f in &flags {
                if f & 4 != 0 {
                    let dy = d.get(p).copied().unwrap_or(0) as i32;
                    p += 1;
                    v += if f & 32 != 0 { dy } else { -dy };
                } else if f & 32 == 0 {
                    v += rd_i16(d, p) as i32;
                    p += 2;
                }
                ys.push(v);
            }
            let mut s = 0;
            for &e in &end_pts {
                if e < s || e >= npts {
                    break;
                }
                let pts: Vec<(f32, f32, bool)> =
                    (s..=e).map(|i| (xs[i] as f32, ys[i] as f32, flags[i] & 1 != 0)).collect();
                s = e + 1;
                let n = pts.len();
                if n < 2 {
                    continue;
                }
                // Стартовая on-curve точка.
                let first_on = pts.iter().position(|q| q.2);
                let (sx, sy, offset) = match first_on {
                    Some(i) => (pts[i].0, pts[i].1, i),
                    None => ((pts[0].0 + pts[1].0) * 0.5, (pts[0].1 + pts[1].1) * 0.5, 0),
                };
                let mut cur = (sx, sy);
                let mut ctrl: Option<(f32, f32)> = None;
                let count = if first_on.is_some() { n } else { n + 1 };
                for k in 1..=count {
                    let q = pts[(offset + k) % n];
                    let qp = if first_on.is_none() && k == count { (sx, sy, true) } else { q };
                    if qp.2 {
                        if let Some(c) = ctrl.take() {
                            let (a, b, cc) = (apply(cur.0, cur.1), apply(c.0, c.1), apply(qp.0, qp.1));
                            out.push(Seg::Quad(a.0, a.1, b.0, b.1, cc.0, cc.1));
                        } else {
                            let (a, b) = (apply(cur.0, cur.1), apply(qp.0, qp.1));
                            out.push(Seg::Line(a.0, a.1, b.0, b.1));
                        }
                        cur = (qp.0, qp.1);
                    } else if let Some(c) = ctrl {
                        let mid = ((c.0 + qp.0) * 0.5, (c.1 + qp.1) * 0.5);
                        let (a, b, cc) = (apply(cur.0, cur.1), apply(c.0, c.1), apply(mid.0, mid.1));
                        out.push(Seg::Quad(a.0, a.1, b.0, b.1, cc.0, cc.1));
                        cur = mid;
                        ctrl = Some((qp.0, qp.1));
                    } else {
                        ctrl = Some((qp.0, qp.1));
                    }
                }
                if let Some(c) = ctrl {
                    let (a, b, cc) = (apply(cur.0, cur.1), apply(c.0, c.1), apply(sx, sy));
                    out.push(Seg::Quad(a.0, a.1, b.0, b.1, cc.0, cc.1));
                } else if (cur.0 - sx).abs() > 0.0 || (cur.1 - sy).abs() > 0.0 {
                    let (a, b) = (apply(cur.0, cur.1), apply(sx, sy));
                    out.push(Seg::Line(a.0, a.1, b.0, b.1));
                }
            }
        } else {
            // Составной глиф.
            let mut p = start + 10;
            loop {
                let flags = rd_u16(d, p);
                let gi = rd_u16(d, p + 2);
                p += 4;
                let (dx, dy) = if flags & 1 != 0 {
                    let r = (rd_i16(d, p) as f32, rd_i16(d, p + 2) as f32);
                    p += 4;
                    r
                } else {
                    let r = (d.get(p).copied().unwrap_or(0) as i8 as f32, d.get(p + 1).copied().unwrap_or(0) as i8 as f32);
                    p += 2;
                    r
                };
                let f2 = |o: usize| rd_i16(d, o) as f32 / 16384.0;
                let (mut a, mut b, mut c, mut dd) = (1.0, 0.0, 0.0, 1.0);
                if flags & 8 != 0 {
                    a = f2(p);
                    dd = a;
                    p += 2;
                } else if flags & 0x40 != 0 {
                    a = f2(p);
                    dd = f2(p + 2);
                    p += 4;
                } else if flags & 0x80 != 0 {
                    a = f2(p);
                    b = f2(p + 2);
                    c = f2(p + 4);
                    dd = f2(p + 6);
                    p += 8;
                }
                let (tx, ty) = if flags & 2 != 0 { (dx, dy) } else { (0.0, 0.0) };
                // Композиция: xf ∘ child.
                let child = [a, b, c, dd, tx, ty];
                let m = [
                    xf[0] * child[0] + xf[2] * child[1],
                    xf[1] * child[0] + xf[3] * child[1],
                    xf[0] * child[2] + xf[2] * child[3],
                    xf[1] * child[2] + xf[3] * child[3],
                    xf[0] * child[4] + xf[2] * child[5] + xf[4],
                    xf[1] * child[4] + xf[3] * child[5] + xf[5],
                ];
                self.outline(gi, depth + 1, out, m);
                if flags & 0x20 == 0 {
                    break;
                }
            }
        }
    }

    fn rasterize(&self, ch: char, px: f32) -> GlyphBitmap {
        let g = self.glyph_index(ch);
        let scale = px / self.units_per_em;
        let advance = self.advance_units(g) * scale;
        let mut segs = Vec::new();
        self.outline(g, 0, &mut segs, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        if segs.is_empty() {
            return GlyphBitmap { advance, ..Default::default() };
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut upd = |x: f32, y: f32| {
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
        };
        for s in &segs {
            match *s {
                Seg::Line(a, b, c, d) => {
                    upd(a, b);
                    upd(c, d);
                }
                Seg::Quad(a, b, c, d, e, f) => {
                    upd(a, b);
                    upd(c, d);
                    upd(e, f);
                }
            }
        }
        let x0 = (minx * scale).floor() - 1.0;
        let x1 = (maxx * scale).ceil() + 1.0;
        let y0 = (-maxy * scale).floor() - 1.0; // y вниз
        let y1 = (-miny * scale).ceil() + 1.0;
        let w = (x1 - x0) as usize + 1;
        let h = (y1 - y0) as usize + 1;
        let mut cov = Coverage::new(w, h);
        let tx = |x: f32| x * scale - x0;
        let ty = |y: f32| -y * scale - y0;
        for s in &segs {
            match *s {
                Seg::Line(a, b, c, d) => cov.line(tx(a), ty(b), tx(c), ty(d)),
                Seg::Quad(a, b, c, d, e, f) => cov.quad(tx(a), ty(b), tx(c), ty(d), tx(e), ty(f)),
            }
        }
        GlyphBitmap { w, h, xoff: x0 as i32, yoff: y0 as i32, advance, alpha: cov.finish() }
    }

    fn key(px: f32) -> u32 {
        (px * 4.0).round() as u32
    }

    /// Ширина символа.
    pub fn advance(&self, ch: char, px: f32) -> f32 {
        self.advance_units(self.glyph_index(ch)) * px / self.units_per_em
    }

    /// Ширина строки в пикселях.
    pub fn measure(&self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.advance(c, px)).sum()
    }

    /// Высота подъёма над базовой линией.
    pub fn ascent(&self, px: f32) -> f32 {
        self.ascent * px / self.units_per_em
    }

    /// Межстрочный интервал.
    pub fn line_height(&self, px: f32) -> f32 {
        (self.ascent - self.descent + self.line_gap) * px / self.units_per_em
    }

    /// Рисование строки. `(x, y)` — левый верхний угол строки. Возвращает ширину.
    pub fn draw(&self, fb: &mut Framebuffer, x: f32, y: f32, text: &str, px: f32, color: u32) -> f32 {
        let base = (y + self.ascent(px)).round();
        let mut pen = x;
        let k = Self::key(px);
        let mut cache = self.cache.borrow_mut();
        for ch in text.chars() {
            if ch == ' ' {
                pen += self.advance(' ', px);
                continue;
            }
            let g = cache.entry((ch, k)).or_insert_with(|| self.rasterize(ch, px));
            if g.w > 0 {
                fb.blit_mask(pen.round() as i32 + g.xoff, base as i32 + g.yoff, g.w, g.h, &g.alpha, color);
            }
            pen += g.advance;
        }
        if cache.len() > 4096 {
            cache.clear();
        }
        pen - x
    }

    /// Обрезка строки по ширине с многоточием.
    pub fn ellipsize(&self, text: &str, px: f32, max_w: f32) -> String {
        if self.measure(text, px) <= max_w {
            return text.to_string();
        }
        let ell = self.advance('…', px);
        let mut w = 0.0;
        let mut out = String::new();
        for ch in text.chars() {
            let a = self.advance(ch, px);
            if w + a + ell > max_w {
                break;
            }
            w += a;
            out.push(ch);
        }
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_loads_and_measures() {
        let f = Font::builtin();
        assert!(f.measure("Привет, World", 14.0) > 50.0);
        let g = f.rasterize('Ж', 20.0);
        assert!(g.w > 5 && g.h > 5);
        assert!(g.alpha.iter().any(|&a| a == 255));
        let o = f.rasterize('o', 20.0);
        // Центр буквы «o» пустой.
        let cx = o.w / 2;
        let cy = o.h / 2;
        assert!(o.alpha[cy * o.w + cx] < 64);
        // Составной глиф (Й = И + бреве).
        let y = f.rasterize('Й', 20.0);
        assert!(y.h > f.rasterize('И', 20.0).h);
    }
}
