//! 2D-холст: кадровый буфер и сглаженные примитивы для интерфейса.
//!
//! Формат пикселя — `0x00RRGGBB` (u32, little-endian в памяти: B, G, R, X),
//! что напрямую подходит для Win32 DIB, X11 ZPixmap и CoreGraphics
//! (`kCGImageAlphaNoneSkipFirst | kCGBitmapByteOrder32Little`).
//!
//! Цвета рисования задаются как `0xAARRGGBB` — альфа используется для смешивания.

/// Прямоугольник в пикселях (целые координаты).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    pub fn shrink(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// Упаковка цвета с альфой.
#[inline]
pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// Непрозрачный цвет.
#[inline]
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    argb(255, r, g, b)
}

/// Замена альфы цвета.
#[inline]
pub fn with_alpha(c: u32, a: u8) -> u32 {
    (c & 0x00FF_FFFF) | ((a as u32) << 24)
}

/// Умножение альфы цвета на коэффициент.
#[inline]
pub fn fade(c: u32, k: f32) -> u32 {
    let a = ((c >> 24) as f32 * k.clamp(0.0, 1.0)) as u32;
    (c & 0x00FF_FFFF) | (a << 24)
}

/// Линейная интерполяция двух цветов (без альфы).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| -> u32 {
        let x = ((a >> s) & 0xFF) as f32;
        let y = ((b >> s) & 0xFF) as f32;
        ((x + (y - x) * t).round() as u32) & 0xFF
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Смешивание `src` (с альфой в старшем байте, домноженной на `cov` 0..=255) поверх `dst`.
#[inline(always)]
pub fn blend(dst: u32, src: u32, cov: u32) -> u32 {
    let a = ((src >> 24) * cov + 127) / 255;
    if a == 0 {
        return dst;
    }
    if a >= 255 {
        return src & 0x00FF_FFFF;
    }
    let ia = 255 - a;
    let rb = ((src & 0x00FF_00FF) * a + (dst & 0x00FF_00FF) * ia + 0x0080_0080) >> 8 & 0x00FF_00FF;
    let g = ((src & 0x0000_FF00) * a + (dst & 0x0000_FF00) * ia + 0x0000_8000) >> 8 & 0x0000_FF00;
    rb | g
}

/// Кадровый буфер с буфером глубины и буфером идентификаторов (для выбора мышью).
#[derive(Clone, Debug, Default)]
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub color: Vec<u32>,
    pub depth: Vec<f32>,
    /// Идентификатор объекта на пикселе (0 — пусто), заполняется 3D-проходом.
    pub ids: Vec<u32>,
    /// Текущая область отсечения 2D-рисования.
    pub clip: Rect,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        let mut fb = Self::default();
        fb.resize(width, height);
        fb
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        let n = width * height;
        self.color = vec![0; n];
        self.depth = vec![f32::INFINITY; n];
        self.ids = vec![0; n];
        self.reset_clip();
    }

    pub fn full_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f32, self.height as f32)
    }

    pub fn reset_clip(&mut self) {
        self.clip = self.full_rect();
    }

    pub fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(&self.full_rect());
    }

    /// Целочисленные границы клипа: (x0, y0, x1, y1), x1/y1 — исключительно.
    #[inline]
    fn clip_bounds(&self) -> (i32, i32, i32, i32) {
        let c = self.clip;
        (
            c.x.floor().max(0.0) as i32,
            c.y.floor().max(0.0) as i32,
            (c.right().ceil() as i32).min(self.width as i32),
            (c.bottom().ceil() as i32).min(self.height as i32),
        )
    }

    pub fn clear(&mut self, c: u32) {
        let c = c & 0x00FF_FFFF;
        self.color.iter_mut().for_each(|p| *p = c);
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, c: u32, cov: u32) {
        let (x0, y0, x1, y1) = self.clip_bounds();
        if x < x0 || y < y0 || x >= x1 || y >= y1 {
            return;
        }
        let i = y as usize * self.width + x as usize;
        self.color[i] = blend(self.color[i], c, cov);
    }

    /// Заливка прямоугольника (с альфой).
    pub fn fill_rect(&mut self, r: Rect, c: u32) {
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let x0 = (r.x.round() as i32).max(cx0);
        let y0 = (r.y.round() as i32).max(cy0);
        let x1 = (r.right().round() as i32).min(cx1);
        let y1 = (r.bottom().round() as i32).min(cy1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let a = c >> 24;
        for y in y0..y1 {
            let row = &mut self.color[y as usize * self.width..][x0 as usize..x1 as usize];
            if a >= 255 {
                let v = c & 0x00FF_FFFF;
                row.iter_mut().for_each(|p| *p = v);
            } else {
                row.iter_mut().for_each(|p| *p = blend(*p, c, 255));
            }
        }
    }

    /// Вертикальный градиент.
    pub fn fill_gradient_v(&mut self, r: Rect, top: u32, bottom: u32) {
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let x0 = (r.x.round() as i32).max(cx0);
        let y0 = (r.y.round() as i32).max(cy0);
        let x1 = (r.right().round() as i32).min(cx1);
        let y1 = (r.bottom().round() as i32).min(cy1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for y in y0..y1 {
            let t = (y as f32 - r.y) / r.h.max(1.0);
            let c = mix(top, bottom, t) & 0x00FF_FFFF;
            let row = &mut self.color[y as usize * self.width..][x0 as usize..x1 as usize];
            row.iter_mut().for_each(|p| *p = c);
        }
    }

    /// Контур прямоугольника толщиной `t`.
    pub fn stroke_rect(&mut self, r: Rect, t: f32, c: u32) {
        self.fill_rect(Rect::new(r.x, r.y, r.w, t), c);
        self.fill_rect(Rect::new(r.x, r.bottom() - t, r.w, t), c);
        self.fill_rect(Rect::new(r.x, r.y + t, t, r.h - 2.0 * t), c);
        self.fill_rect(Rect::new(r.right() - t, r.y + t, t, r.h - 2.0 * t), c);
    }

    /// Заливка скруглённого прямоугольника со сглаживанием углов.
    pub fn fill_rounded(&mut self, r: Rect, radius: f32, c: u32) {
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
        if rad < 0.5 {
            self.fill_rect(r, c);
            return;
        }
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let x0 = (r.x.floor() as i32).max(cx0);
        let y0 = (r.y.floor() as i32).max(cy0);
        let x1 = (r.right().ceil() as i32).min(cx1);
        let y1 = (r.bottom().ceil() as i32).min(cy1);
        let (ix0, iy0, ix1, iy1) = (r.x + rad, r.y + rad, r.right() - rad, r.bottom() - rad);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let dx = if px < ix0 { ix0 - px } else if px > ix1 { px - ix1 } else { 0.0 };
                let dy = if py < iy0 { iy0 - py } else if py > iy1 { py - iy1 } else { 0.0 };
                let cov = if dx == 0.0 && dy == 0.0 {
                    // Внутренняя часть — учитываем дробные края прямоугольника.
                    let ex = (px - r.x + 0.5).min(r.right() - px + 0.5).clamp(0.0, 1.0);
                    let ey = (py - r.y + 0.5).min(r.bottom() - py + 0.5).clamp(0.0, 1.0);
                    ex * ey
                } else {
                    let d = (dx * dx + dy * dy).sqrt();
                    (rad - d + 0.5).clamp(0.0, 1.0)
                };
                if cov > 0.0 {
                    let i = y as usize * self.width + x as usize;
                    self.color[i] = blend(self.color[i], c, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Контур скруглённого прямоугольника.
    pub fn stroke_rounded(&mut self, r: Rect, radius: f32, t: f32, c: u32) {
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
        let (cx0, cy0, cx1, cy1) = self.clip_bounds();
        let x0 = (r.x.floor() as i32).max(cx0);
        let y0 = (r.y.floor() as i32).max(cy0);
        let x1 = (r.right().ceil() as i32).min(cx1);
        let y1 = (r.bottom().ceil() as i32).min(cy1);
        let (ix0, iy0, ix1, iy1) = (r.x + rad, r.y + rad, r.right() - rad, r.bottom() - rad);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let dx = if px < ix0 { ix0 - px } else if px > ix1 { px - ix1 } else { 0.0 };
                let dy = if py < iy0 { iy0 - py } else if py > iy1 { py - iy1 } else { 0.0 };
                // Расстояние до границы со знаком (отрицательно внутри).
                let sd = if dx > 0.0 || dy > 0.0 {
                    (dx * dx + dy * dy).sqrt() - rad
                } else {
                    -(px - r.x).min(r.right() - px).min(py - r.y).min(r.bottom() - py)
                };
                // Полоса [−t, 0].
                let cov = ((sd + t + 0.5).min(0.5 - sd)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = y as usize * self.width + x as usize;
                    self.color[i] = blend(self.color[i], c, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Сглаженный залитый круг.
    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: u32) {
        let (bx0, by0, bx1, by1) = self.clip_bounds();
        let x0 = ((cx - r - 1.0).floor() as i32).max(bx0);
        let y0 = ((cy - r - 1.0).floor() as i32).max(by0);
        let x1 = ((cx + r + 1.0).ceil() as i32).min(bx1);
        let y1 = ((cy + r + 1.0).ceil() as i32).min(by1);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = y as usize * self.width + x as usize;
                    self.color[i] = blend(self.color[i], c, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Сглаженная окружность (кольцо) толщиной `t`.
    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, t: f32, c: u32) {
        let (bx0, by0, bx1, by1) = self.clip_bounds();
        let x0 = ((cx - r - t).floor() as i32).max(bx0);
        let y0 = ((cy - r - t).floor() as i32).max(by0);
        let x1 = ((cx + r + t).ceil() as i32 + 1).min(bx1);
        let y1 = ((cy + r + t).ceil() as i32 + 1).min(by1);
        let ht = t * 0.5;
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = ((dx * dx + dy * dy).sqrt() - r).abs();
                let cov = (ht - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = y as usize * self.width + x as usize;
                    self.color[i] = blend(self.color[i], c, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Сглаженный отрезок толщиной `t` (по расстоянию до отрезка, с круглыми концами).
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, c: u32) {
        let ht = (t * 0.5).max(0.35);
        let (bx0, by0, bx1, by1) = self.clip_bounds();
        let minx = ((x0.min(x1) - ht - 1.0).floor() as i32).max(bx0);
        let miny = ((y0.min(y1) - ht - 1.0).floor() as i32).max(by0);
        let maxx = ((x0.max(x1) + ht + 1.0).ceil() as i32).min(bx1);
        let maxy = ((y0.max(y1) + ht + 1.0).ceil() as i32).min(by1);
        if minx >= maxx || miny >= maxy {
            return;
        }
        // Слишком длинные линии с огромным bbox режем — для UI это не нужно.
        if (maxx - minx) as i64 * (maxy - miny) as i64 > 4_000_000 {
            return;
        }
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let alpha_scale = if t < 1.0 { t.max(0.3) } else { 1.0 };
        for y in miny..maxy {
            let py = y as f32 + 0.5;
            for x in minx..maxx {
                let px = x as f32 + 0.5;
                let tt = (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0);
                let qx = x0 + dx * tt - px;
                let qy = y0 + dy * tt - py;
                let d = (qx * qx + qy * qy).sqrt();
                let cov = (ht - d + 0.5).clamp(0.0, 1.0) * alpha_scale;
                if cov > 0.0 {
                    let i = y as usize * self.width + x as usize;
                    self.color[i] = blend(self.color[i], c, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Ломаная.
    pub fn polyline(&mut self, pts: &[(f32, f32)], t: f32, c: u32, closed: bool) {
        if pts.len() < 2 {
            return;
        }
        for w in pts.windows(2) {
            self.line(w[0].0, w[0].1, w[1].0, w[1].1, t, c);
        }
        if closed {
            let a = pts[pts.len() - 1];
            let b = pts[0];
            self.line(a.0, a.1, b.0, b.1, t, c);
        }
    }

    /// Заливка произвольного многоугольника (non-zero) со сглаживанием.
    pub fn fill_polygon(&mut self, pts: &[(f32, f32)], c: u32) {
        if pts.len() < 3 {
            return;
        }
        let mut minx = f32::MAX;
        let mut miny = f32::MAX;
        let mut maxx = f32::MIN;
        let mut maxy = f32::MIN;
        for &(x, y) in pts {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        let ox = minx.floor();
        let oy = miny.floor();
        let w = (maxx.ceil() - ox) as usize + 2;
        let h = (maxy.ceil() - oy) as usize + 2;
        if w * h > 4_000_000 {
            return;
        }
        let mut cov = Coverage::new(w, h);
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            cov.line(a.0 - ox, a.1 - oy, b.0 - ox, b.1 - oy);
        }
        let mask = cov.finish();
        self.blit_mask(ox as i32, oy as i32, w, h, &mask, c);
    }

    /// Вывод альфа-маски заданным цветом.
    pub fn blit_mask(&mut self, x: i32, y: i32, w: usize, h: usize, mask: &[u8], c: u32) {
        let (bx0, by0, bx1, by1) = self.clip_bounds();
        let sx0 = (bx0 - x).max(0);
        let sy0 = (by0 - y).max(0);
        let sx1 = (bx1 - x).min(w as i32);
        let sy1 = (by1 - y).min(h as i32);
        if sx0 >= sx1 || sy0 >= sy1 {
            return;
        }
        for sy in sy0..sy1 {
            let drow = (y + sy) as usize * self.width;
            let mrow = sy as usize * w;
            for sx in sx0..sx1 {
                let m = mask[mrow + sx as usize] as u32;
                if m == 0 {
                    continue;
                }
                let i = drow + (x + sx) as usize;
                self.color[i] = blend(self.color[i], c, m);
            }
        }
    }

    /// Мягкая тень под панелью.
    pub fn shadow(&mut self, r: Rect, radius: f32, spread: f32, alpha: u8) {
        let steps = spread.max(1.0) as i32;
        for i in (1..=steps).rev() {
            let k = 1.0 - i as f32 / (steps as f32 + 1.0);
            let a = (alpha as f32 * k * k / steps as f32 * 2.0).min(255.0) as u8;
            let d = i as f32;
            self.fill_rounded(
                Rect::new(r.x - d * 0.5, r.y - d * 0.25, r.w + d, r.h + d * 1.25),
                radius + d,
                argb(a, 0, 0, 0),
            );
        }
    }

    /// Масштабированный вывод другого буфера (ближайший сосед) в прямоугольник.
    pub fn blit_scaled(&mut self, src: &Framebuffer, dst: Rect) {
        let x0 = dst.x.max(0.0) as usize;
        let y0 = dst.y.max(0.0) as usize;
        let x1 = (dst.right() as usize).min(self.width);
        let y1 = (dst.bottom() as usize).min(self.height);
        if x0 >= x1 || y0 >= y1 || src.width == 0 || src.height == 0 {
            return;
        }
        let sx = src.width as f32 / dst.w;
        let sy = src.height as f32 / dst.h;
        // Предвычисленные индексы столбцов.
        let cols: Vec<usize> = (x0..x1)
            .map(|x| (((x as f32 - dst.x + 0.5) * sx) as usize).min(src.width - 1))
            .collect();
        for y in y0..y1 {
            let syy = (((y as f32 - dst.y + 0.5) * sy) as usize).min(src.height - 1);
            let srow = &src.color[syy * src.width..(syy + 1) * src.width];
            let srow_ids = &src.ids[syy * src.width..(syy + 1) * src.width];
            let srow_d = &src.depth[syy * src.width..(syy + 1) * src.width];
            let base = y * self.width;
            for (k, &cx) in cols.iter().enumerate() {
                let i = base + x0 + k;
                self.color[i] = srow[cx];
                self.ids[i] = srow_ids[cx];
                self.depth[i] = srow_d[cx];
            }
        }
    }

    /// Копирование области другого буфера того же размера.
    pub fn copy_region_from(&mut self, src: &Framebuffer, r: Rect) {
        if src.width != self.width || src.height != self.height {
            return;
        }
        let x0 = r.x.max(0.0) as usize;
        let y0 = r.y.max(0.0) as usize;
        let x1 = (r.right() as usize).min(self.width);
        let y1 = (r.bottom() as usize).min(self.height);
        for y in y0..y1 {
            let a = y * self.width + x0;
            let b = y * self.width + x1;
            self.color[a..b].copy_from_slice(&src.color[a..b]);
            self.ids[a..b].copy_from_slice(&src.ids[a..b]);
            self.depth[a..b].copy_from_slice(&src.depth[a..b]);
        }
    }

    /// Пиксели в формате RGB8 (для сохранения скриншота).
    pub fn to_rgb8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 3);
        for &p in &self.color {
            out.push((p >> 16) as u8);
            out.push((p >> 8) as u8);
            out.push(p as u8);
        }
        out
    }
}

/// Аналитический растеризатор покрытия (метод накопления площадей).
///
/// Используется для глифов шрифта и векторных иконок: даёт качественное
/// сглаживание при любом масштабе (HiDPI) без внешних библиотек.
pub struct Coverage {
    pub w: usize,
    pub h: usize,
    acc: Vec<f32>,
}

impl Coverage {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, acc: vec![0.0; w * h + 4] }
    }

    /// Добавление ребра контура (координаты в пикселях, y вниз).
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if (y0 - y1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, ax, ay, bx, by) = if y0 < y1 { (1.0, x0, y0, x1, y1) } else { (-1.0, x1, y1, x0, y0) };
        let ax = ax.clamp(0.0, self.w as f32 - 1.001);
        let bx = bx.clamp(0.0, self.w as f32 - 1.001);
        let dxdy = (bx - ax) / (by - ay);
        let mut x = ax;
        if ay < 0.0 {
            x -= ay * dxdy;
        }
        let ystart = ay.max(0.0) as usize;
        let yend = (self.h).min(by.ceil().max(0.0) as usize);
        for y in ystart..yend {
            let linestart = y * self.w;
            let dy = ((y + 1) as f32).min(by) - (y as f32).max(ay);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0floor = x0.floor();
            let x0i = x0floor as i32;
            let x1ceil = x1.ceil();
            let x1i = x1ceil as i32;
            if x0i < 0 {
                x = xnext;
                continue;
            }
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (x + xnext) - x0floor;
                let i = linestart + x0i as usize;
                self.acc[i] += d - d * xmf;
                self.acc[i + 1] += d * xmf;
            } else {
                let s = (x1 - x0).recip();
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                let i = linestart + x0i as usize;
                self.acc[i] += d * a0;
                if x1i == x0i + 2 {
                    self.acc[i + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.acc[i + 1] += d * (a1 - a0);
                    for xi in x0i + 2..x1i - 1 {
                        self.acc[linestart + xi as usize] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.acc[linestart + (x1i - 1) as usize] += d * (1.0 - a2 - am);
                }
                let j = linestart + x1i as usize;
                if j < self.acc.len() {
                    self.acc[j] += d * am;
                }
            }
            x = xnext;
        }
    }

    /// Квадратичная кривая Безье (разбиение на отрезки).
    pub fn quad(&mut self, x0: f32, y0: f32, cx: f32, cy: f32, x1: f32, y1: f32) {
        let ddx = x0 - 2.0 * cx + x1;
        let ddy = y0 - 2.0 * cy + y1;
        let dev = (ddx * ddx + ddy * ddy).sqrt();
        let n = (1 + (dev * 3.0).sqrt() as usize).min(32);
        let mut px = x0;
        let mut py = y0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let qx = mt * mt * x0 + 2.0 * mt * t * cx + t * t * x1;
            let qy = mt * mt * y0 + 2.0 * mt * t * cy + t * t * y1;
            self.line(px, py, qx, qy);
            px = qx;
            py = qy;
        }
    }

    /// Итоговая маска 0..=255.
    pub fn finish(self) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h];
        let mut acc = 0.0f32;
        for (i, o) in out.iter_mut().enumerate() {
            acc += self.acc[i];
            let a = acc.abs().min(1.0);
            *o = (a * 255.0 + 0.5) as u8;
        }
        out
    }
}

/// Минимальный PNG-кодировщик (deflate без сжатия, «stored» блоки).
pub fn encode_png(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8], mut crc: u32) -> u32 {
        crc = !crc;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(ty);
        out.extend_from_slice(data);
        let crc = crc32(data, crc32(ty, 0));
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut raw = Vec::with_capacity((width * 3 + 1) * height);
    for y in 0..height {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * width * 3..(y + 1) * width * 3]);
    }
    // zlib-поток со stored-блоками.
    let mut z = vec![0x78, 0x01];
    let mut pos = 0;
    while pos < raw.len() || raw.is_empty() {
        let n = (raw.len() - pos).min(65535);
        let last = pos + n >= raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[pos..pos + n]);
        pos += n;
        if raw.is_empty() {
            break;
        }
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_works() {
        assert_eq!(blend(0x000000, rgb(255, 255, 255), 255), 0xFFFFFF);
        let half = blend(0x000000, argb(128, 255, 255, 255), 255);
        assert!(((half >> 16) & 0xFF) > 120 && ((half >> 16) & 0xFF) < 136);
    }

    #[test]
    fn coverage_square() {
        let mut c = Coverage::new(10, 10);
        c.line(2.0, 2.0, 8.0, 2.0);
        c.line(8.0, 2.0, 8.0, 8.0);
        c.line(8.0, 8.0, 2.0, 8.0);
        c.line(2.0, 8.0, 2.0, 2.0);
        let m = c.finish();
        assert_eq!(m[5 * 10 + 5], 255);
        assert_eq!(m[0], 0);
        assert_eq!(m[5 * 10 + 9], 0);
    }

    #[test]
    fn png_header() {
        let png = encode_png(2, 2, &[0; 12]);
        assert_eq!(&png[1..4], b"PNG");
    }
}
