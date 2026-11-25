//! Операции моделирования в духе Fusion 360 и Blender.
//!
//! Твёрдые тела (Fusion 360): вращение (Revolve), сдвиг по траектории
//! (Sweep), лофт по сечениям (Loft), оболочка (Shell), массивы (Pattern).
//! Полигональное моделирование (Blender): подразделение (Subdivide — Loop
//! и простое), сглаживание (Laplacian / Taubin), придание толщины
//! (Solidify). Сложные формы: скручивание, сужение и изгиб (Simple Deform),
//! пружина (Coil).
//!
//! Все функции чистые: принимают меши/эскизы и возвращают новый меш.
//! Тела — треугольные сетки (без B-rep), поэтому операции работают с
//! полилиниями профилей и вершинами сетки.

use crate::math::*;
use crate::mesh::Mesh;
use crate::sketch::{ear_clip, signed_area, triangulate_region, Region, Sketch};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Протягивание областей профиля (общая основа Revolve / Sweep / Coil)
// ---------------------------------------------------------------------------

/// Протягивание областей профиля через `steps` сечений. Точка `p` области
/// на сечении `k` переходит в мировую точку `f(k, p)`. `closed` — сечения
/// образуют кольцо (последнее соединяется с первым, крышек нет).
///
/// Ориентация: внешний контур CCW, отверстия CW; движение вдоль нормали
/// плоскости профиля даёт нормали наружу. Для зеркального движения сетка
/// разворачивается целиком в `finish_solid`.
pub fn sweep_regions(regions: &[Region], steps: usize, closed: bool, f: impl Fn(usize, Vec2) -> Vec3) -> Option<Mesh> {
    if regions.is_empty() || steps < 2 {
        return None;
    }
    let mut mesh = Mesh::new();
    for region in regions {
        if region.outer.len() < 3 {
            continue;
        }
        let mut contours: Vec<Vec<Vec2>> = Vec::new();
        let mut outer = region.outer.clone();
        if signed_area(&outer) < 0.0 {
            outer.reverse();
        }
        contours.push(outer);
        for h in &region.holes {
            if h.len() < 3 {
                continue;
            }
            let mut h = h.clone();
            if signed_area(&h) > 0.0 {
                h.reverse();
            }
            contours.push(h);
        }
        // Боковые поверхности.
        for c in &contours {
            let n = c.len();
            let base = mesh.positions.len() as u32;
            for k in 0..steps {
                for p in c {
                    mesh.add_vertex(f(k, *p));
                }
            }
            let spans = if closed { steps } else { steps - 1 };
            for k in 0..spans {
                let k1 = (k + 1) % steps;
                for i in 0..n {
                    let j = (i + 1) % n;
                    let b0 = base + (k * n + i) as u32;
                    let b1 = base + (k * n + j) as u32;
                    let t0 = base + (k1 * n + i) as u32;
                    let t1 = base + (k1 * n + j) as u32;
                    mesh.add_triangle(b0, b1, t1);
                    mesh.add_triangle(b0, t1, t0);
                }
            }
        }
        // Крышки.
        if !closed {
            let Some(tri) = triangulate_region(region) else { continue };
            let bottom = mesh.positions.len() as u32;
            for p in &tri.verts {
                mesh.add_vertex(f(0, *p));
            }
            let top = mesh.positions.len() as u32;
            for p in &tri.verts {
                mesh.add_vertex(f(steps - 1, *p));
            }
            for t in &tri.tris {
                mesh.add_triangle(bottom + t[0], bottom + t[2], bottom + t[1]);
                mesh.add_triangle(top + t[0], top + t[1], top + t[2]);
            }
        }
    }
    finish_solid(mesh)
}

/// Сварка, нормали и ориентация наружу. `None`, если сетка пуста.
fn finish_solid(mut mesh: Mesh) -> Option<Mesh> {
    if mesh.indices.is_empty() {
        return None;
    }
    mesh.weld();
    if mesh.indices.is_empty() {
        return None;
    }
    mesh.compute_smooth_normals();
    mesh.ensure_outward();
    Some(mesh)
}

// ---------------------------------------------------------------------------
// Revolve
// ---------------------------------------------------------------------------

/// Ось вращения в координатах эскиза.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis2 {
    pub origin: Vec2,
    pub dir: Vec2,
}

impl Axis2 {
    pub const X: Axis2 = Axis2 { origin: Vec2 { x: 0.0, y: 0.0 }, dir: Vec2 { x: 1.0, y: 0.0 } };
    pub const Y: Axis2 = Axis2 { origin: Vec2 { x: 0.0, y: 0.0 }, dir: Vec2 { x: 0.0, y: 1.0 } };
}

/// Ось вращения эскиза: 0 — ось X эскиза, 1 — ось Y, 2 — самый длинный
/// отдельный отрезок эскиза (не входящий в замкнутый контур).
pub fn sketch_axis(sk: &Sketch, kind: usize) -> Option<Axis2> {
    match kind {
        0 => Some(Axis2::X),
        1 => Some(Axis2::Y),
        _ => {
            let mut best: Option<(f32, Axis2)> = None;
            for (pts, closed) in sk.chains() {
                if closed || pts.len() < 2 {
                    continue;
                }
                let (a, b) = (pts[0], *pts.last().unwrap());
                let l = a.distance(b);
                // Только прямые цепочки (все точки на отрезке a–b).
                let d = (b - a) * (1.0 / l.max(1e-9));
                let straight = pts.iter().all(|p| ((*p - a).x * d.y - (*p - a).y * d.x).abs() < 1e-3 * l.max(1.0));
                if l > 1e-6 && straight && best.map_or(true, |(bl, _)| l > bl) {
                    best = Some((l, Axis2 { origin: a, dir: d }));
                }
            }
            best.map(|(_, a)| a)
        }
    }
}

/// Вращение профиля эскиза вокруг оси на `angle_deg` градусов.
/// Меш — в координатах эскиза (плоскость учитывается, трансформация тела — нет).
pub fn revolve(sk: &Sketch, axis: Axis2, angle_deg: f32) -> Result<Mesh, String> {
    let regions = sk.regions();
    if regions.is_empty() {
        return Err("в эскизе нет замкнутого контура".into());
    }
    let angle = angle_deg.clamp(-360.0, 360.0);
    if angle.abs() < 0.01 {
        return Err("угол должен быть больше нуля".into());
    }
    let d = axis.dir.normalize();
    let perp = Vec2::new(-d.y, d.x);
    // Профиль должен лежать по одну сторону от оси.
    let tol = 1e-4;
    let (mut has_pos, mut has_neg) = (false, false);
    for r in &regions {
        for p in r.outer.iter().chain(r.holes.iter().flatten()) {
            let rr = (*p - axis.origin).dot(perp);
            has_pos |= rr > tol;
            has_neg |= rr < -tol;
        }
    }
    if has_pos && has_neg {
        return Err("профиль пересекает ось вращения".into());
    }
    let (o, u, v, n) = sk.plane.frame();
    let a3 = (u * d.x + v * d.y).normalize();
    let r3 = n.cross(a3);
    let o3 = o + u * axis.origin.x + v * axis.origin.y;
    let full = angle.abs() >= 359.99;
    let total = angle * DEG2RAD;
    let segs = ((angle.abs() / 360.0) * 96.0).ceil().max(4.0) as usize;
    let steps = if full { segs } else { segs + 1 };
    let f = |k: usize, p: Vec2| {
        let th = if full { TAU * k as f32 / steps as f32 } else { total * k as f32 / (steps - 1) as f32 };
        let q = p - axis.origin;
        let (aa, rr) = (q.dot(d), q.dot(perp));
        o3 + a3 * aa + (r3 * th.cos() + n * th.sin()) * rr
    };
    sweep_regions(&regions, steps, full, f).ok_or_else(|| "не удалось построить тело".to_string())
}

// ---------------------------------------------------------------------------
// Sweep
// ---------------------------------------------------------------------------

fn rotate_about(v: Vec3, axis: Vec3, ang: f32) -> Vec3 {
    let (s, c) = ang.sin_cos();
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

/// Касательные полилинии.
fn tangents(path: &[Vec3], closed: bool) -> Vec<Vec3> {
    let n = path.len();
    (0..n)
        .map(|i| {
            let prev = if i > 0 { path[i - 1] } else if closed { path[n - 1] } else { path[i] };
            let next = if i + 1 < n { path[i + 1] } else if closed { path[0] } else { path[i] };
            let a = (path[i] - prev).normalize();
            let b = (next - path[i]).normalize();
            let t = a + b;
            if t.len2() < 1e-12 {
                if b.len2() > 0.0 {
                    b
                } else {
                    a
                }
            } else {
                t.normalize()
            }
        })
        .collect()
}

/// Рамки с минимальным вращением (метод двойного отражения, Wang 2008).
/// Возвращает нормали `r_i` (перпендикулярны касательным).
fn rmf(path: &[Vec3], t: &[Vec3], r0: Vec3, closed: bool) -> Vec<Vec3> {
    let n = path.len();
    let step = |xi: Vec3, xj: Vec3, ti: Vec3, tj: Vec3, ri: Vec3| -> Vec3 {
        let v1 = xj - xi;
        let c1 = v1.dot(v1);
        if c1 < 1e-18 {
            return ri;
        }
        let rl = ri - v1 * (2.0 / c1 * v1.dot(ri));
        let tl = ti - v1 * (2.0 / c1 * v1.dot(ti));
        let v2 = tj - tl;
        let c2 = v2.dot(v2);
        let r = if c2 < 1e-18 { rl } else { rl - v2 * (2.0 / c2 * v2.dot(rl)) };
        (r - tj * r.dot(tj)).normalize()
    };
    let mut r = vec![r0; n];
    for i in 0..n - 1 {
        r[i + 1] = step(path[i], path[i + 1], t[i], t[i + 1], r[i]);
    }
    if closed && n > 2 {
        // Компенсация набега угла на замкнутой траектории.
        let rn = step(path[n - 1], path[0], t[n - 1], t[0], r[n - 1]);
        let s0 = t[0].cross(r[0]);
        let phi = rn.dot(s0).atan2(rn.dot(r[0]));
        for (i, ri) in r.iter_mut().enumerate() {
            *ri = rotate_about(*ri, t[i], -phi * i as f32 / n as f32);
        }
    }
    r
}

/// Сдвиг профиля эскиза (с трансформацией `profile_tf`) вдоль траектории
/// `path` (мировые точки). Профиль переносится вдоль пути, сохраняя
/// положение относительно начала траектории. `twist_deg` — закрутка на
/// всей длине, `end_scale` — масштаб профиля в конце. Меш — в мировых координатах.
pub fn sweep(profile: &Sketch, profile_tf: Mat4, path: &[Vec3], closed: bool, twist_deg: f32, end_scale: f32) -> Result<Mesh, String> {
    let regions = profile.regions();
    if regions.is_empty() {
        return Err("в эскизе профиля нет замкнутого контура".into());
    }
    let mut pts: Vec<Vec3> = Vec::with_capacity(path.len());
    for p in path {
        if pts.last().map_or(true, |q: &Vec3| q.distance(*p) > 1e-5) {
            pts.push(*p);
        }
    }
    if closed && pts.len() > 2 && pts[0].distance(*pts.last().unwrap()) < 1e-5 {
        pts.pop();
    }
    if pts.len() < 2 {
        return Err("траектория слишком короткая".into());
    }
    let (_, pu, _, pn) = profile.plane.frame();
    let pn = profile_tf.transform_vector(pn).normalize();
    let pu = profile_tf.transform_vector(pu).normalize();
    let t = tangents(&pts, closed);
    if t[0].dot(pn).abs() < 0.2 {
        return Err("плоскость профиля должна пересекать траекторию поперёк".into());
    }
    let mut r0 = pu - t[0] * pu.dot(t[0]);
    if r0.len2() < 1e-8 {
        r0 = t[0].cross(Vec3::new(0.0, 0.0, 1.0));
        if r0.len2() < 1e-8 {
            r0 = t[0].cross(Vec3::new(1.0, 0.0, 0.0));
        }
    }
    let r = rmf(&pts, &t, r0.normalize(), closed);
    let s: Vec<Vec3> = t.iter().zip(&r).map(|(a, b)| a.cross(*b)).collect();
    // Доля длины пути для закрутки и масштаба.
    let mut cum = vec![0.0f32; pts.len()];
    for i in 1..pts.len() {
        cum[i] = cum[i - 1] + pts[i].distance(pts[i - 1]);
    }
    let total = if closed { cum[pts.len() - 1] + pts[0].distance(*pts.last().unwrap()) } else { cum[pts.len() - 1] };
    let total = total.max(1e-9);
    let p0 = pts[0];
    let twist = twist_deg * DEG2RAD;
    let f = |k: usize, p: Vec2| {
        let x = profile_tf.transform_point(profile.plane.to_world(p)) - p0;
        let (ct, cr, cs) = (x.dot(t[0]), x.dot(r[0]), x.dot(s[0]));
        let frac = cum[k] / total;
        let sc = 1.0 + (end_scale - 1.0) * frac;
        let (sa, ca) = (twist * frac).sin_cos();
        let (cr2, cs2) = (cr * ca - cs * sa, cr * sa + cs * ca);
        pts[k] + t[k] * (ct * sc) + r[k] * (cr2 * sc) + s[k] * (cs2 * sc)
    };
    sweep_regions(&regions, pts.len(), closed, f).ok_or_else(|| "не удалось построить тело".to_string())
}

// ---------------------------------------------------------------------------
// Loft
// ---------------------------------------------------------------------------

fn resample_closed(poly: &[Vec3], n: usize) -> Vec<Vec3> {
    let m = poly.len();
    let mut cum = vec![0.0f32; m + 1];
    for i in 0..m {
        cum[i + 1] = cum[i] + poly[i].distance(poly[(i + 1) % m]);
    }
    let total = cum[m].max(1e-9);
    let mut out = Vec::with_capacity(n);
    let mut seg = 0;
    for k in 0..n {
        let d = total * k as f32 / n as f32;
        while seg + 1 < m && cum[seg + 1] < d {
            seg += 1;
        }
        let l = (cum[seg + 1] - cum[seg]).max(1e-12);
        let tt = ((d - cum[seg]) / l).clamp(0.0, 1.0);
        out.push(poly[seg].lerp(poly[(seg + 1) % m], tt));
    }
    out
}

fn newell(poly: &[Vec3]) -> Vec3 {
    let mut n = Vec3::zero();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

fn centroid(poly: &[Vec3]) -> Vec3 {
    let mut c = Vec3::zero();
    for p in poly {
        c += *p;
    }
    c * (1.0 / poly.len().max(1) as f32)
}

/// Лофт через замкнутые сечения (мировые полилинии), по порядку.
pub fn loft(sections: &[Vec<Vec3>]) -> Result<Mesh, String> {
    if sections.len() < 2 {
        return Err("нужно минимум два сечения".into());
    }
    if sections.iter().any(|s| s.len() < 3) {
        return Err("каждое сечение должно быть замкнутым контуром".into());
    }
    // Точек на сечение: если у всех одинаково (например, прямоугольники) —
    // сохраняем вершины, иначе равномерная выборка по длине.
    let same = sections.iter().all(|s| s.len() == sections[0].len());
    let n = if same { sections[0].len() } else { 128 };
    let mut secs: Vec<Vec<Vec3>> = sections.iter().map(|s| if same { s.clone() } else { resample_closed(s, n) }).collect();
    let cents: Vec<Vec3> = secs.iter().map(|s| centroid(s)).collect();
    for i in 0..secs.len() {
        let dir = if i + 1 < secs.len() { cents[i + 1] - cents[i] } else { cents[i] - cents[i - 1] };
        if dir.len2() < 1e-10 {
            return Err("сечения лежат в одном месте — разнесите их по высоте".into());
        }
        if newell(&secs[i]).dot(dir) < 0.0 {
            secs[i].reverse();
        }
    }
    // Выравнивание начальных точек по предыдущему сечению.
    for i in 1..secs.len() {
        let (ca, cb) = (cents[i - 1], cents[i]);
        let mut best = (f32::MAX, 0usize);
        for k in 0..n {
            let mut e = 0.0;
            for j in 0..n {
                e += ((secs[i - 1][j] - ca) - (secs[i][(j + k) % n] - cb)).len2();
            }
            if e < best.0 {
                best = (e, k);
            }
        }
        secs[i].rotate_left(best.1);
    }
    let mut mesh = Mesh::new();
    let ring = |m: &mut Mesh, s: &Vec<Vec3>| -> u32 {
        let base = m.positions.len() as u32;
        for p in s {
            m.add_vertex(*p);
        }
        base
    };
    let bases: Vec<u32> = secs.iter().map(|s| ring(&mut mesh, s)).collect();
    for w in 0..secs.len() - 1 {
        let (b, tp) = (bases[w], bases[w + 1]);
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            mesh.add_triangle(b + i, b + j, tp + j);
            mesh.add_triangle(b + i, tp + j, tp + i);
        }
    }
    // Крышки — триангуляция в плоскости каждого крайнего сечения.
    for (si, flip) in [(0usize, true), (secs.len() - 1, false)] {
        let s = &secs[si];
        let nn = newell(s).normalize();
        let u = if nn.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
        let u = (u - nn * u.dot(nn)).normalize();
        let v = nn.cross(u);
        let p2: Vec<Vec2> = s.iter().map(|p| Vec2::new(p.dot(u), p.dot(v))).collect();
        let tris = ear_clip(&p2);
        let base = bases[si];
        for t in tris.chunks_exact(3) {
            let (a, b, c) = (base + t[0] as u32, base + t[1] as u32, base + t[2] as u32);
            if flip {
                mesh.add_triangle(a, c, b);
            } else {
                mesh.add_triangle(a, b, c);
            }
        }
    }
    finish_solid(mesh).ok_or_else(|| "не удалось построить тело".to_string())
}

/// Внешний контур сечения эскиза в мировых координатах (самый большой по площади).
pub fn sketch_section(sk: &Sketch, tf: Mat4) -> Option<Vec<Vec3>> {
    let regions = sk.regions();
    let r = regions.iter().max_by(|a, b| signed_area(&a.outer).abs().partial_cmp(&signed_area(&b.outer).abs()).unwrap_or(std::cmp::Ordering::Equal))?;
    let mut outer = r.outer.clone();
    if signed_area(&outer) < 0.0 {
        outer.reverse();
    }
    Some(outer.iter().map(|p| tf.transform_point(sk.plane.to_world(*p))).collect())
}

// ---------------------------------------------------------------------------
// Shell / Solidify
// ---------------------------------------------------------------------------

/// Придание толщины (Solidify, Blender) / оболочка (Shell, Fusion 360).
/// Замкнутое тело становится полым со стенкой `thickness`; у открытой
/// поверхности появляется толщина и торцы по краям. Положительная
/// толщина — внутрь (против нормалей), отрицательная — наружу.
pub fn solidify(src: &Mesh, thickness: f32) -> Result<Mesh, String> {
    if thickness.abs() < 1e-6 {
        return Err("толщина должна быть больше нуля".into());
    }
    let mut m = src.clone();
    m.colors = None;
    m.weld();
    if m.indices.is_empty() {
        return Err("пустая сетка".into());
    }
    let nv = m.positions.len();
    let ntri = m.triangle_count();
    // Нормаль вершины — сумма нормалей граней с весом по углу.
    let mut vn = vec![Vec3::zero(); nv];
    let mut fnorm = Vec::with_capacity(ntri);
    for i in 0..ntri {
        let fnm = m.face_normal(i).normalize();
        fnorm.push(fnm);
        let idx = m.tri_indices(i);
        for k in 0..3 {
            let (a, b, c) = (m.positions[idx[k] as usize], m.positions[idx[(k + 1) % 3] as usize], m.positions[idx[(k + 2) % 3] as usize]);
            let e1 = (b - a).normalize();
            let e2 = (c - a).normalize();
            let ang = e1.dot(e2).clamp(-1.0, 1.0).acos();
            vn[idx[k] as usize] += fnm * ang;
        }
    }
    for n in &mut vn {
        *n = n.normalize();
    }
    // Равномерная толщина: делим на минимальный косинус с нормалями граней.
    let mut mindot = vec![1.0f32; nv];
    for i in 0..ntri {
        for &vi in &m.tri_indices(i) {
            let d = vn[vi as usize].dot(fnorm[i]);
            if d < mindot[vi as usize] {
                mindot[vi as usize] = d;
            }
        }
    }
    let mut out = Mesh::new();
    for p in &m.positions {
        out.add_vertex(*p);
    }
    for (i, p) in m.positions.iter().enumerate() {
        let k = 1.0 / mindot[i].max(0.35);
        out.add_vertex(*p - vn[i] * (thickness * k));
    }
    let off = nv as u32;
    for t in m.indices.chunks_exact(3) {
        out.add_triangle(t[0], t[1], t[2]);
        out.add_triangle(off + t[0], off + t[2], off + t[1]);
    }
    // Торцы по граничным рёбрам.
    let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
    for t in m.indices.chunks_exact(3) {
        for k in 0..3 {
            *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
        }
    }
    let closed = m.is_closed();
    for (&(a, b), _) in edges.iter() {
        if !edges.contains_key(&(b, a)) {
            out.add_triangle(b, a, off + a);
            out.add_triangle(b, off + a, off + b);
        }
    }
    if thickness < 0.0 {
        out.flip_winding();
    }
    out.compute_smooth_normals();
    if closed || out.is_closed() {
        out.ensure_outward();
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Подразделение и сглаживание
// ---------------------------------------------------------------------------

/// Один уровень подразделения: каждый треугольник → 4.
/// `smooth = true` — схема Loop (сглаживание, как Subdivision Surface),
/// `false` — простое деление по серединам (форма не меняется).
pub fn subdivide(src: &Mesh, smooth: bool) -> Mesh {
    let mut m = src.clone();
    m.colors = None;
    m.weld();
    let nv = m.positions.len();
    // Ребро → противолежащие вершины.
    let mut opp: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for t in m.indices.chunks_exact(3) {
        for k in 0..3 {
            let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            opp.entry(key).or_default().push(c);
        }
    }
    // Соседи и граничные соседи вершин.
    let mut nbr: Vec<Vec<u32>> = vec![Vec::new(); nv];
    let mut bnd: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (&(a, b), o) in &opp {
        nbr[a as usize].push(b);
        nbr[b as usize].push(a);
        if o.len() != 2 {
            bnd[a as usize].push(b);
            bnd[b as usize].push(a);
        }
    }
    let p = &m.positions;
    let mut out = Mesh::new();
    for (i, v) in p.iter().enumerate() {
        let np = if !smooth {
            *v
        } else if !bnd[i].is_empty() {
            if bnd[i].len() == 2 {
                *v * 0.75 + (p[bnd[i][0] as usize] + p[bnd[i][1] as usize]) * 0.125
            } else {
                *v
            }
        } else {
            let n = nbr[i].len();
            if n < 3 {
                *v
            } else {
                let beta = if n == 3 { 3.0 / 16.0 } else { 3.0 / (8.0 * n as f32) };
                let mut s = Vec3::zero();
                for &j in &nbr[i] {
                    s += p[j as usize];
                }
                *v * (1.0 - n as f32 * beta) + s * beta
            }
        };
        out.add_vertex(np);
    }
    let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
    for (&(a, b), o) in &opp {
        let (pa, pb) = (p[a as usize], p[b as usize]);
        let np = if smooth && o.len() == 2 { (pa + pb) * 0.375 + (p[o[0] as usize] + p[o[1] as usize]) * 0.125 } else { (pa + pb) * 0.5 };
        mid.insert((a, b), out.add_vertex(np));
    }
    let e = |a: u32, b: u32| -> u32 { mid[&if a < b { (a, b) } else { (b, a) }] };
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (t[0], t[1], t[2]);
        let (ab, bc, ca) = (e(a, b), e(b, c), e(c, a));
        out.add_triangle(a, ab, ca);
        out.add_triangle(ab, b, bc);
        out.add_triangle(ca, bc, c);
        out.add_triangle(ab, bc, ca);
    }
    out.compute_smooth_normals();
    out
}

/// Сглаживание вершин: Laplacian (`taubin = false`) или Taubin
/// (без усадки). Граничные вершины не двигаются.
pub fn smooth(src: &Mesh, iterations: usize, factor: f32, taubin: bool) -> Mesh {
    let mut m = src.clone();
    m.colors = None;
    m.weld();
    let nv = m.positions.len();
    let mut cnt: HashMap<(u32, u32), u32> = HashMap::new();
    for t in m.indices.chunks_exact(3) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *cnt.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0) += 1;
        }
    }
    let mut nbr: Vec<Vec<u32>> = vec![Vec::new(); nv];
    let mut fixed = vec![false; nv];
    for (&(a, b), &c) in &cnt {
        nbr[a as usize].push(b);
        nbr[b as usize].push(a);
        if c != 2 {
            fixed[a as usize] = true;
            fixed[b as usize] = true;
        }
    }
    let lambda = factor.clamp(0.0, 1.0);
    let mu = -(lambda + 0.03).min(1.0);
    let pass = |pos: &Vec<Vec3>, w: f32| -> Vec<Vec3> {
        (0..nv)
            .map(|i| {
                if fixed[i] || nbr[i].is_empty() {
                    return pos[i];
                }
                let mut c = Vec3::zero();
                for &j in &nbr[i] {
                    c += pos[j as usize];
                }
                let c = c * (1.0 / nbr[i].len() as f32);
                pos[i] + (c - pos[i]) * w
            })
            .collect()
    };
    let mut pos = m.positions.clone();
    for _ in 0..iterations {
        pos = pass(&pos, lambda);
        if taubin {
            pos = pass(&pos, mu);
        }
    }
    m.positions = pos;
    m.compute_smooth_normals();
    m
}

/// Уплотнение сетки простым делением, пока самое длинное ребро больше
/// `max_edge` (и треугольников не больше `max_tris`). Нужно деформациям.
pub fn refine(src: &Mesh, max_edge: f32, max_tris: usize) -> Mesh {
    let mut m = src.clone();
    for _ in 0..8 {
        let mut longest = 0.0f32;
        for i in 0..m.triangle_count() {
            let [a, b, c] = m.tri(i);
            longest = longest.max(a.distance(b)).max(b.distance(c)).max(c.distance(a));
        }
        if longest <= max_edge || m.triangle_count() * 4 > max_tris {
            break;
        }
        m = subdivide(&m, false);
    }
    m
}

// ---------------------------------------------------------------------------
// Деформации (Simple Deform)
// ---------------------------------------------------------------------------

/// Вид деформации.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeformKind {
    /// Скручивание вокруг оси на угол (градусы).
    Twist,
    /// Сужение: масштаб в конце (1 = без изменений).
    Taper,
    /// Изгиб на угол (градусы).
    Bend,
}

/// Деформация точек сетки вдоль оси `axis` (0 — X, 1 — Y, 2 — Z) в
/// пределах её габаритов.
pub fn deform(src: &Mesh, kind: DeformKind, axis: usize, amount: f32) -> Mesh {
    let mut m = src.clone();
    let bb = m.aabb();
    if bb.is_empty() {
        return m;
    }
    let axis = axis.min(2);
    let (i1, i2) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let c = bb.center();
    let lo = bb.min.get(axis);
    let len = (bb.max.get(axis) - lo).max(1e-6);
    for p in &mut m.positions {
        let h = p.get(axis) - lo;
        let tt = h / len;
        let (x, y) = (p.get(i1) - c.get(i1), p.get(i2) - c.get(i2));
        match kind {
            DeformKind::Twist => {
                let (s, co) = (amount * DEG2RAD * tt).sin_cos();
                p.set(i1, c.get(i1) + x * co - y * s);
                p.set(i2, c.get(i2) + x * s + y * co);
            }
            DeformKind::Taper => {
                let k = 1.0 + (amount - 1.0) * tt;
                p.set(i1, c.get(i1) + x * k);
                p.set(i2, c.get(i2) + y * k);
            }
            DeformKind::Bend => {
                let phi = amount * DEG2RAD;
                if phi.abs() < 1e-5 {
                    continue;
                }
                let r = len / phi;
                let th = h / r;
                let (s, co) = th.sin_cos();
                p.set(i1, c.get(i1) + r - (r - x) * co);
                p.set(axis, lo + (r - x) * s);
            }
        }
    }
    m.compute_smooth_normals();
    m
}

// ---------------------------------------------------------------------------
// Пружина
// ---------------------------------------------------------------------------

/// Пружина (винтовая линия с круглым сечением): радиус витка, шаг,
/// число витков, радиус проволоки. Ось — Z, начало на плоскости XY.
pub fn coil(radius: f32, pitch: f32, turns: f32, wire: f32) -> Result<Mesh, String> {
    if radius <= 0.0 || wire <= 0.0 || turns <= 0.0 {
        return Err("параметры должны быть больше нуля".into());
    }
    if wire >= radius {
        return Err("радиус проволоки должен быть меньше радиуса витка".into());
    }
    if pitch.abs() < wire * 2.0 {
        return Err("шаг меньше толщины проволоки — витки пересекаются".into());
    }
    let seg_per_turn = 64usize;
    let steps = ((turns * seg_per_turn as f32).ceil() as usize).max(2) + 1;
    let total = turns * TAU;
    let n = 20;
    let circle: Vec<Vec2> = (0..n).map(|i| {
        let a = TAU * i as f32 / n as f32;
        Vec2::new(a.cos() * wire, a.sin() * wire)
    }).collect();
    let regions = [Region { outer: circle, holes: Vec::new() }];
    let k = pitch / TAU;
    let f = |s: usize, p: Vec2| {
        let th = total * s as f32 / (steps - 1) as f32;
        let (sn, cs) = th.sin_cos();
        let pos = Vec3::new(radius * cs, radius * sn, k * th + wire);
        // Френе для винтовой линии: касательная, нормаль к оси, бинормаль.
        let tang = Vec3::new(-radius * sn, radius * cs, k).normalize();
        let nrm = Vec3::new(-cs, -sn, 0.0);
        let bin = tang.cross(nrm);
        pos + nrm * p.x + bin * p.y
    };
    sweep_regions(&regions, steps, false, f).ok_or_else(|| "не удалось построить пружину".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;
    use crate::sketch::{SketchEntity, SketchPlane};

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> SketchEntity {
        SketchEntity::Rect { min: Vec3::new(x0, y0, 0.0), max: Vec3::new(x1, y1, 0.0) }
    }

    #[test]
    fn revolve_rect_makes_tube() {
        // Прямоугольник r∈[5,10], h∈[0,20] вокруг оси Y → труба.
        let mut sk = Sketch::new(SketchPlane::Xz);
        sk.entities.push(rect(5.0, 0.0, 10.0, 20.0));
        let m = revolve(&sk, Axis2::Y, 360.0).unwrap();
        assert!(m.is_closed());
        let expect = PI * (100.0 - 25.0) * 20.0;
        assert!((m.volume() - expect).abs() / expect < 0.01, "vol={} expect={expect}", m.volume());
        // Половина оборота.
        let h = revolve(&sk, Axis2::Y, 180.0).unwrap();
        assert!(h.is_closed());
        assert!((h.volume() - expect * 0.5).abs() / expect < 0.01, "vol={}", h.volume());
    }

    #[test]
    fn revolve_touching_axis_and_crossing() {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(rect(0.0, 0.0, 5.0, 10.0));
        let m = revolve(&sk, Axis2::Y, 360.0).unwrap();
        assert!(m.is_closed());
        let expect = PI * 25.0 * 10.0;
        assert!((m.volume() - expect).abs() / expect < 0.01, "vol={}", m.volume());
        let mut bad = Sketch::new(SketchPlane::Xy);
        bad.entities.push(rect(-2.0, 0.0, 5.0, 10.0));
        assert!(revolve(&bad, Axis2::Y, 360.0).is_err());
    }

    #[test]
    fn revolve_around_sketch_line() {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(rect(10.0, 0.0, 12.0, 4.0));
        sk.entities.push(SketchEntity::Line { a: Vec3::new(20.0, -5.0, 0.0), b: Vec3::new(20.0, 5.0, 0.0) });
        let ax = sketch_axis(&sk, 2).unwrap();
        assert!((ax.origin.x - 20.0).abs() < 1e-4);
        let m = revolve(&sk, ax, 360.0).unwrap();
        // Кольцо: r от 8 до 10, высота 4.
        let expect = PI * (100.0 - 64.0) * 4.0;
        assert!((m.volume() - expect).abs() / expect < 0.01, "vol={}", m.volume());
    }

    #[test]
    fn sweep_straight_equals_extrude() {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Circle { center: Vec3::zero(), radius: 3.0 });
        let path = vec![Vec3::zero(), Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, 25.0)];
        let m = sweep(&sk, Mat4::identity(), &path, false, 0.0, 1.0).unwrap();
        assert!(m.is_closed());
        let expect = sk.extrude(25.0).unwrap().volume();
        assert!((m.volume() - expect).abs() / expect < 0.01, "vol={} expect={expect}", m.volume());
        // Параллельная траектория — ошибка.
        let flat = vec![Vec3::zero(), Vec3::new(10.0, 0.0, 0.0)];
        assert!(sweep(&sk, Mat4::identity(), &flat, false, 0.0, 1.0).is_err());
    }

    #[test]
    fn sweep_closed_ring_is_torus() {
        // Окружность r=2 в плоскости XZ с центром (10,0,0), путь — окружность R=10 в XY.
        let mut sk = Sketch::new(SketchPlane::Xz);
        sk.entities.push(SketchEntity::Circle { center: Vec3::new(10.0, 0.0, 0.0), radius: 2.0 });
        let path: Vec<Vec3> = (0..96).map(|i| {
            let a = TAU * i as f32 / 96.0;
            Vec3::new(10.0 * a.cos(), 10.0 * a.sin(), 0.0)
        }).collect();
        let m = sweep(&sk, Mat4::identity(), &path, true, 0.0, 1.0).unwrap();
        assert!(m.is_closed());
        let expect = 2.0 * PI * PI * 10.0 * 4.0;
        assert!((m.volume() - expect).abs() / expect < 0.02, "vol={} expect={expect}", m.volume());
    }

    #[test]
    fn loft_square_to_square_is_frustum() {
        let a: Vec<Vec3> = vec![Vec3::new(-5.0, -5.0, 0.0), Vec3::new(5.0, -5.0, 0.0), Vec3::new(5.0, 5.0, 0.0), Vec3::new(-5.0, 5.0, 0.0)];
        let b: Vec<Vec3> = vec![Vec3::new(-2.0, -2.0, 10.0), Vec3::new(-2.0, 2.0, 10.0), Vec3::new(2.0, 2.0, 10.0), Vec3::new(2.0, -2.0, 10.0)];
        let m = loft(&[a, b]).unwrap();
        assert!(m.is_closed());
        // Объём усечённой пирамиды: h/3 (A1 + A2 + sqrt(A1 A2)).
        let expect = 10.0 / 3.0 * (100.0 + 16.0 + 40.0);
        assert!((m.volume() - expect).abs() / expect < 0.02, "vol={} expect={expect}", m.volume());
    }

    #[test]
    fn loft_circle_to_rect() {
        let mut s1 = Sketch::new(SketchPlane::Xy);
        s1.entities.push(SketchEntity::Circle { center: Vec3::zero(), radius: 5.0 });
        let mut s2 = Sketch::new(SketchPlane::Custom { origin: Vec3::new(0.0, 0.0, 20.0), normal: Vec3::new(0.0, 0.0, 1.0) });
        s2.entities.push(rect(-3.0, -3.0, 3.0, 3.0));
        let m = loft(&[sketch_section(&s1, Mat4::identity()).unwrap(), sketch_section(&s2, Mat4::identity()).unwrap()]).unwrap();
        assert!(m.is_closed());
        assert!(m.volume() > 0.0);
        let bb = m.aabb();
        assert!((bb.max.z - 20.0).abs() < 1e-3 && bb.min.z.abs() < 1e-3);
    }

    #[test]
    fn shell_box_is_hollow() {
        let b = primitive_box(20.0, 20.0, 20.0);
        let s = solidify(&b, 2.0).unwrap();
        assert!(s.is_closed());
        let expect = 8000.0 - 16.0f32.powi(3);
        assert!((s.volume() - expect).abs() / expect < 0.01, "vol={} expect={expect}", s.volume());
        // Наружу: стенка добавляется снаружи.
        let o = solidify(&b, -2.0).unwrap();
        let expect_o = 24.0f32.powi(3) - 8000.0;
        assert!((o.volume() - expect_o).abs() / expect_o < 0.01, "vol={} expect={expect_o}", o.volume());
    }

    #[test]
    fn solidify_open_plane_closes_it() {
        let mut m = Mesh::new();
        let a = m.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = m.add_vertex(Vec3::new(10.0, 0.0, 0.0));
        let c = m.add_vertex(Vec3::new(10.0, 10.0, 0.0));
        let d = m.add_vertex(Vec3::new(0.0, 10.0, 0.0));
        m.add_triangle(a, b, c);
        m.add_triangle(a, c, d);
        let s = solidify(&m, 1.0).unwrap();
        assert!(s.is_closed());
        assert!((s.volume() - 100.0).abs() < 0.1, "vol={}", s.volume());
    }

    #[test]
    fn subdivide_counts_and_shape() {
        let b = primitive_box(10.0, 10.0, 10.0);
        let mut w = b.clone();
        w.weld();
        let simple = subdivide(&b, false);
        assert_eq!(simple.triangle_count(), w.triangle_count() * 4);
        assert!(simple.is_closed());
        assert!((simple.volume() - 1000.0).abs() < 0.1);
        let loop1 = subdivide(&b, true);
        assert!(loop1.is_closed());
        assert!(loop1.volume() < 1000.0 && loop1.volume() > 200.0, "vol={}", loop1.volume());
    }

    #[test]
    fn smooth_taubin_shrinks_less() {
        let s = crate::mesh::primitive_sphere(10.0, 24, 12);
        let v0 = s.volume();
        let lap = smooth(&s, 10, 0.5, false);
        let tau = smooth(&s, 10, 0.5, true);
        assert!(lap.volume() < v0);
        assert!((tau.volume() - v0).abs() < (lap.volume() - v0).abs(), "taubin={} lap={} v0={v0}", tau.volume(), lap.volume());
    }

    #[test]
    fn deform_twist_taper_bend() {
        let b = refine(&primitive_box(10.0, 10.0, 40.0), 2.5, 200_000);
        assert!(b.triangle_count() > 12);
        let v = b.volume();
        let tw = deform(&b, DeformKind::Twist, 2, 90.0);
        assert!((tw.volume() - v).abs() / v < 0.03, "twist vol={}", tw.volume());
        let tp = deform(&b, DeformKind::Taper, 2, 0.5);
        let bb = tp.aabb();
        assert!(bb.size().x < 10.01 && tp.volume() < v);
        let bd = deform(&b, DeformKind::Bend, 2, 90.0);
        let bb = bd.aabb();
        assert!(bb.size().x > 15.0, "bend size={:?}", bb.size());
    }

    #[test]
    fn coil_is_closed() {
        let m = coil(10.0, 5.0, 3.0, 1.0).unwrap();
        assert!(m.is_closed());
        let len = 3.0 * ((TAU * 10.0).powi(2) + 25.0).sqrt();
        let expect = PI * 1.0 * len;
        assert!((m.volume() - expect).abs() / expect < 0.03, "vol={} expect={expect}", m.volume());
        assert!(coil(10.0, 1.0, 3.0, 1.0).is_err());
    }
}
