//! Программный 3D-рендерер.
//!
//! Оптимизирован для слабых компьютеров без GPU-драйверов:
//! - все этапы (вершины, сборка треугольников, растеризация) распараллелены
//!   через `std::thread::scope`, кадр делится на чередующиеся горизонтальные полосы;
//! - инкрементальные функции рёбер и плоскости атрибутов (без делений в цикле);
//! - буфер «близости» (1/w) — масштабно-независимая точность глубины;
//! - буфер идентификаторов: выбор объекта мышью за O(1);
//! - сглаженные линии рёбер с тестом глубины.

use super::canvas::Framebuffer;
use crate::math::{Aabb, Color, Mat4, Ray, Vec3, Vec4};
use crate::mesh::Mesh;
use std::collections::HashMap;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Камера
// ---------------------------------------------------------------------------

/// Орбитальная камера (ось Z — вверх).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    /// Азимут (рад), 0 — камера на оси +X.
    pub yaw: f32,
    /// Угол возвышения (рад).
    pub pitch: f32,
    pub distance: f32,
    pub ortho: bool,
    pub fov_y: f32,
    /// Плоскости отсечения (обновляются по габаритам сцены).
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::zero(),
            yaw: -60f32.to_radians(),
            pitch: 25f32.to_radians(),
            distance: 250.0,
            ortho: false,
            fov_y: 40f32.to_radians(),
            near: 0.1,
            far: 10000.0,
        }
    }
}

impl Camera {
    /// Единичный вектор от цели к камере.
    pub fn back_dir(&self) -> Vec3 {
        Vec3::new(self.pitch.cos() * self.yaw.cos(), self.pitch.cos() * self.yaw.sin(), self.pitch.sin())
    }
    pub fn eye(&self) -> Vec3 {
        self.target + self.back_dir() * self.distance
    }
    /// Вектор «вверх» камеры (никогда не вырождается).
    pub fn up(&self) -> Vec3 {
        Vec3::new(-self.pitch.sin() * self.yaw.cos(), -self.pitch.sin() * self.yaw.sin(), self.pitch.cos())
    }
    pub fn right(&self) -> Vec3 {
        (-self.back_dir()).cross(self.up()).normalize()
    }
    pub fn view(&self) -> Mat4 {
        Mat4::look_at(self.eye(), self.target, self.up())
    }
    /// Половина высоты видимой области для ортографической проекции.
    pub fn ortho_half_height(&self) -> f32 {
        self.distance * (self.fov_y * 0.5).tan()
    }
    pub fn proj(&self, aspect: f32) -> Mat4 {
        if self.ortho {
            let h = self.ortho_half_height();
            let w = h * aspect;
            Mat4::ortho(-w, w, -h, h, self.near, self.far)
        } else {
            Mat4::perspective(self.fov_y, aspect, self.near, self.far)
        }
    }
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Подстройка ближней/дальней плоскостей под габариты сцены.
    pub fn update_clip(&mut self, bounds: &Aabb) {
        let (center, radius) = if bounds.is_empty() {
            (self.target, self.distance)
        } else {
            (bounds.center(), bounds.diagonal() * 0.5)
        };
        let radius = radius.max(self.distance * 0.5).max(1e-3);
        let d = self.eye().distance(center);
        if self.ortho {
            self.near = -(d + radius * 4.0);
            self.far = d + radius * 4.0;
        } else {
            self.near = (d - radius * 2.0).max(self.distance * 0.002).max(1e-4);
            self.far = d + radius * 4.0;
        }
    }

    /// Луч из пикселя `(sx, sy)` области просмотра `w × h`.
    pub fn ray(&self, sx: f32, sy: f32, w: f32, h: f32) -> Ray {
        let aspect = w / h.max(1.0);
        let nx = (sx / w) * 2.0 - 1.0;
        let ny = 1.0 - (sy / h) * 2.0;
        let fwd = -self.back_dir();
        let up = self.up();
        let right = self.right();
        if self.ortho {
            let hh = self.ortho_half_height();
            let origin = self.eye() + right * (nx * hh * aspect) + up * (ny * hh) - fwd * (self.distance * 10.0);
            Ray::new(origin, fwd)
        } else {
            let t = (self.fov_y * 0.5).tan();
            let dir = fwd + right * (nx * t * aspect) + up * (ny * t);
            Ray::new(self.eye(), dir)
        }
    }

    /// Проекция мировой точки в пиксели области просмотра. `None` — за камерой.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<(f32, f32)> {
        let vp = self.view_proj(w / h.max(1.0));
        let c = vp.transform_vec4(Vec4::new(p.x, p.y, p.z, 1.0));
        if c.w <= 1e-6 {
            return None;
        }
        Some(((c.x / c.w * 0.5 + 0.5) * w, (1.0 - (c.y / c.w * 0.5 + 0.5)) * h))
    }

    /// Мировая точка по пикселю и значению буфера близости.
    pub fn unproject(&self, sx: f32, sy: f32, q: f32, w: f32, h: f32) -> Option<Vec3> {
        if q <= 0.0 {
            return None;
        }
        let ray = self.ray(sx, sy, w, h);
        let fwd = -self.back_dir();
        if self.ortho {
            // q = 1 − z_ndc  →  z_ndc → глубина вида.
            let zn = 1.0 - q;
            let depth = (zn * (self.far - self.near) + (self.far + self.near)) * 0.5;
            let origin_depth = (ray.origin - self.eye()).dot(fwd);
            Some(ray.origin + fwd * (depth - origin_depth))
        } else {
            let wv = 1.0 / q;
            let t = wv / ray.dir.dot(fwd).max(1e-6);
            Some(ray.origin + ray.dir * t)
        }
    }

    /// Размер пикселя в мировых единицах на расстоянии цели.
    pub fn pixel_size(&self, viewport_h: f32) -> f32 {
        2.0 * self.ortho_half_height() / viewport_h.max(1.0)
    }

    /// Вписать габаритный контейнер в вид.
    pub fn fit(&mut self, b: &Aabb, aspect: f32) {
        if b.is_empty() {
            self.target = Vec3::zero();
            self.distance = 250.0;
            return;
        }
        self.target = b.center();
        let r = (b.diagonal() * 0.5).max(1e-3);
        let half = (self.fov_y * 0.5).tan();
        let fit_h = r / half;
        let fit_w = r / (half * aspect.max(0.1));
        self.distance = fit_h.max(fit_w) * 1.15;
    }

    /// Установка стандартного вида по направлению «от цели к камере».
    pub fn set_view_dir(&mut self, dir: Vec3) {
        let d = dir.normalize();
        self.pitch = d.z.clamp(-1.0, 1.0).asin();
        if d.x.abs() > 1e-6 || d.y.abs() > 1e-6 {
            self.yaw = d.y.atan2(d.x);
        } else {
            // Вид сверху/снизу: «верх» экрана — +Y.
            self.yaw = -std::f32::consts::FRAC_PI_2;
        }
    }

    /// Орбитальное вращение.
    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        let lim = std::f32::consts::FRAC_PI_2;
        self.pitch = (self.pitch + dpitch).clamp(-lim, lim);
    }

    /// Панорамирование на пиксели экрана.
    pub fn pan(&mut self, dx: f32, dy: f32, viewport_h: f32) {
        let s = self.pixel_size(viewport_h);
        self.target = self.target - self.right() * (dx * s) + self.up() * (dy * s);
    }

    /// Масштаб к точке под курсором (`factor` < 1 — приближение).
    pub fn zoom_at(&mut self, factor: f32, toward: Option<Vec3>) {
        let factor = factor.clamp(0.2, 5.0);
        let old = self.distance;
        self.distance = (self.distance * factor).clamp(1e-3, 1e7);
        if let Some(p) = toward {
            let k = 1.0 - self.distance / old;
            self.target = self.target + (p - self.target) * k;
        }
    }
}

// ---------------------------------------------------------------------------
// Отображаемый меш (кэш для рендера)
// ---------------------------------------------------------------------------

/// Меш, подготовленный для отображения: нормали с разбиением по углу излома,
/// рёбра-контуры и (лениво) все рёбра для каркасного режима.
#[derive(Debug, Default)]
pub struct DisplayMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub colors: Option<Vec<Color>>,
    pub indices: Vec<u32>,
    /// Характерные рёбра (излом > порога или граница) — пары точек.
    pub feature_edges: Vec<[Vec3; 2]>,
    /// Все рёбра (лениво).
    all_edges: OnceLock<Vec<[Vec3; 2]>>,
    /// Замкнутость (для отсечения задних граней).
    pub closed: bool,
    pub bounds: Aabb,
    src_positions: Vec<Vec3>,
    src_indices: Vec<u32>,
    /// Уникальный идентификатор (для кэша буферов GPU); 0 — не кэшировать.
    pub uid: u64,
}

static NEXT_MESH_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl DisplayMesh {
    /// Построение из меша. `flat` — плоское затенение; иначе гладкое с изломом `crease_deg`.
    pub fn build(mesh: &Mesh, flat: bool, crease_deg: f32) -> DisplayMesh {
        let tri_n = mesh.indices.len() / 3;
        let pos = &mesh.positions;
        let mut face_n = Vec::with_capacity(tri_n);
        for t in 0..tri_n {
            let a = pos[mesh.indices[t * 3] as usize];
            let b = pos[mesh.indices[t * 3 + 1] as usize];
            let c = pos[mesh.indices[t * 3 + 2] as usize];
            face_n.push((b - a).cross(c - a));
        }
        let cos_crease = crease_deg.to_radians().cos();

        // Топология рёбер: ребро → список граней.
        let mut edge_faces: HashMap<(u32, u32), [u32; 3]> = HashMap::with_capacity(tri_n * 2);
        for t in 0..tri_n {
            for k in 0..3 {
                let a = mesh.indices[t * 3 + k];
                let b = mesh.indices[t * 3 + (k + 1) % 3];
                let key = if a < b { (a, b) } else { (b, a) };
                let e = edge_faces.entry(key).or_insert([u32::MAX, u32::MAX, 0]);
                if e[2] == 0 {
                    e[0] = t as u32;
                } else if e[2] == 1 {
                    e[1] = t as u32;
                }
                e[2] += 1;
            }
        }
        let mut closed = tri_n > 0;
        let mut feature_edges = Vec::new();
        for (&(a, b), f) in &edge_faces {
            if f[2] != 2 {
                closed = false;
                if f[2] == 1 {
                    feature_edges.push([pos[a as usize], pos[b as usize]]);
                }
                continue;
            }
            let n1 = face_n[f[0] as usize].normalize();
            let n2 = face_n[f[1] as usize].normalize();
            if n1.dot(n2) < cos_crease {
                feature_edges.push([pos[a as usize], pos[b as usize]]);
            }
        }

        let mut dm = DisplayMesh { closed, bounds: mesh.aabb(), uid: NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed), ..Default::default() };
        let src_colors = mesh.colors.as_ref().filter(|c| c.len() == pos.len());

        if flat {
            dm.positions.reserve(tri_n * 3);
            for t in 0..tri_n {
                let n = face_n[t].normalize();
                for k in 0..3 {
                    let vi = mesh.indices[t * 3 + k] as usize;
                    dm.positions.push(pos[vi]);
                    dm.normals.push(n);
                    dm.indices.push((t * 3 + k) as u32);
                }
            }
            if let Some(c) = src_colors {
                dm.colors = Some(mesh.indices.iter().map(|&i| c[i as usize]).collect());
            }
        } else {
            // Нормаль угла треугольника = сумма нормалей смежных граней в пределах угла излома.
            let mut vert_faces: Vec<Vec<u32>> = vec![Vec::new(); pos.len()];
            for t in 0..tri_n {
                for k in 0..3 {
                    vert_faces[mesh.indices[t * 3 + k] as usize].push(t as u32);
                }
            }
            let mut remap: HashMap<(u32, i32, i32, i32), u32> = HashMap::with_capacity(pos.len() * 2);
            let mut out_colors = src_colors.map(|_| Vec::new());
            for t in 0..tri_n {
                let fnorm = face_n[t].normalize();
                for k in 0..3 {
                    let vi = mesh.indices[t * 3 + k];
                    let mut n = Vec3::zero();
                    for &f in &vert_faces[vi as usize] {
                        let fnn = face_n[f as usize];
                        let fl = fnn.len();
                        if fl > 0.0 && fnorm.dot(fnn * (1.0 / fl)) >= cos_crease {
                            n += fnn;
                        }
                    }
                    let n = if n.len2() > 0.0 { n.normalize() } else { fnorm };
                    let key = (vi, (n.x * 64.0) as i32, (n.y * 64.0) as i32, (n.z * 64.0) as i32);
                    let idx = *remap.entry(key).or_insert_with(|| {
                        dm.positions.push(pos[vi as usize]);
                        dm.normals.push(n);
                        if let (Some(oc), Some(sc)) = (out_colors.as_mut(), src_colors) {
                            oc.push(sc[vi as usize]);
                        }
                        (dm.positions.len() - 1) as u32
                    });
                    dm.indices.push(idx);
                }
            }
            dm.colors = out_colors;
        }
        dm.feature_edges = feature_edges;
        dm.src_positions = mesh.positions.clone();
        dm.src_indices = mesh.indices.clone();
        dm
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Все рёбра (для каркасного режима), вычисляются один раз.
    pub fn all_edges(&self) -> &[[Vec3; 2]] {
        self.all_edges.get_or_init(|| {
            let mut set: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
            let mut out = Vec::new();
            for t in self.src_indices.chunks_exact(3) {
                for k in 0..3 {
                    let a = t[k];
                    let b = t[(k + 1) % 3];
                    let key = if a < b { (a, b) } else { (b, a) };
                    if set.insert(key) {
                        out.push([self.src_positions[a as usize], self.src_positions[b as usize]]);
                    }
                }
            }
            out
        })
    }
}

/// Быстрый отпечаток меша для инвалидации кэша.
pub fn mesh_fingerprint(m: &Mesh) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mixu = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    mixu(m.positions.len() as u64);
    mixu(m.indices.len() as u64);
    let n = m.positions.len();
    if n > 0 {
        let step = (n / 61).max(1);
        let mut i = 0;
        while i < n {
            let p = m.positions[i];
            mixu(p.x.to_bits() as u64);
            mixu(p.y.to_bits() as u64);
            mixu(p.z.to_bits() as u64);
            i += step;
        }
        let p = m.positions[n - 1];
        mixu(p.x.to_bits() as u64 ^ p.z.to_bits() as u64);
    }
    let ni = m.indices.len();
    if ni > 0 {
        let step = (ni / 61).max(1);
        let mut i = 0;
        while i < ni {
            mixu(m.indices[i] as u64);
            i += step;
        }
    }
    h
}

// ---------------------------------------------------------------------------
// Параметры кадра
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Shaded,
    ShadedEdges,
    Wireframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Highlight {
    None,
    Hover,
    Selected,
}

/// Объект для отрисовки.
pub struct RenderItem<'a> {
    /// Идентификатор для выбора (≠ 0).
    pub id: u32,
    pub mesh: &'a DisplayMesh,
    pub model: Mat4,
    pub color: Color,
    pub highlight: Highlight,
    /// Режим каркаса только для этого объекта.
    pub wire: bool,
    /// Полупрозрачный «призрак» (без записи в буфер выбора).
    pub ghost: bool,
}

/// 3D-отрезок для наложений (сетка, эскизы, манипуляторы, измерения).
#[derive(Clone, Copy, Debug)]
pub struct Line3 {
    pub a: Vec3,
    pub b: Vec3,
    pub color: u32,
    pub width: f32,
    pub depth_test: bool,
}

#[derive(Clone, Debug)]
pub struct RenderSettings {
    pub mode: DisplayMode,
    pub threads: usize,
    /// Множитель толщины линий (HiDPI).
    pub line_scale: f32,
    pub bg_top: u32,
    pub bg_bottom: u32,
    /// Пропуск рёбер (во время навигации на слабых ПК).
    pub skip_edges: bool,
    pub edge_color: u32,
    pub select_color: Color,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            mode: DisplayMode::ShadedEdges,
            threads: default_threads(),
            line_scale: 1.0,
            bg_top: 0xFFFF_FFFF,
            bg_bottom: 0xFFC9_D3DE,
            skip_edges: false,
            edge_color: 0xD028_2C33,
            select_color: Color::rgb(0, 122, 217),
        }
    }
}

/// Число потоков рендера по умолчанию.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 16)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub triangles: usize,
    pub drawn_triangles: usize,
    pub lines: usize,
    pub millis: f32,
}

// ---------------------------------------------------------------------------
// Внутренние структуры конвейера
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ClipVert {
    p: Vec4,
    /// Цвет лицевой и обратной стороны (0..255).
    cf: [f32; 3],
    cb: [f32; 3],
}

#[derive(Clone, Copy)]
struct STri {
    minx: i32,
    maxx: i32,
    miny: i32,
    maxy: i32,
    e: [[f32; 3]; 3],
    q: [f32; 3],
    r: [f32; 3],
    g: [f32; 3],
    b: [f32; 3],
    id: u32,
    ghost: bool,
}

#[derive(Clone, Copy)]
struct SLine {
    x0: f32,
    y0: f32,
    q0: f32,
    x1: f32,
    y1: f32,
    q1: f32,
    color: u32,
    width: f32,
    depth: bool,
}

struct FrameCtx {
    w: f32,
    h: f32,
    ortho: bool,
    bias_rel: f32,
    bias_abs: f32,
}

impl FrameCtx {
    #[inline]
    fn to_screen(&self, p: Vec4) -> (f32, f32, f32) {
        let iw = 1.0 / p.w;
        let x = (p.x * iw * 0.5 + 0.5) * self.w;
        let y = (0.5 - p.y * iw * 0.5) * self.h;
        let q = if self.ortho { 1.0 - p.z * iw } else { iw };
        (x, y, q)
    }
}

/// Отсечение многоугольника по ближней плоскости (z ≥ −w).
fn clip_near(input: &[ClipVert], out: &mut Vec<ClipVert>) {
    out.clear();
    let n = input.len();
    for i in 0..n {
        let a = input[i];
        let b = input[(i + 1) % n];
        let da = a.p.z + a.p.w;
        let db = b.p.z + b.p.w;
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let lerp4 = |x: Vec4, y: Vec4| Vec4::new(x.x + (y.x - x.x) * t, x.y + (y.y - x.y) * t, x.z + (y.z - x.z) * t, x.w + (y.w - x.w) * t);
            let lerp3 = |x: [f32; 3], y: [f32; 3]| [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t];
            out.push(ClipVert { p: lerp4(a.p, b.p), cf: lerp3(a.cf, b.cf), cb: lerp3(a.cb, b.cb) });
        }
    }
}

/// Освещение: возвращает (диффузная, блик) для нормали `n` (мировая) и направления на камеру `v`.
#[inline]
fn light(n: Vec3, v: Vec3, l1: Vec3, l2: Vec3) -> (f32, f32) {
    let d1 = n.dot(l1).max(0.0);
    let d2 = n.dot(l2).max(0.0);
    let hv = (l1 + v).normalize();
    let s = n.dot(hv).max(0.0);
    let s2 = s * s;
    let s4 = s2 * s2;
    let s16 = s4 * s4 * s4 * s4;
    (0.30 + 0.58 * d1 + 0.22 * d2 + 0.10 * n.z.max(0.0), 0.22 * s16)
}

// ---------------------------------------------------------------------------
// Основной проход
// ---------------------------------------------------------------------------

/// Отрисовка сцены в буфер (весь буфер — область просмотра).
pub fn render(fb: &mut Framebuffer, cam: &Camera, items: &[RenderItem], overlay: &[Line3], s: &RenderSettings) -> RenderStats {
    let t0 = std::time::Instant::now();
    let w = fb.width;
    let h = fb.height;
    let aspect = w as f32 / h as f32;
    let vp = cam.view_proj(aspect);
    let ctx = FrameCtx {
        w: w as f32,
        h: h as f32,
        ortho: cam.ortho,
        bias_rel: if cam.ortho { 0.0 } else { 2.5e-3 },
        bias_abs: if cam.ortho { 2.0 * 2.5e-3 * cam.distance / (cam.far - cam.near).max(1e-6) } else { 0.0 },
    };
    let eye = cam.eye();
    let back = cam.back_dir();
    // Свет: «фара» (слегка сверху-слева от камеры) и заполняющий.
    let (l1, l2) = lights(cam);
    let threads = s.threads.max(1);

    // Фон.
    for y in 0..h {
        let c = super::canvas::mix(s.bg_top, s.bg_bottom, y as f32 / h as f32) & 0x00FF_FFFF;
        fb.color[y * w..(y + 1) * w].iter_mut().for_each(|p| *p = c);
    }
    fb.depth.iter_mut().for_each(|d| *d = 0.0);
    fb.ids.iter_mut().for_each(|d| *d = 0);

    let mut stats = RenderStats::default();

    // --- Этап 1-2: вершины и сборка треугольников по объектам, параллельно по чанкам.
    let n_bands = (threads * 3).max(1).min(h.max(1));
    let band_h = (h + n_bands - 1) / n_bands;

    struct Job<'a> {
        item: &'a RenderItem<'a>,
        tri_start: usize,
        tri_end: usize,
    }
    let mut jobs: Vec<Job> = Vec::new();
    const CHUNK: usize = 16384;
    for it in items {
        let tn = it.mesh.triangle_count();
        stats.triangles += tn;
        if it.wire && !it.ghost {
            continue;
        }
        let mut st = 0;
        while st < tn {
            let en = (st + CHUNK).min(tn);
            jobs.push(Job { item: it, tri_start: st, tri_end: en });
            st = en;
        }
    }

    let sel = s.select_color;
    let setup = |job: &Job| -> Vec<Vec<STri>> {
        let it = job.item;
        let m = it.model;
        let nm = m.normal_matrix();
        let mvp = vp * m;
        let dm = it.mesh;
        let mut bins: Vec<Vec<STri>> = (0..n_bands).map(|_| Vec::new()).collect();
        let base = base_color(it, sel);
        // Кэш преобразованных вершин в пределах чанка.
        let mut cache: HashMap<u32, ClipVert> = HashMap::with_capacity((job.tri_end - job.tri_start) * 2);
        let mut poly = Vec::with_capacity(6);
        let mut clipped = Vec::with_capacity(8);
        let shade = |vi: u32| -> ClipVert {
            let p = dm.positions[vi as usize];
            let wp = m.transform_point(p);
            let n = nm.transform_vector(dm.normals[vi as usize]).normalize();
            let v = if cam.ortho { back } else { (eye - wp).normalize() };
            let (df, sf) = light(n, v, l1, l2);
            let (db, sb) = light(-n, v, l1, l2);
            let bc = match &dm.colors {
                Some(c) if it.highlight == Highlight::None => c[vi as usize],
                _ => base,
            };
            let (r, g, b) = (bc.r as f32, bc.g as f32, bc.b as f32);
            let cf = [r * df + 255.0 * sf, g * df + 255.0 * sf, b * df + 255.0 * sf];
            let cb = [r * db * 0.85 + 255.0 * sb, g * db * 0.85 + 255.0 * sb, b * db * 0.85 + 255.0 * sb];
            let c = mvp.transform_vec4(Vec4::new(p.x, p.y, p.z, 1.0));
            ClipVert { p: c, cf, cb }
        };
        for t in job.tri_start..job.tri_end {
            let ia = dm.indices[t * 3];
            let ib = dm.indices[t * 3 + 1];
            let ic = dm.indices[t * 3 + 2];
            let va = *cache.entry(ia).or_insert_with(|| shade(ia));
            let vb = *cache.entry(ib).or_insert_with(|| shade(ib));
            let vc = *cache.entry(ic).or_insert_with(|| shade(ic));
            // Отсечение по x/y целиком вне.
            let outside = |f: &dyn Fn(&Vec4) -> bool| f(&va.p) && f(&vb.p) && f(&vc.p);
            if outside(&|p| p.x > p.w) || outside(&|p| p.x < -p.w) || outside(&|p| p.y > p.w) || outside(&|p| p.y < -p.w) {
                continue;
            }
            poly.clear();
            poly.push(va);
            poly.push(vb);
            poly.push(vc);
            let verts: &[ClipVert] = if va.p.z + va.p.w < 0.0 || vb.p.z + vb.p.w < 0.0 || vc.p.z + vc.p.w < 0.0 {
                clip_near(&poly, &mut clipped);
                &clipped
            } else {
                &poly
            };
            if verts.len() < 3 {
                continue;
            }
            let scr: Vec<(f32, f32, f32)> = verts.iter().map(|v| ctx.to_screen(v.p)).collect();
            for k in 1..verts.len() - 1 {
                let (a, b, c) = (0, k, k + 1);
                let (x0, y0, q0) = scr[a];
                let (x1, y1, q1) = scr[b];
                let (x2, y2, q2) = scr[c];
                let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
                if area.abs() < 1e-9 {
                    continue;
                }
                // На экране (y вниз) лицевые грани (CCW в мире) имеют area < 0.
                let front = area < 0.0;
                if !front && dm.closed && !it.ghost {
                    continue;
                }
                let pick = |v: &ClipVert| if front { v.cf } else { v.cb };
                let (ca, cb, cc) = (pick(&verts[a]), pick(&verts[b]), pick(&verts[c]));
                // Упорядочиваем с положительной площадью.
                let (p0, p1, p2, c0, c1, c2) = if area > 0.0 {
                    ((x0, y0, q0), (x1, y1, q1), (x2, y2, q2), ca, cb, cc)
                } else {
                    ((x0, y0, q0), (x2, y2, q2), (x1, y1, q1), ca, cc, cb)
                };
                let area = area.abs();
                let minx = (p0.0.min(p1.0).min(p2.0).floor() as i32).max(0);
                let maxx = (p0.0.max(p1.0).max(p2.0).ceil() as i32).min(w as i32 - 1);
                let miny = (p0.1.min(p1.1).min(p2.1).floor() as i32).max(0);
                let maxy = (p0.1.max(p1.1).max(p2.1).ceil() as i32).min(h as i32 - 1);
                if minx > maxx || miny > maxy {
                    continue;
                }
                // Рёбра: E_i(x, y) = A x + B y + C, положительно внутри.
                let edge = |pa: (f32, f32, f32), pb: (f32, f32, f32)| -> [f32; 3] {
                    [pa.1 - pb.1, pb.0 - pa.0, pa.0 * pb.1 - pb.0 * pa.1]
                };
                let e0 = edge(p1, p2);
                let e1 = edge(p2, p0);
                let e2 = edge(p0, p1);
                let inv = 1.0 / area;
                let plane = |a0: f32, a1: f32, a2: f32| -> [f32; 3] {
                    [
                        (a0 * e0[0] + a1 * e1[0] + a2 * e2[0]) * inv,
                        (a0 * e0[1] + a1 * e1[1] + a2 * e2[1]) * inv,
                        (a0 * e0[2] + a1 * e1[2] + a2 * e2[2]) * inv,
                    ]
                };
                let tri = STri {
                    minx,
                    maxx,
                    miny,
                    maxy,
                    e: [e0, e1, e2],
                    q: plane(p0.2, p1.2, p2.2),
                    r: plane(c0[0], c1[0], c2[0]),
                    g: plane(c0[1], c1[1], c2[1]),
                    b: plane(c0[2], c1[2], c2[2]),
                    id: it.id,
                    ghost: it.ghost,
                };
                let b0 = miny as usize / band_h;
                let b1 = (maxy as usize / band_h).min(n_bands - 1);
                for bi in b0..=b1 {
                    bins[bi].push(tri);
                }
            }
        }
        bins
    };

    let mut all_bins: Vec<Vec<Vec<STri>>> = Vec::new();
    if threads <= 1 || jobs.len() <= 1 {
        for j in &jobs {
            all_bins.push(setup(j));
        }
    } else {
        let per = (jobs.len() + threads - 1) / threads;
        std::thread::scope(|sc| {
            let handles: Vec<_> = jobs
                .chunks(per)
                .map(|chunk| {
                    let setup = &setup;
                    sc.spawn(move || chunk.iter().map(setup).collect::<Vec<_>>())
                })
                .collect();
            for hnd in handles {
                all_bins.extend(hnd.join().unwrap_or_default());
            }
        });
    }

    // --- Линии: рёбра объектов + наложения.
    let mut lines: Vec<SLine> = Vec::new();
    let ls = s.line_scale;
    let push_line = |lines: &mut Vec<SLine>, mvp: &Mat4, a: Vec3, b: Vec3, color: u32, width: f32, depth: bool| {
        let mut pa = mvp.transform_vec4(Vec4::new(a.x, a.y, a.z, 1.0));
        let mut pb = mvp.transform_vec4(Vec4::new(b.x, b.y, b.z, 1.0));
        let da = pa.z + pa.w;
        let db = pb.z + pb.w;
        if da < 0.0 && db < 0.0 {
            return;
        }
        if da < 0.0 || db < 0.0 {
            let t = da / (da - db);
            let p = Vec4::new(pa.x + (pb.x - pa.x) * t, pa.y + (pb.y - pa.y) * t, pa.z + (pb.z - pa.z) * t, pa.w + (pb.w - pa.w) * t);
            if da < 0.0 {
                pa = p;
            } else {
                pb = p;
            }
        }
        if (pa.x > pa.w && pb.x > pb.w) || (pa.x < -pa.w && pb.x < -pb.w) || (pa.y > pa.w && pb.y > pb.w) || (pa.y < -pa.w && pb.y < -pb.w) {
            return;
        }
        let (x0, y0, q0) = ctx.to_screen(pa);
        let (x1, y1, q1) = ctx.to_screen(pb);
        if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
            return;
        }
        lines.push(SLine { x0, y0, q0, x1, y1, q1, color, width, depth });
    };
    for it in items {
        let mvp = vp * it.model;
        let Some((color, width, all)) = edge_style(it, s) else { continue };
        let edges: &[[Vec3; 2]] = if all { it.mesh.all_edges() } else { &it.mesh.feature_edges };
        for e in edges {
            push_line(&mut lines, &mvp, e[0], e[1], color, width, true);
        }
    }
    for l in overlay {
        push_line(&mut lines, &vp, l.a, l.b, l.color, l.width * ls, l.depth_test);
    }
    stats.lines = lines.len();

    // Распределение линий по полосам.
    let mut line_bins: Vec<Vec<u32>> = (0..n_bands).map(|_| Vec::new()).collect();
    for (i, l) in lines.iter().enumerate() {
        let pad = l.width + 2.0;
        let y0 = (l.y0.min(l.y1) - pad).max(0.0);
        let y1 = (l.y0.max(l.y1) + pad).min(h as f32 - 1.0);
        if y1 < 0.0 || y0 > h as f32 - 1.0 {
            continue;
        }
        let b0 = (y0 as usize / band_h).min(n_bands - 1);
        let b1 = (y1 as usize / band_h).min(n_bands - 1);
        for bi in b0..=b1 {
            line_bins[bi].push(i as u32);
        }
    }

    // --- Этап 3: растеризация по полосам.
    let drawn = std::sync::atomic::AtomicUsize::new(0);
    {
        let mut bands: Vec<(usize, &mut [u32], &mut [f32], &mut [u32])> = Vec::new();
        let rows = band_h * w;
        for (bi, ((c, d), id)) in fb
            .color
            .chunks_mut(rows)
            .zip(fb.depth.chunks_mut(rows))
            .zip(fb.ids.chunks_mut(rows))
            .enumerate()
        {
            bands.push((bi, c, d, id));
        }
        let all_bins = &all_bins;
        let lines = &lines;
        let line_bins = &line_bins;
        let ctx = &ctx;
        let drawn = &drawn;
        let work = move |band: (usize, &mut [u32], &mut [f32], &mut [u32])| {
            let (bi, color, depth, ids) = band;
            let y0 = bi * band_h;
            let y1 = (y0 + band_h).min(h);
            let mut cnt = 0;
            for job_bins in all_bins {
                for tri in &job_bins[bi] {
                    raster_tri(tri, color, depth, ids, w, y0, y1);
                    cnt += 1;
                }
            }
            for &li in &line_bins[bi] {
                raster_line(&lines[li as usize], color, depth, w, y0, y1, ctx);
            }
            drawn.fetch_add(cnt, std::sync::atomic::Ordering::Relaxed);
        };
        if threads <= 1 {
            for b in bands {
                work(b);
            }
        } else {
            let mut per_thread: Vec<Vec<(usize, &mut [u32], &mut [f32], &mut [u32])>> = (0..threads).map(|_| Vec::new()).collect();
            for (k, b) in bands.into_iter().enumerate() {
                per_thread[k % threads].push(b);
            }
            std::thread::scope(|sc| {
                for list in per_thread {
                    let work = &work;
                    sc.spawn(move || {
                        for b in list {
                            work(b);
                        }
                    });
                }
            });
        }
    }
    stats.drawn_triangles = drawn.load(std::sync::atomic::Ordering::Relaxed);
    stats.millis = t0.elapsed().as_secs_f32() * 1000.0;
    stats
}

/// Стиль рёбер объекта: (цвет, толщина в пикселях, все рёбра?) или `None`, если рёбра не рисуются.
/// Общий для программного и GPU-рендера.
pub fn edge_style(it: &RenderItem, s: &RenderSettings) -> Option<(u32, f32, bool)> {
    let ls = s.line_scale;
    let show_edges = it.wire || s.mode == DisplayMode::Wireframe || (s.mode == DisplayMode::ShadedEdges && !s.skip_edges) || it.highlight != Highlight::None;
    if !show_edges {
        return None;
    }
    let wire = it.wire || s.mode == DisplayMode::Wireframe;
    let (color, width) = match it.highlight {
        Highlight::Selected => (0xFF00_7AD9, 1.6 * ls),
        Highlight::Hover => (0xE04A_A8FF, 1.3 * ls),
        Highlight::None => {
            if wire {
                (0xC040_4650, 1.0 * ls)
            } else {
                (s.edge_color, 1.0 * ls)
            }
        }
    };
    let all = wire && !(s.skip_edges && it.mesh.triangle_count() > 50_000);
    let color = if it.ghost { super::canvas::fade(color, 0.5) } else { color };
    Some((color, width, all))
}

/// Базовый цвет объекта с учётом подсветки.
pub fn base_color(it: &RenderItem, sel: Color) -> Color {
    match it.highlight {
        Highlight::Selected => {
            let c = it.color;
            Color::rgb(
                ((c.r as u32 * 3 + sel.r as u32 * 2) / 5) as u8,
                ((c.g as u32 * 3 + sel.g as u32 * 2) / 5) as u8,
                ((c.b as u32 * 3 + sel.b as u32 * 2) / 5) as u8,
            )
        }
        Highlight::Hover => it.color.lerp(Color::rgb(120, 190, 255), 0.35),
        Highlight::None => it.color,
    }
}

/// Направления двух источников света для камеры.
pub fn lights(cam: &Camera) -> (Vec3, Vec3) {
    let back = cam.back_dir();
    let l1 = (back + cam.up() * 0.55 - cam.right() * 0.35).normalize();
    let l2 = (-back + cam.up() * 0.3 + cam.right() * 0.8).normalize();
    (l1, l2)
}

#[inline]
fn raster_tri(t: &STri, color: &mut [u32], depth: &mut [f32], ids: &mut [u32], w: usize, y0: usize, y1: usize) {
    let ys = (t.miny as usize).max(y0);
    let ye = ((t.maxy + 1) as usize).min(y1);
    if ys >= ye {
        return;
    }
    let [e0, e1, e2] = t.e;
    let fx = t.minx as f32 + 0.5;
    for y in ys..ye {
        let fy = y as f32 + 0.5;
        let mut w0 = e0[0] * fx + e0[1] * fy + e0[2];
        let mut w1 = e1[0] * fx + e1[1] * fy + e1[2];
        let mut w2 = e2[0] * fx + e2[1] * fy + e2[2];
        let mut q = t.q[0] * fx + t.q[1] * fy + t.q[2];
        let mut r = t.r[0] * fx + t.r[1] * fy + t.r[2];
        let mut g = t.g[0] * fx + t.g[1] * fy + t.g[2];
        let mut b = t.b[0] * fx + t.b[1] * fy + t.b[2];
        let row = (y - y0) * w;
        let mut inside_seen = false;
        for x in t.minx..=t.maxx {
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                inside_seen = true;
                let i = row + x as usize;
                if q > depth[i] {
                    let px = ((r.clamp(0.0, 255.0) as u32) << 16) | ((g.clamp(0.0, 255.0) as u32) << 8) | (b.clamp(0.0, 255.0) as u32);
                    if t.ghost {
                        color[i] = super::canvas::blend(color[i], px | 0x5A00_0000, 255);
                    } else {
                        color[i] = px;
                        depth[i] = q;
                        ids[i] = t.id;
                    }
                }
            } else if inside_seen {
                break;
            }
            w0 += e0[0];
            w1 += e1[0];
            w2 += e2[0];
            q += t.q[0];
            r += t.r[0];
            g += t.g[0];
            b += t.b[0];
        }
    }
}

#[inline]
fn raster_line(l: &SLine, color: &mut [u32], depth: &[f32], w: usize, y0: usize, y1: usize, ctx: &FrameCtx) {
    let dx = l.x1 - l.x0;
    let dy = l.y1 - l.y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-4 {
        return;
    }
    let hw = l.width.max(0.6) * 0.5;
    let alpha = l.color >> 24;
    let rgb = l.color & 0x00FF_FFFF;
    let wi = w as i32;
    let plot = |color: &mut [u32], x: i32, y: i32, cov: f32, q: f32| {
        if x < 0 || x >= wi || (y as usize) < y0 || y as usize >= y1 || y < 0 || cov <= 0.0 {
            return;
        }
        let i = (y as usize - y0) * w + x as usize;
        if l.depth && q + ctx.bias_abs + q * ctx.bias_rel < depth[i] {
            return;
        }
        let a = (alpha as f32 * cov.min(1.0)) as u32;
        color[i] = super::canvas::blend(color[i], rgb | (a << 24), 255);
    };
    if dx.abs() >= dy.abs() {
        // x — главная ось. Толщина по вертикали = hw / cos.
        let k = dy / dx;
        let tw = hw * len / dx.abs();
        let (xa, ya, qa, xb, qb) = if l.x0 <= l.x1 { (l.x0, l.y0, l.q0, l.x1, l.q1) } else { (l.x1, l.y1, l.q1, l.x0, l.q0) };
        let mut xs = xa.floor() as i32;
        let mut xe = xb.ceil() as i32;
        // Ограничение по полосе.
        if k.abs() > 1e-6 {
            let xa_band = xa + ((y0 as f32 - tw - 1.0) - ya) / k;
            let xb_band = xa + ((y1 as f32 + tw + 1.0) - ya) / k;
            let (lo, hi) = if xa_band < xb_band { (xa_band, xb_band) } else { (xb_band, xa_band) };
            xs = xs.max(lo.floor() as i32);
            xe = xe.min(hi.ceil() as i32);
        } else if ya + tw + 1.0 < y0 as f32 || ya - tw - 1.0 > y1 as f32 {
            return;
        }
        xs = xs.max(0);
        xe = xe.min(wi - 1);
        let span = (xb - xa).max(1e-6);
        for x in xs..=xe {
            let fx = x as f32 + 0.5;
            // Концевое покрытие.
            let ec = ((fx - xa + 0.5).min(xb - fx + 0.5)).clamp(0.0, 1.0);
            if ec <= 0.0 {
                continue;
            }
            let t = ((fx - xa) / span).clamp(0.0, 1.0);
            let yc = ya + k * (fx - xa);
            let q = qa + (qb - qa) * t;
            let yi0 = (yc - tw - 0.5).floor() as i32;
            let yi1 = (yc + tw + 0.5).ceil() as i32;
            for y in yi0..=yi1 {
                let d = (y as f32 + 0.5 - yc).abs();
                let cov = (tw + 0.5 - d).clamp(0.0, 1.0) * ec;
                plot(color, x, y, cov, q);
            }
        }
    } else {
        let k = dx / dy;
        let tw = hw * len / dy.abs();
        let (xa, ya, qa, yb, qb) = if l.y0 <= l.y1 { (l.x0, l.y0, l.q0, l.y1, l.q1) } else { (l.x1, l.y1, l.q1, l.y0, l.q0) };
        let ys = (ya.floor() as i32).max(y0 as i32);
        let ye = (yb.ceil() as i32).min(y1 as i32 - 1);
        let span = (yb - ya).max(1e-6);
        for y in ys..=ye {
            let fy = y as f32 + 0.5;
            let ec = ((fy - ya + 0.5).min(yb - fy + 0.5)).clamp(0.0, 1.0);
            if ec <= 0.0 {
                continue;
            }
            let t = ((fy - ya) / span).clamp(0.0, 1.0);
            let xc = xa + k * (fy - ya);
            let q = qa + (qb - qa) * t;
            let xi0 = (xc - tw - 0.5).floor() as i32;
            let xi1 = (xc + tw + 0.5).ceil() as i32;
            for x in xi0..=xi1 {
                let d = (x as f32 + 0.5 - xc).abs();
                let cov = (tw + 0.5 - d).clamp(0.0, 1.0) * ec;
                plot(color, x, y, cov, q);
            }
        }
    }
}

/// Линии координатной сетки на плоскости XY вокруг цели камеры.
pub fn grid_lines(cam: &Camera, out: &mut Vec<Line3>, dark_bg: bool) -> f32 {
    let target = cam.target;
    let extent = cam.distance * 2.5;
    let raw = cam.distance / 12.0;
    let p10 = 10f32.powf(raw.max(1e-6).log10().floor());
    let step = if raw / p10 >= 5.0 {
        p10 * 5.0
    } else if raw / p10 >= 2.0 {
        p10 * 2.0
    } else {
        p10
    };
    let major = step * 5.0;
    let n = (extent / step).ceil() as i32;
    let cx = (target.x / major).round() * major;
    let cy = (target.y / major).round() * major;
    let half = n as f32 * step;
    // Ослабление сетки при взгляде вдоль плоскости.
    let k = cam.back_dir().z.abs().sqrt().clamp(0.15, 1.0);
    for i in -n..=n {
        let off = i as f32 * step;
        let x = cx + off;
        let y = cy + off;
        let is_major = ((x / major).round() * major - x).abs() < step * 0.01;
        let fadef = 1.0 - (off.abs() / half).powi(2);
        let a = ((if is_major { 70.0 } else { 34.0 }) * fadef * k) as u8;
        if a < 3 {
            continue;
        }
        let col = if dark_bg { super::canvas::argb(a.saturating_add(a / 3), 170, 178, 190) } else { super::canvas::argb(a, 60, 70, 85) };
        if x.abs() > step * 0.01 {
            out.push(Line3 { a: Vec3::new(x, cy - half, 0.0), b: Vec3::new(x, cy + half, 0.0), color: col, width: 1.0, depth_test: true });
        }
        if y.abs() > step * 0.01 {
            out.push(Line3 { a: Vec3::new(cx - half, y, 0.0), b: Vec3::new(cx + half, y, 0.0), color: col, width: 1.0, depth_test: true });
        }
    }
    // Оси X (красная) и Y (зелёная).
    let ax = (200.0 * k) as u8;
    out.push(Line3 { a: Vec3::new(cx - half, 0.0, 0.0), b: Vec3::new(cx + half, 0.0, 0.0), color: super::canvas::argb(ax, 220, 60, 60), width: 1.3, depth_test: true });
    out.push(Line3 { a: Vec3::new(0.0, cy - half, 0.0), b: Vec3::new(0.0, cy + half, 0.0), color: super::canvas::argb(ax, 60, 170, 70), width: 1.3, depth_test: true });
    step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn renders_box_and_picks() {
        let mesh = primitive_box(10.0, 10.0, 10.0);
        let dm = DisplayMesh::build(&mesh, false, 30.0);
        assert!(dm.closed);
        assert_eq!(dm.feature_edges.len(), 12);
        let mut fb = Framebuffer::new(160, 120);
        let mut cam = Camera::default();
        cam.fit(&mesh.aabb(), 160.0 / 120.0);
        cam.update_clip(&mesh.aabb());
        let items = [RenderItem { id: 7, mesh: &dm, model: Mat4::identity(), color: Color::rgb(200, 200, 200), highlight: Highlight::None, wire: false, ghost: false }];
        let mut settings = RenderSettings::default();
        settings.threads = 3;
        let st = render(&mut fb, &cam, &items, &[], &settings);
        assert!(st.drawn_triangles > 0);
        // Центр кадра — объект.
        let c = 60 * 160 + 80;
        assert_eq!(fb.ids[c], 7);
        assert!(fb.depth[c] > 0.0);
        // Восстановленная точка лежит на поверхности куба.
        let p = cam.unproject(80.5, 60.5, fb.depth[c], 160.0, 120.0).unwrap();
        let on_face = (p.x.abs() - 5.0).abs() < 0.3 || (p.y.abs() - 5.0).abs() < 0.3 || (p.z - 10.0).abs() < 0.3 || p.z.abs() < 0.3;
        assert!(on_face, "p={p:?}");
        // Угол — фон.
        assert_eq!(fb.ids[0], 0);
    }

    #[test]
    fn single_and_multi_thread_match() {
        let mesh = crate::mesh::primitive_sphere(5.0, 24, 12);
        let dm = DisplayMesh::build(&mesh, false, 30.0);
        let mut cam = Camera::default();
        cam.fit(&mesh.aabb(), 1.0);
        cam.update_clip(&mesh.aabb());
        let items = [RenderItem { id: 1, mesh: &dm, model: Mat4::identity(), color: Color::rgb(200, 200, 200), highlight: Highlight::None, wire: false, ghost: false }];
        let mut s = RenderSettings::default();
        let mut a = Framebuffer::new(97, 89);
        let mut b = Framebuffer::new(97, 89);
        s.threads = 1;
        render(&mut a, &cam, &items, &[], &s);
        s.threads = 4;
        render(&mut b, &cam, &items, &[], &s);
        assert_eq!(a.ids, b.ids);
    }
}
