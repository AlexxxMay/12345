//! Треугольные меши: структура, примитивы, измерения, операции.
//!
//! Меш — индексированный треугольный суппорт: вершины + индексы.
//! Храним `f32` позиции и нормали; цвета вершин — опционально (для STEP/OBJ).

use crate::math::*;
use std::collections::HashMap;

/// Треугольный меш.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// Индексы треугольников (по 3 на треугольник). Пусто = non-indexed fallback.
    pub indices: Vec<u32>,
    /// Цвета вершин (опционально, len == positions.len()).
    pub colors: Option<Vec<Color>>,
    pub name: String,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_name(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Self::default() }
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty() || self.indices.is_empty()
    }

    /// Добавить вершину, вернуть её индекс.
    pub fn add_vertex(&mut self, p: Vec3) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push(p);
        self.normals.push(Vec3::zero());
        if let Some(c) = &mut self.colors {
            c.push(Color::white());
        }
        i
    }

    pub fn add_triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.push(a);
        self.indices.push(b);
        self.indices.push(c);
    }

    /// Добавить треугольник из трёх новых вершин.
    pub fn push_tri(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        let ia = self.add_vertex(a);
        let ib = self.add_vertex(b);
        let ic = self.add_vertex(c);
        self.add_triangle(ia, ib, ic);
    }

    pub fn tri(&self, i: usize) -> [Vec3; 3] {
        let b = i * 3;
        [
            self.positions[self.indices[b] as usize],
            self.positions[self.indices[b + 1] as usize],
            self.positions[self.indices[b + 2] as usize],
        ]
    }

    pub fn tri_indices(&self, i: usize) -> [u32; 3] {
        let b = i * 3;
        [self.indices[b], self.indices[b + 1], self.indices[b + 2]]
    }

    /// Нормаль треугольника (не нормализованная = 2*площадь * направление).
    pub fn face_normal(&self, i: usize) -> Vec3 {
        let [a, b, c] = self.tri(i);
        (b - a).cross(c - a)
    }

    /// Пересчитать гладкие вершинные нормали (взвешенно по площади).
    pub fn compute_smooth_normals(&mut self) {
        self.normals = vec![Vec3::zero(); self.positions.len()];
        for i in 0..self.triangle_count() {
            let n = self.face_normal(i);
            let [a, b, c] = self.tri_indices(i);
            self.normals[a as usize] += n;
            self.normals[b as usize] += n;
            self.normals[c as usize] += n;
        }
        for n in &mut self.normals {
            *n = n.normalize();
        }
    }

    /// Плоские нормали: нормаль каждой грани записывается во все её вершины.
    /// Используется для STL (там нормали плоские по определению).
    pub fn compute_flat_normals(&mut self) {
        self.normals = vec![Vec3::zero(); self.positions.len()];
        for i in 0..self.triangle_count() {
            let n = self.face_normal(i).normalize();
            let [a, b, c] = self.tri_indices(i);
            self.normals[a as usize] = n;
            self.normals[b as usize] = n;
            self.normals[c as usize] = n;
        }
    }

    /// Схлопнуть дублирующиеся вершины (с допуском 1e-5 — учитывает
    /// погрешности f32 на швах типа sin(2π)≠0).
    /// Возвращает количество удалённых вершин.
    pub fn weld(&mut self) -> usize {
        let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut remap = vec![0u32; self.positions.len()];
        let mut new_pos: Vec<Vec3> = Vec::with_capacity(self.positions.len());
        let mut new_col: Vec<Color> = Vec::new();
        let has_col = self.colors.is_some();
        for (i, p) in self.positions.iter().enumerate() {
            let key = (
                (p.x * 1e5).round() as i64,
                (p.y * 1e5).round() as i64,
                (p.z * 1e5).round() as i64,
            );
            if let Some(&idx) = map.get(&key) {
                remap[i] = idx;
            } else {
                let idx = new_pos.len() as u32;
                map.insert(key, idx);
                remap[i] = idx;
                new_pos.push(*p);
                if has_col {
                    new_col.push(self.colors.as_ref().unwrap()[i]);
                }
            }
        }
        let removed = self.positions.len() - new_pos.len();
        for idx in &mut self.indices {
            *idx = remap[*idx as usize];
        }
        self.positions = new_pos;
        if has_col {
            self.colors = Some(new_col);
        }
        // Удаляем вырожденные треугольники.
        let mut keep = Vec::with_capacity(self.indices.len());
        for t in self.indices.chunks_exact(3) {
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                keep.extend_from_slice(t);
            }
        }
        self.indices = keep;
        self.compute_smooth_normals();
        removed
    }

    pub fn aabb(&self) -> Aabb {
        let mut b = Aabb::empty();
        for p in &self.positions {
            b.include(*p);
        }
        b
    }

    /// Объём (знаковый, по формуле дивергенции). Положительный для
    /// замкнутых мешей с внешней ориентацией.
    pub fn volume(&self) -> f32 {
        let mut v = 0.0f32;
        for i in 0..self.triangle_count() {
            let [a, b, c] = self.tri(i);
            v += a.dot(b.cross(c)) / 6.0;
        }
        v
    }

    /// Площадь поверхности.
    pub fn surface_area(&self) -> f32 {
        let mut s = 0.0f32;
        for i in 0..self.triangle_count() {
            s += self.face_normal(i).len() * 0.5;
        }
        s
    }

    /// Центр масс (для замкнутого меша).
    pub fn center_of_mass(&self) -> Vec3 {
        let mut acc = Vec3::zero();
        let mut vol = 0.0f32;
        for i in 0..self.triangle_count() {
            let [a, b, c] = self.tri(i);
            let v = a.dot(b.cross(c)) / 6.0;
            vol += v;
            acc += (a + b + c) * (v * 0.25);
        }
        if vol.abs() > 1e-12 {
            acc * (1.0 / vol)
        } else {
            self.aabb().center()
        }
    }

    /// Применить матрицу ко всем вершинам (нормали — через normal matrix).
    pub fn transform(&mut self, m: Mat4) {
        let nm = m.normal_matrix();
        for p in &mut self.positions {
            *p = m.transform_point(*p);
        }
        for n in &mut self.normals {
            *n = nm.transform_vector(*n).normalize();
        }
    }

    /// Перемещение.
    pub fn translate(&mut self, t: Vec3) {
        self.transform(Mat4::translation(t));
    }

    /// Слияние с другим мешем (конкатенация, без сварки).
    pub fn merge(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.indices.extend(other.indices.iter().map(|i| i + base));
        if let (Some(dst), Some(src)) = (&mut self.colors, &other.colors) {
            dst.extend_from_slice(src);
        } else if self.colors.is_none() && other.colors.is_some() {
            let mut c = vec![Color::white(); base as usize];
            c.extend_from_slice(other.colors.as_ref().unwrap());
            self.colors = Some(c);
        }
    }

    /// Развернуть ориентацию всех треугольников (внутрь ↔ наружу).
    pub fn flip_winding(&mut self) {
        for t in self.indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
        self.compute_smooth_normals();
    }

    /// Если меш ориентирован внутрь (объём < 0) — развернуть.
    /// Корректно для замкнутых мешей (все примитивы).
    pub fn ensure_outward(&mut self) {
        if self.volume() < 0.0 {
            self.flip_winding();
        }
    }

    /// Проверка целостности: количество проблем.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.positions.is_empty() {
            issues.push("меш не содержит вершин".into());
        }
        if self.indices.len() % 3 != 0 {
            issues.push("количество индексов не кратно 3".into());
        }
        let n = self.positions.len() as u32;
        for (i, idx) in self.indices.iter().enumerate() {
            if *idx >= n {
                issues.push(format!("индекс {i} вне диапазона ({idx} >= {n})"));
                break;
            }
        }
        for (i, p) in self.positions.iter().enumerate() {
            if !p.is_finite() {
                issues.push(format!("вершина {i} не конечна: {p:?}"));
                break;
            }
        }
        // Вырожденные треугольники.
        let mut degen = 0usize;
        for i in 0..self.triangle_count() {
            if self.face_normal(i).len2() < 1e-20 {
                degen += 1;
            }
        }
        if degen > 0 {
            issues.push(format!("вырожденных треугольников: {degen}"));
        }
        issues
    }

    /// Замкнутость: каждое ребро встречается ровно 2 раза.
    pub fn is_closed(&self) -> bool {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for t in self.indices.chunks_exact(3) {
            for k in 0..3 {
                let a = t[k];
                let b = t[(k + 1) % 3];
                let key = if a < b { (a, b) } else { (b, a) };
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        edges.values().all(|&c| c == 2)
    }

    /// Количество граничных (незамкнутых) рёбер.
    pub fn boundary_edge_count(&self) -> usize {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for t in self.indices.chunks_exact(3) {
            for k in 0..3 {
                let a = t[k];
                let b = t[(k + 1) % 3];
                let key = if a < b { (a, b) } else { (b, a) };
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        edges.values().filter(|&&c| c != 2).count()
    }

    /// Упрощение меша: схлопывание рёбер с наименьшей ошибкой (упрощённая
    /// эвристика по длине ребра). Сохраняет не более `target` треугольников.
    /// Используется для предпросмотра больших моделей.
    pub fn decimate(&mut self, target_tris: usize) {
        if self.triangle_count() <= target_tris || target_tris == 0 {
            return;
        }
        // Простая итеративная схема: пока треугольников больше цели,
        // находим самое короткое ребро и схлопываем его.
        // Для больших мешей это O(n^2) в худшем случае, поэтому ограничиваем
        // число итераций и работаем только если меш не гигантский.
        let max_iters = (self.triangle_count() - target_tris).min(50_000);
        for _ in 0..max_iters {
            if self.triangle_count() <= target_tris {
                break;
            }
            // Найти самое короткое ребро.
            let mut best: Option<(u32, u32, f32)> = None;
            let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
            for t in self.indices.chunks_exact(3) {
                for k in 0..3 {
                    let a = t[k];
                    let b = t[(k + 1) % 3];
                    let key = if a < b { (a, b) } else { (b, a) };
                    if !seen.insert(key) {
                        continue;
                    }
                    let d = self.positions[a as usize].distance(self.positions[b as usize]);
                    if best.map_or(true, |(_, _, bd)| d < bd) {
                        best = Some((a, b, d));
                    }
                }
            }
            let Some((a, b, _)) = best else { break };
            self.collapse_edge(a, b);
        }
        self.weld();
    }

    fn collapse_edge(&mut self, a: u32, b: u32) {
        if a == b {
            return;
        }
        let mid = (self.positions[a as usize] + self.positions[b as usize]) * 0.5;
        self.positions[a as usize] = mid;
        // Все треугольники с b перепривязываем к a; вырожденные удалятся при weld.
        for idx in &mut self.indices {
            if *idx == b {
                *idx = a;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Примитивы
// ---------------------------------------------------------------------------

/// Параметры примитива (для параметрического редактирования в UI).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    Box { w: f32, h: f32, d: f32 },
    Cylinder { radius: f32, height: f32, segments: u32 },
    Sphere { radius: f32, segments: u32, rings: u32 },
    Cone { radius_bottom: f32, radius_top: f32, height: f32, segments: u32 },
    Torus { major_radius: f32, minor_radius: f32, major_segments: u32, minor_segments: u32 },
    Capsule { radius: f32, height: f32, segments: u32 },
    Tube { radius_outer: f32, radius_inner: f32, height: f32, segments: u32 },
    Wedge { w: f32, h: f32, d: f32 },
    Pyramid { base: f32, height: f32, segments: u32 },
}

impl Primitive {
    pub fn build(&self) -> Mesh {
        let mut m = match *self {
            Primitive::Box { w, h, d } => primitive_box(w, h, d),
            Primitive::Cylinder { radius, height, segments } => {
                primitive_cylinder(radius, height, segments.max(3))
            }
            Primitive::Sphere { radius, segments, rings } => {
                primitive_sphere(radius, segments.max(3), rings.max(2))
            }
            Primitive::Cone { radius_bottom, radius_top, height, segments } => {
                primitive_cone(radius_bottom, radius_top, height, segments.max(3))
            }
            Primitive::Torus { major_radius, minor_radius, major_segments, minor_segments } => {
                primitive_torus(major_radius, minor_radius, major_segments.max(3), minor_segments.max(3))
            }
            Primitive::Capsule { radius, height, segments } => {
                primitive_capsule(radius, height, segments.max(3))
            }
            Primitive::Tube { radius_outer, radius_inner, height, segments } => {
                primitive_tube(radius_outer, radius_inner, height, segments.max(3))
            }
            Primitive::Wedge { w, h, d } => primitive_wedge(w, h, d),
            Primitive::Pyramid { base, height, segments } => {
                primitive_pyramid(base, height, segments.max(3))
            }
        };
        m.ensure_outward();
        m
    }

    pub fn default_name(&self) -> &'static str {
        match self {
            Primitive::Box { .. } => "Куб",
            Primitive::Cylinder { .. } => "Цилиндр",
            Primitive::Sphere { .. } => "Сфера",
            Primitive::Cone { .. } => "Конус",
            Primitive::Torus { .. } => "Тор",
            Primitive::Capsule { .. } => "Капсула",
            Primitive::Tube { .. } => "Труба",
            Primitive::Wedge { .. } => "Клин",
            Primitive::Pyramid { .. } => "Пирамида",
        }
    }
}

/// Куб с центром в начале координат.
pub fn primitive_box(w: f32, h: f32, d: f32) -> Mesh {
    let (x, y, z) = (w * 0.5, h * 0.5, d * 0.5);
    let v = [
        Vec3::new(-x, -y, -z), Vec3::new(x, -y, -z), Vec3::new(x, y, -z), Vec3::new(-x, y, -z),
        Vec3::new(-x, -y, z), Vec3::new(x, -y, z), Vec3::new(x, y, z), Vec3::new(-x, y, z),
    ];
    let faces: [[u32; 4]; 6] = [
        [0, 1, 2, 3], // -Z
        [5, 4, 7, 6], // +Z
        [4, 0, 3, 7], // -X
        [1, 5, 6, 2], // +X
        [4, 5, 1, 0], // -Y
        [3, 2, 6, 7], // +Y
    ];
    let mut m = Mesh::new();
    for f in faces {
        let base = m.positions.len() as u32;
        for &i in &f {
            m.add_vertex(v[i as usize]);
        }
        m.add_triangle(base, base + 1, base + 2);
        m.add_triangle(base, base + 2, base + 3);
    }
    m.weld();
    m.ensure_outward();
    m
}

/// Цилиндр вдоль оси Z, центр в начале координат.
pub fn primitive_cylinder(radius: f32, height: f32, segments: u32) -> Mesh {
    primitive_cone(radius, radius, height, segments)
}

/// Усечённый конус вдоль оси Z.
pub fn primitive_cone(rb: f32, rt: f32, height: f32, segments: u32) -> Mesh {
    let mut m = Mesh::new();
    let z0 = -height * 0.5;
    let z1 = height * 0.5;
    let n = segments as usize;
    // Боковая поверхность.
    for i in 0..n {
        let a0 = (i as f32 / n as f32) * TAU;
        let a1 = ((i + 1) as f32 / n as f32) * TAU;
        let p0 = Vec3::new(a0.cos() * rb, a0.sin() * rb, z0);
        let p1 = Vec3::new(a1.cos() * rb, a1.sin() * rb, z0);
        let p2 = Vec3::new(a1.cos() * rt, a1.sin() * rt, z1);
        let p3 = Vec3::new(a0.cos() * rt, a0.sin() * rt, z1);
        let base = m.positions.len() as u32;
        m.add_vertex(p0);
        m.add_vertex(p1);
        m.add_vertex(p2);
        m.add_vertex(p3);
        m.add_triangle(base, base + 1, base + 2);
        m.add_triangle(base, base + 2, base + 3);
    }
    // Нижняя крышка (веер).
    let c0 = m.add_vertex(Vec3::new(0.0, 0.0, z0));
    for i in 0..n {
        let a0 = (i as f32 / n as f32) * TAU;
        let a1 = ((i + 1) as f32 / n as f32) * TAU;
        let p0 = m.add_vertex(Vec3::new(a0.cos() * rb, a0.sin() * rb, z0));
        let p1 = m.add_vertex(Vec3::new(a1.cos() * rb, a1.sin() * rb, z0));
        m.add_triangle(c0, p1, p0);
    }
    // Верхняя крышка.
    let c1 = m.add_vertex(Vec3::new(0.0, 0.0, z1));
    for i in 0..n {
        let a0 = (i as f32 / n as f32) * TAU;
        let a1 = ((i + 1) as f32 / n as f32) * TAU;
        let p0 = m.add_vertex(Vec3::new(a0.cos() * rt, a0.sin() * rt, z1));
        let p1 = m.add_vertex(Vec3::new(a1.cos() * rt, a1.sin() * rt, z1));
        m.add_triangle(c1, p0, p1);
    }
    m.weld();
    m.ensure_outward();
    m
}

/// Сфера с центром в начале координат.
pub fn primitive_sphere(radius: f32, segments: u32, rings: u32) -> Mesh {
    let mut m = Mesh::new();
    let seg = segments as usize;
    let ring = rings as usize;
    // Полюса.
    let top = m.add_vertex(Vec3::new(0.0, 0.0, radius));
    let bottom = m.add_vertex(Vec3::new(0.0, 0.0, -radius));
    let mut ring_start = Vec::with_capacity(ring);
    for r in 0..ring {
        let phi = ((r + 1) as f32 / (ring + 1) as f32) * PI; // 0..PI
        let z = radius * phi.cos();
        let rr = radius * phi.sin();
        let start = m.positions.len() as u32;
        for s in 0..seg {
            let a = (s as f32 / seg as f32) * TAU;
            m.add_vertex(Vec3::new(a.cos() * rr, a.sin() * rr, z));
        }
        ring_start.push(start);
    }
    // Верхний веер.
    for s in 0..seg {
        let a = ring_start[0] + s as u32;
        let b = ring_start[0] + ((s + 1) % seg) as u32;
        m.add_triangle(top, a, b);
    }
    // Кольца.
    for r in 0..ring - 1 {
        for s in 0..seg {
            let s1 = (s + 1) % seg;
            let a = ring_start[r] + s as u32;
            let b = ring_start[r] + s1 as u32;
            let c = ring_start[r + 1] + s1 as u32;
            let d = ring_start[r + 1] + s as u32;
            m.add_triangle(a, b, c);
            m.add_triangle(a, c, d);
        }
    }
    // Нижний веер.
    let last = ring_start[ring - 1];
    for s in 0..seg {
        let a = last + s as u32;
        let b = last + ((s + 1) % seg) as u32;
        m.add_triangle(bottom, b, a);
    }
    m.weld();
    m.ensure_outward();
    m
}

/// Тор в плоскости XY (ось — Z).
pub fn primitive_torus(major: f32, minor: f32, major_seg: u32, minor_seg: u32) -> Mesh {
    let mut m = Mesh::new();
    let (ms, ns) = (major_seg as usize, minor_seg as usize);
    let mut grid: Vec<Vec<u32>> = Vec::with_capacity(ms);
    for i in 0..ms {
        let u = (i as f32 / ms as f32) * TAU;
        let mut row = Vec::with_capacity(ns);
        for j in 0..ns {
            let v = (j as f32 / ns as f32) * TAU;
            let r = major + minor * v.cos();
            let p = Vec3::new(r * u.cos(), r * u.sin(), minor * v.sin());
            row.push(m.add_vertex(p));
        }
        grid.push(row);
    }
    for i in 0..ms {
        for j in 0..ns {
            let i1 = (i + 1) % ms;
            let j1 = (j + 1) % ns;
            let a = grid[i][j];
            let b = grid[i1][j];
            let c = grid[i1][j1];
            let d = grid[i][j1];
            m.add_triangle(a, b, c);
            m.add_triangle(a, c, d);
        }
    }
    m.weld();
    m.ensure_outward();
    m
}

/// Капсула (цилиндр + две полусферы) вдоль оси Z.
pub fn primitive_capsule(radius: f32, height: f32, segments: u32) -> Mesh {
    let mut m = Mesh::new();
    let n = segments as usize;
    let zc = height * 0.5;
    // Цилиндрическая часть.
    for i in 0..n {
        let a0 = (i as f32 / n as f32) * TAU;
        let a1 = ((i + 1) as f32 / n as f32) * TAU;
        let base = m.positions.len() as u32;
        m.add_vertex(Vec3::new(a0.cos() * radius, a0.sin() * radius, -zc));
        m.add_vertex(Vec3::new(a1.cos() * radius, a1.sin() * radius, -zc));
        m.add_vertex(Vec3::new(a1.cos() * radius, a1.sin() * radius, zc));
        m.add_vertex(Vec3::new(a0.cos() * radius, a0.sin() * radius, zc));
        m.add_triangle(base, base + 1, base + 2);
        m.add_triangle(base, base + 2, base + 3);
    }
    // Верхняя полусфера (2 кольца + полюс).
    let rings = 4usize;
    let mut prev: Vec<u32> = (0..n)
        .map(|i| {
            let a = (i as f32 / n as f32) * TAU;
            m.add_vertex(Vec3::new(a.cos() * radius, a.sin() * radius, zc))
        })
        .collect();
    for r in 1..=rings {
        let phi = (r as f32 / rings as f32) * (PI * 0.5);
        let z = zc + radius * phi.sin();
        let rr = radius * phi.cos();
        let cur: Vec<u32> = (0..n)
            .map(|i| {
                let a = (i as f32 / n as f32) * TAU;
                m.add_vertex(Vec3::new(a.cos() * rr, a.sin() * rr, z))
            })
            .collect();
        for i in 0..n {
            let j = (i + 1) % n;
            m.add_triangle(prev[i], cur[j], cur[i]);
            m.add_triangle(prev[i], prev[j], cur[j]);
        }
        prev = cur;
    }
    let top = m.add_vertex(Vec3::new(0.0, 0.0, zc + radius));
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(top, prev[i], prev[j]);
    }
    // Нижняя полусфера.
    let mut prev: Vec<u32> = (0..n)
        .map(|i| {
            let a = (i as f32 / n as f32) * TAU;
            m.add_vertex(Vec3::new(a.cos() * radius, a.sin() * radius, -zc))
        })
        .collect();
    for r in 1..=rings {
        let phi = (r as f32 / rings as f32) * (PI * 0.5);
        let z = -zc - radius * phi.sin();
        let rr = radius * phi.cos();
        let cur: Vec<u32> = (0..n)
            .map(|i| {
                let a = (i as f32 / n as f32) * TAU;
                m.add_vertex(Vec3::new(a.cos() * rr, a.sin() * rr, z))
            })
            .collect();
        for i in 0..n {
            let j = (i + 1) % n;
            m.add_triangle(prev[i], prev[j], cur[j]);
            m.add_triangle(prev[i], cur[j], cur[i]);
        }
        prev = cur;
    }
    let bottom = m.add_vertex(Vec3::new(0.0, 0.0, -zc - radius));
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(bottom, prev[j], prev[i]);
    }
    m.weld();
    m.ensure_outward();
    m
}

/// Труба (полый цилиндр) вдоль оси Z.
pub fn primitive_tube(ro: f32, ri: f32, height: f32, segments: u32) -> Mesh {
    let mut m = Mesh::new();
    let n = segments as usize;
    let z0 = -height * 0.5;
    let z1 = height * 0.5;
    for i in 0..n {
        let a0 = (i as f32 / n as f32) * TAU;
        let a1 = ((i + 1) as f32 / n as f32) * TAU;
        let (c0, s0) = (a0.cos(), a0.sin());
        let (c1, s1) = (a1.cos(), a1.sin());
        // Внешняя стенка (нормаль наружу).
        quad(
            &mut m,
            Vec3::new(c0 * ro, s0 * ro, z0),
            Vec3::new(c1 * ro, s1 * ro, z0),
            Vec3::new(c1 * ro, s1 * ro, z1),
            Vec3::new(c0 * ro, s0 * ro, z1),
        );
        // Внутренняя стенка (нормаль к оси): тот же порядок вершин, обратные треугольники.
        {
            let base = m.positions.len() as u32;
            m.add_vertex(Vec3::new(c0 * ri, s0 * ri, z0));
            m.add_vertex(Vec3::new(c1 * ri, s1 * ri, z0));
            m.add_vertex(Vec3::new(c1 * ri, s1 * ri, z1));
            m.add_vertex(Vec3::new(c0 * ri, s0 * ri, z1));
            m.add_triangle(base, base + 2, base + 1);
            m.add_triangle(base, base + 3, base + 2);
        }
        // Верхний торец (нормаль +Z).
        quad(
            &mut m,
            Vec3::new(c0 * ro, s0 * ro, z1),
            Vec3::new(c1 * ro, s1 * ro, z1),
            Vec3::new(c1 * ri, s1 * ri, z1),
            Vec3::new(c0 * ri, s0 * ri, z1),
        );
        // Нижний торец (нормаль -Z): обратные треугольники.
        {
            let base = m.positions.len() as u32;
            m.add_vertex(Vec3::new(c0 * ro, s0 * ro, z0));
            m.add_vertex(Vec3::new(c1 * ro, s1 * ro, z0));
            m.add_vertex(Vec3::new(c1 * ri, s1 * ri, z0));
            m.add_vertex(Vec3::new(c0 * ri, s0 * ri, z0));
            m.add_triangle(base, base + 2, base + 1);
            m.add_triangle(base, base + 3, base + 2);
        }
    }
    m.weld();
    m.ensure_outward();
    m
}

/// Клин (треугольная призма): основание w×d, высота h, ребро сверху вдоль X.
pub fn primitive_wedge(w: f32, h: f32, d: f32) -> Mesh {
    let (x, y, z) = (w * 0.5, d * 0.5, h * 0.5);
    let p0 = Vec3::new(-x, -y, -z);
    let p1 = Vec3::new(x, -y, -z);
    let p2 = Vec3::new(x, y, -z);
    let p3 = Vec3::new(-x, y, -z);
    let p4 = Vec3::new(-x, y, z);
    let p5 = Vec3::new(x, y, z);
    let mut m = Mesh::new();
    quad(&mut m, p0, p3, p2, p1); // основание (-Z)
    quad(&mut m, p2, p3, p4, p5); // задняя грань (+Y)
    quad(&mut m, p4, p5, p1, p0); // наклонная грань
    tri(&mut m, p0, p4, p3); // торец -X
    tri(&mut m, p1, p2, p5); // торец +X
    m.weld();
    m.ensure_outward();
    m
}

fn quad(m: &mut Mesh, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
    let base = m.positions.len() as u32;
    m.add_vertex(a);
    m.add_vertex(b);
    m.add_vertex(c);
    m.add_vertex(d);
    m.add_triangle(base, base + 1, base + 2);
    m.add_triangle(base, base + 2, base + 3);
}

fn tri(m: &mut Mesh, a: Vec3, b: Vec3, c: Vec3) {
    m.push_tri(a, b, c);
}

/// Пирамида с правильным многоугольным основанием.
pub fn primitive_pyramid(base_size: f32, height: f32, segments: u32) -> Mesh {
    let mut m = Mesh::new();
    let n = segments as usize;
    let r = base_size * 0.5 / (PI / n as f32).cos(); // описанный радиус
    let z0 = -height * 0.5;
    let z1 = height * 0.5;
    let apex = m.add_vertex(Vec3::new(0.0, 0.0, z1));
    let mut ring = Vec::with_capacity(n);
    for i in 0..n {
        let a = (i as f32 / n as f32) * TAU + PI / n as f32;
        ring.push(m.add_vertex(Vec3::new(a.cos() * r, a.sin() * r, z0)));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(apex, ring[j], ring[i]);
    }
    let c = m.add_vertex(Vec3::new(0.0, 0.0, z0));
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(c, ring[i], ring[j]);
    }
    m.weld();
    m.ensure_outward();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_volume() {
        let m = primitive_box(2.0, 3.0, 4.0);
        assert_eq!(m.triangle_count(), 12);
        assert!((m.volume() - 24.0).abs() < 1e-3, "vol={}", m.volume());
        assert!(m.is_closed());
    }

    #[test]
    fn cylinder_volume() {
        let m = primitive_cylinder(1.0, 2.0, 64);
        let expect = PI * 2.0;
        assert!((m.volume() - expect).abs() < 0.05, "vol={} expect={expect}", m.volume());
        assert!(m.is_closed());
    }

    #[test]
    fn sphere_volume() {
        let m = primitive_sphere(1.0, 48, 24);
        let expect = 4.0 / 3.0 * PI;
        assert!((m.volume() - expect).abs() < 0.15, "vol={} expect={expect}", m.volume());
        assert!(m.is_closed());
    }

    #[test]
    fn torus_volume() {
        let m = primitive_torus(2.0, 0.5, 48, 24);
        let expect = TAU * 2.0 * PI * 0.25; // 2*pi^2*R*r^2
        assert!((m.volume() - expect).abs() < 0.3, "vol={} expect={expect}", m.volume());
        assert!(m.is_closed());
    }

    #[test]
    fn tube_volume() {
        let m = primitive_tube(2.0, 1.0, 3.0, 48);
        let expect = PI * (4.0 - 1.0) * 3.0;
        assert!((m.volume() - expect).abs() < 0.2, "vol={} expect={expect}", m.volume());
        assert!(m.is_closed());
    }

    #[test]
    fn wedge_volume() {
        let m = primitive_wedge(2.0, 2.0, 3.0);
        // Треугольная призма: (1/2 * 2 * 2) * 3 = 6
        assert!((m.volume().abs() - 6.0).abs() < 1e-3, "vol={}", m.volume());
    }

    #[test]
    fn weld_removes_dupes() {
        let mut m = Mesh::new();
        m.push_tri(Vec3::zero(), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
        m.push_tri(Vec3::zero(), Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(m.vertex_count(), 6);
        m.weld();
        assert_eq!(m.vertex_count(), 4);
        assert_eq!(m.triangle_count(), 2);
    }

    #[test]
    fn transform_mesh() {
        let mut m = primitive_box(1.0, 1.0, 1.0);
        m.translate(Vec3::new(5.0, 0.0, 0.0));
        let b = m.aabb();
        assert!((b.center().x - 5.0).abs() < 1e-5);
    }
}
