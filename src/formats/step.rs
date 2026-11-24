//! STEP AP214/AP203: чтение с тесселяцией в треугольный меш, запись AP214.
//!
//! Чтение: парсинг сущностей Part 21, извлечение MANIFOLD_SOLID_BREP →
//! CLOSED_SHELL → ADVANCED_FACE, тесселяция кривых (линии, окружности,
//! эллипсы, B-сплайны) и поверхностей (плоскости, цилиндры, конусы, сферы,
//! торы, B-сплайн поверхности). Триангуляция — ear clipping с отверстиями,
//! для кривых поверхностей — подразделение с проекцией на поверхность.
//!
//! Запись: каждый треугольник меша становится ADVANCED_FACE с PLANE —
//! валидный AP214, который импортируется большинством CAD-систем.

use crate::math::*;
use crate::mesh::Mesh;
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Значение STEP
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Str(String),
    Enum(String),
    Ref(u32),
    List(Vec<Value>),
    Null,
    Omitted,
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_ref(&self) -> Option<u32> {
        match self {
            Value::Ref(r) => Some(*r),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_enum(&self) -> Option<&str> {
        match self {
            Value::Enum(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub id: u32,
    pub ty: String,
    pub args: Vec<Value>,
}

// ---------------------------------------------------------------------------
// Токенизатор / парсер Part 21
// ---------------------------------------------------------------------------

struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ref(u32),
    Str(String),
    Number(f64),
    Enum(String),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Eq,
    Semi,
    Dollar,
    Star,
    Eof,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Self { bytes: s.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.bytes.len() && (self.bytes[self.pos] as char).is_whitespace() {
                self.pos += 1;
            }
            // Комментарий /* ... */
            if self.pos + 1 < self.bytes.len()
                && self.bytes[self.pos] == b'/'
                && self.bytes[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.bytes.len()
                    && !(self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/')
                {
                    self.pos += 1;
                }
                self.pos = (self.pos + 2).min(self.bytes.len());
            } else {
                break;
            }
        }
    }

    fn next_tok(&mut self) -> Tok {
        self.skip_ws();
        if self.pos >= self.bytes.len() {
            return Tok::Eof;
        }
        let c = self.bytes[self.pos] as char;
        match c {
            '(' => {
                self.pos += 1;
                Tok::LParen
            }
            ')' => {
                self.pos += 1;
                Tok::RParen
            }
            ',' => {
                self.pos += 1;
                Tok::Comma
            }
            '=' => {
                self.pos += 1;
                Tok::Eq
            }
            ';' => {
                self.pos += 1;
                Tok::Semi
            }
            '$' => {
                self.pos += 1;
                Tok::Dollar
            }
            '*' => {
                self.pos += 1;
                Tok::Star
            }
            '#' => {
                self.pos += 1;
                let start = self.pos;
                while self.pos < self.bytes.len() && (self.bytes[self.pos] as char).is_ascii_digit() {
                    self.pos += 1;
                }
                let s = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("0");
                Tok::Ref(s.parse().unwrap_or(0))
            }
            '\'' => {
                self.pos += 1;
                let mut s = String::new();
                while self.pos < self.bytes.len() {
                    if self.bytes[self.pos] == b'\'' {
                        // '' — экранированная кавычка.
                        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'\'' {
                            s.push('\'');
                            self.pos += 2;
                        } else {
                            self.pos += 1;
                            break;
                        }
                    } else {
                        s.push(self.bytes[self.pos] as char);
                        self.pos += 1;
                    }
                }
                Tok::Str(s)
            }
            '.' => {
                // Перечисление .NAME. или число .5
                let start = self.pos;
                self.pos += 1;
                while self.pos < self.bytes.len() {
                    let ch = self.bytes[self.pos] as char;
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if self.pos < self.bytes.len() && self.bytes[self.pos] == b'.' {
                    self.pos += 1;
                    let s = std::str::from_utf8(&self.bytes[start + 1..self.pos - 1]).unwrap_or("");
                    Tok::Enum(s.to_string())
                } else {
                    // Число вида .5
                    self.pos = start;
                    self.read_number()
                }
            }
            '-' | '+' | '0'..='9' => self.read_number(),
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = self.pos;
                while self.pos < self.bytes.len() {
                    let ch = self.bytes[self.pos] as char;
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Tok::Ident(std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("").to_string())
            }
            _ => {
                self.pos += 1;
                Tok::Ident(String::new())
            }
        }
    }

    fn read_number(&mut self) -> Tok {
        let start = self.pos;
        if self.pos < self.bytes.len() && (self.bytes[self.pos] == b'-' || self.bytes[self.pos] == b'+') {
            self.pos += 1;
        }
        while self.pos < self.bytes.len() {
            let ch = self.bytes[self.pos] as char;
            if ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == 'E' || ch == '+' || ch == '-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("0");
        Tok::Number(s.parse().unwrap_or(0.0))
    }
}

/// Распарсить весь файл в карту сущностей.
pub fn parse_entities(text: &str) -> Result<HashMap<u32, Entity>, String> {
    let mut lx = Lexer::new(text);
    let mut map: HashMap<u32, Entity> = HashMap::new();
    loop {
        let tok = lx.next_tok();
        match tok {
            Tok::Eof => break,
            Tok::Ref(id) => {
                // Ожидаем '=' TYPE '(' args ')' ';'
                if lx.next_tok() != Tok::Eq {
                    continue;
                }
                let ty = match lx.next_tok() {
                    Tok::Ident(s) => s,
                    _ => continue,
                };
                if lx.next_tok() != Tok::LParen {
                    continue;
                }
                let args = parse_args(&mut lx)?;
                // ';'
                let _ = lx.next_tok();
                map.insert(id, Entity { id, ty, args });
            }
            _ => continue,
        }
    }
    if map.is_empty() {
        return Err("В STEP-файле не найдено сущностей".into());
    }
    Ok(map)
}

fn parse_args(lx: &mut Lexer) -> Result<Vec<Value>, String> {
    let mut args = Vec::new();
    loop {
        let tok = lx.next_tok();
        match tok {
            Tok::RParen => break,
            Tok::Comma => continue,
            Tok::Eof => break,
            Tok::LParen => {
                // Вложенный список: собираем до matching ')'.
                // parse_args рекурсивно съест свой ')'.
                let inner = parse_args(lx)?;
                args.push(Value::List(inner));
            }
            Tok::Ref(r) => args.push(Value::Ref(r)),
            Tok::Str(s) => args.push(Value::Str(s)),
            Tok::Number(n) => args.push(Value::Number(n)),
            Tok::Enum(s) => args.push(Value::Enum(s)),
            Tok::Ident(s) => {
                // Голый идентификатор (редко) — как строка.
                if !s.is_empty() {
                    args.push(Value::Str(s));
                }
            }
            Tok::Dollar => args.push(Value::Null),
            Tok::Star => args.push(Value::Omitted),
            Tok::Eq | Tok::Semi => {}
        }
    }
    Ok(args)
}

// ---------------------------------------------------------------------------
// Доступ к сущностям
// ---------------------------------------------------------------------------

pub struct StepDb {
    pub entities: HashMap<u32, Entity>,
}

impl StepDb {
    pub fn get(&self, id: u32) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn point(&self, id: u32) -> Option<Vec3> {
        let e = self.get(id)?;
        if e.ty != "CARTESIAN_POINT" {
            return None;
        }
        let coords = e.args.get(1)?.as_list()?;
        if coords.len() < 3 {
            return None;
        }
        Some(Vec3::new(
            coords[0].as_f64()? as f32,
            coords[1].as_f64()? as f32,
            coords[2].as_f64()? as f32,
        ))
    }

    pub fn direction(&self, id: u32) -> Option<Vec3> {
        let e = self.get(id)?;
        if e.ty != "DIRECTION" {
            return None;
        }
        let coords = e.args.get(1)?.as_list()?;
        if coords.len() < 3 {
            return None;
        }
        Some(Vec3::new(
            coords[0].as_f64()? as f32,
            coords[1].as_f64()? as f32,
            coords[2].as_f64()? as f32,
        ))
    }

    /// AXIS2_PLACEMENT_3D → (origin, z_axis, x_axis).
    pub fn axis2_placement(&self, id: u32) -> Option<(Vec3, Vec3, Vec3)> {
        let e = self.get(id)?;
        if e.ty != "AXIS2_PLACEMENT_3D" {
            return None;
        }
        let origin = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| self.point(r)).unwrap_or(Vec3::zero());
        let z = e.args.get(2).and_then(|v| v.as_ref()).and_then(|r| self.direction(r)).unwrap_or(Vec3::new(0.0, 0.0, 1.0));
        let x = e.args.get(3).and_then(|v| v.as_ref()).and_then(|r| self.direction(r)).unwrap_or(Vec3::new(1.0, 0.0, 0.0));
        let z = z.normalize();
        // Ортогонализуем x относительно z.
        let x = (x - z * x.dot(z)).normalize();
        if x.len2() < 1e-10 {
            let x = Vec3::new(1.0, 0.0, 0.0);
            let x = (x - z * x.dot(z)).normalize();
            return Some((origin, z, x));
        }
        Some((origin, z, x))
    }

    /// VERTEX_POINT → позиция.
    pub fn vertex_point(&self, id: u32) -> Option<Vec3> {
        let e = self.get(id)?;
        if e.ty != "VERTEX_POINT" {
            return None;
        }
        e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| self.point(r))
    }

    /// Все сущности заданного типа.
    pub fn of_type(&self, ty: &str) -> Vec<&Entity> {
        self.entities.values().filter(|e| e.ty == ty).collect()
    }
}

// ---------------------------------------------------------------------------
// Кривые: тесселяция в полилинию
// ---------------------------------------------------------------------------

/// Тесселяция кривой (по id сущности кривой) в точки.
/// `start`/`end` — концы ребра (для ориентации и обрезки).
fn tessellate_curve(db: &StepDb, curve_id: u32, start: Vec3, end: Vec3) -> Vec<Vec3> {
    let Some(e) = db.get(curve_id) else {
        return vec![start, end];
    };
    match e.ty.as_str() {
        "LINE" => {
            // LINE('', #point, #vector) — прямая бесконечная; используем концы ребра.
            vec![start, end]
        }
        "CIRCLE" => {
            let (origin, z, x) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            let radius = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let y = z.cross(x).normalize();
            // Углы концов.
            let a0 = angle_on_circle(start - origin, x, y);
            let a1 = angle_on_circle(end - origin, x, y);
            let mut sweep = a1 - a0;
            while sweep <= 0.0 {
                sweep += TAU;
            }
            if sweep >= TAU - 1e-4 {
                sweep = TAU;
            }
            let n = circle_segments(radius, sweep);
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let a = a0 + sweep * (i as f32 / n as f32);
                pts.push(origin + x * (a.cos() * radius) + y * (a.sin() * radius));
            }
            pts
        }
        "ELLIPSE" => {
            let (origin, z, x) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            let major = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let minor = e.args.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let y = z.cross(x).normalize();
            let a0 = angle_on_circle(start - origin, x, y);
            let a1 = angle_on_circle(end - origin, x, y);
            let mut sweep = a1 - a0;
            while sweep <= 0.0 {
                sweep += TAU;
            }
            let n = circle_segments(major.max(minor), sweep);
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let a = a0 + sweep * (i as f32 / n as f32);
                pts.push(origin + x * (a.cos() * major) + y * (a.sin() * minor));
            }
            pts
        }
        "TRIMMED_CURVE" => {
            // TRIMMED_CURVE('', #basis, trim1, trim2, sense, master)
            let basis = e.args.get(1).and_then(|v| v.as_ref()).unwrap_or(0);
            let mut pts = tessellate_curve(db, basis, start, end);
            // sense: .T. — trim1→trim2 по направлению, .F. — наоборот.
            let sense = e.args.get(4).and_then(|v| v.as_enum()).unwrap_or("T");
            if sense == "F" {
                pts.reverse();
            }
            pts
        }
        "B_SPLINE_CURVE_WITH_KNOTS" | "B_SPLINE_CURVE" => {
            tessellate_bspline_curve(db, e, start, end)
        }
        "PARABOLA" | "HYPERBOLA" => {
            // Редкие: аппроксимируем прямой между концами.
            vec![start, end]
        }
        _ => vec![start, end],
    }
}

fn angle_on_circle(v: Vec3, x: Vec3, y: Vec3) -> f32 {
    v.dot(x).atan2(v.dot(y))
}

fn circle_segments(radius: f32, sweep: f32) -> usize {
    // ~2 градуса на сегмент, минимум 4, максимум 256.
    let n = (sweep.abs() / (2.0 * DEG2RAD)).ceil() as usize;
    let _ = radius;
    n.clamp(4, 256)
}

fn tessellate_bspline_curve(db: &StepDb, e: &Entity, start: Vec3, end: Vec3) -> Vec<Vec3> {
    // B_SPLINE_CURVE_WITH_KNOTS('', (#points), degree, knots, form, closed, self_intersect)
    let pts_refs = e.args.get(1).and_then(|v| v.as_list()).unwrap_or(&[]);
    let mut ctrl: Vec<Vec3> = Vec::new();
    for r in pts_refs {
        if let Some(id) = r.as_ref() {
            if let Some(p) = db.point(id) {
                ctrl.push(p);
            }
        }
    }
    if ctrl.len() < 2 {
        return vec![start, end];
    }
    let degree = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as usize;
    let knots: Vec<f64> = e
        .args
        .get(2)
        .and_then(|v| v.as_list())
        .map(|l| l.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let closed = e.args.get(4).and_then(|v| v.as_enum()).unwrap_or("F") == "T";
    let n = ctrl.len();
    let nseg = (n * 8).clamp(8, 256);
    let mut out = Vec::with_capacity(nseg + 1);
    for i in 0..=nseg {
        let t = i as f64 / nseg as f64;
        let u = if knots.len() >= 2 {
            knots[0] + t * (knots[knots.len() - 1] - knots[0])
        } else {
            t
        };
        out.push(bspline_point(&ctrl, degree, &knots, u, closed));
    }
    // Привязываем концы к вершинам ребра.
    if !out.is_empty() {
        out[0] = start;
        let last = out.len() - 1;
        out[last] = end;
    }
    out
}

/// Точка B-сплайна (алгоритм де Бура).
fn bspline_point(ctrl: &[Vec3], degree: usize, knots: &[f64], u: f64, closed: bool) -> Vec3 {
    let n = ctrl.len();
    if n == 0 {
        return Vec3::zero();
    }
    if n == 1 {
        return ctrl[0];
    }
    let k = degree.min(n - 1);
    // Узловой вектор по умолчанию (равномерный).
    let default_knots: Vec<f64> = (0..(n + k + 1)).map(|i| i as f64).collect();
    let kv = if knots.len() == n + k + 1 && !knots.is_empty() {
        knots
    } else {
        &default_knots
    };
    // Найти интервал.
    let mut span = k;
    for i in k..n {
        if u < kv[i + 1] || i == n - 1 {
            span = i;
            break;
        }
    }
    // Де Бур.
    let mut d: Vec<Vec3> = ctrl.to_vec();
    for j in 1..=k {
        for i in (span - k + j)..=span {
            if i >= n {
                break;
            }
            let denom1 = kv[i + k - j + 1] - kv[i + 1];
            let denom2 = kv[i + k - j] - kv[i];
            let a1 = if denom1.abs() > 1e-12 {
                (u - kv[i + 1]) / denom1
            } else {
                0.0
            };
            let a2 = if denom2.abs() > 1e-12 {
                (kv[i + k] - u) / denom2
            } else {
                0.0
            };
            d[i] = d[i - 1] * a1 as f32 + d[i] * a2 as f32;
        }
    }
    let _ = closed;
    d[span]
}

// ---------------------------------------------------------------------------
// Поверхности
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Surface {
    Plane { origin: Vec3, normal: Vec3, x_axis: Vec3 },
    Cylinder { origin: Vec3, axis: Vec3, radius: f32 },
    Cone { origin: Vec3, axis: Vec3, radius: f32, semi_angle: f32 },
    Sphere { center: Vec3, radius: f32 },
    Torus { origin: Vec3, axis: Vec3, major: f32, minor: f32 },
    Bspline,
    Unknown,
}

impl Surface {
    /// Проекция точки на поверхность (для подразделения).
    pub fn project(&self, p: Vec3) -> Vec3 {
        match self {
            Surface::Plane { origin, normal, .. } => p - *normal * (p - *origin).dot(*normal),
            Surface::Cylinder { origin, axis, radius } => {
                let d = p - *origin;
                let h = d.dot(*axis);
                let radial = d - *axis * h;
                let l = radial.len();
                if l < 1e-9 {
                    return p;
                }
                *origin + *axis * h + radial * (*radius / l)
            }
            Surface::Cone { origin, axis, radius, semi_angle } => {
                let d = p - *origin;
                let h = d.dot(*axis);
                let radial = d - *axis * h;
                let l = radial.len();
                if l < 1e-9 {
                    return p;
                }
                let r_at_h = radius + h * semi_angle.tan();
                *origin + *axis * h + radial * (r_at_h.max(0.0) / l)
            }
            Surface::Sphere { center, radius } => {
                let d = p - *center;
                let l = d.len();
                if l < 1e-9 {
                    return p;
                }
                *center + d * (*radius / l)
            }
            Surface::Torus { origin, axis, major, minor } => {
                let d = p - *origin;
                let h = d.dot(*axis);
                let radial = d - *axis * h;
                let l = radial.len();
                if l < 1e-9 {
                    return p;
                }
                let radial_dir = radial / l;
                let ring_center = *origin + *axis * h + radial_dir * *major;
                let off = p - ring_center;
                let ol = off.len();
                if ol < 1e-9 {
                    return p;
                }
                ring_center + off * (*minor / ol)
            }
            _ => p,
        }
    }

    pub fn is_planar(&self) -> bool {
        matches!(self, Surface::Plane { .. })
    }
}

fn parse_surface(db: &StepDb, id: u32) -> Surface {
    let Some(e) = db.get(id) else {
        return Surface::Unknown;
    };
    match e.ty.as_str() {
        "PLANE" => {
            let (origin, z, x) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            Surface::Plane { origin, normal: z, x_axis: x }
        }
        "CYLINDRICAL_SURFACE" => {
            let (origin, z, _) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            let radius = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            Surface::Cylinder { origin, axis: z, radius }
        }
        "CONICAL_SURFACE" => {
            let (origin, z, _) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            let radius = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let angle = e.args.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            Surface::Cone { origin, axis: z, radius, semi_angle: angle }
        }
        "SPHERICAL_SURFACE" => {
            let (origin, _, _) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            let radius = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            Surface::Sphere { center: origin, radius }
        }
        "TOROIDAL_SURFACE" => {
            let (origin, z, _) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.axis2_placement(r)).unwrap_or((Vec3::zero(), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)));
            let major = e.args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let minor = e.args.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            Surface::Torus { origin, axis: z, major, minor }
        }
        "B_SPLINE_SURFACE_WITH_KNOTS" | "B_SPLINE_SURFACE" => Surface::Bspline,
        _ => Surface::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Триангуляция грани (ear clipping с отверстиями)
// ---------------------------------------------------------------------------

/// Площадь со знаком полигона (2D).
fn signed_area(poly: &[Vec2]) -> f32 {
    let mut s = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        s += a.x * b.y - b.x * a.y;
    }
    s * 0.5
}

fn point_in_tri(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_neg = d1 < -1e-12 || d2 < -1e-12 || d3 < -1e-12;
    let has_pos = d1 > 1e-12 || d2 > 1e-12 || d3 > 1e-12;
    !(has_neg && has_pos)
}

fn sign(p1: Vec2, p2: Vec2, p3: Vec2) -> f32 {
    (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
}

/// Ear clipping полигона (CCW, без самопересечений). Возвращает индексы.
fn ear_clip(poly: &[Vec2]) -> Vec<usize> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    // Обеспечиваем CCW.
    if signed_area(poly) < 0.0 {
        idx.reverse();
    }
    let mut tris = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n * 2 {
        guard += 1;
        let mut clipped = false;
        for i in 0..idx.len() {
            let i0 = idx[(i + idx.len() - 1) % idx.len()];
            let i1 = idx[i];
            let i2 = idx[(i + 1) % idx.len()];
            let (a, b, c) = (poly[i0], poly[i1], poly[i2]);
            // Ухо: выпуклая вершина и нет других вершин внутри.
            if sign(a, b, c) <= 1e-12 {
                continue;
            }
            let mut blocked = false;
            for &j in &idx {
                if j == i0 || j == i1 || j == i2 {
                    continue;
                }
                // Пропускаем совпадающие вершины (дубли моста).
                if poly[j].distance(a) < 1e-9 || poly[j].distance(b) < 1e-9 || poly[j].distance(c) < 1e-9 {
                    continue;
                }
                if point_in_tri(poly[j], a, b, c) {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                continue;
            }
            tris.push(i0);
            tris.push(i1);
            tris.push(i2);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Убираем вырожденную (коллинеарную) вершину, если она есть.
            let mut removed = false;
            for i in 0..idx.len() {
                let i0 = idx[(i + idx.len() - 1) % idx.len()];
                let i1 = idx[i];
                let i2 = idx[(i + 1) % idx.len()];
                if sign(poly[i0], poly[i1], poly[i2]).abs() <= 1e-9 {
                    idx.remove(i);
                    removed = true;
                    break;
                }
            }
            if !removed {
                break;
            }
        }
    }
    if idx.len() == 3 {
        tris.extend_from_slice(&idx);
    }
    tris
}

/// Триангуляция грани: внешний контур + отверстия.
/// `outer`/`holes` — 3D полилинии (замкнутые, без повтора первой точки).
/// Возвращает треугольники как индексы в объединённый список вершин.
fn triangulate_face(
    outer: &[Vec3],
    holes: &[Vec<Vec3>],
    surface: &Surface,
    mesh: &mut Mesh,
    flip: bool,
) {
    if outer.len() < 3 {
        return;
    }
    // Базис проекции: для плоскости — её базис, иначе — best-fit plane.
    let (u_axis, v_axis, normal) = match surface {
        Surface::Plane { normal, x_axis, .. } => {
            let u = *x_axis;
            let v = normal.cross(u).normalize();
            (u, v, *normal)
        }
        _ => {
            // Нормаль Ньюэлла по внешнему контуру.
            let mut n = Vec3::zero();
            for i in 0..outer.len() {
                let a = outer[i];
                let b = outer[(i + 1) % outer.len()];
                n += a.cross(b);
            }
            let n = n.normalize();
            let u = if n.abs().x < 0.9 {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            let u = (u - n * u.dot(n)).normalize();
            let v = n.cross(u).normalize();
            (u, v, n)
        }
    };
    let origin = outer[0];
    let to2d = |p: Vec3| -> Vec2 {
        let d = p - origin;
        Vec2::new(d.dot(u_axis), d.dot(v_axis))
    };
    // Собираем полигон: внешний + отверстия, соединённые мостами.
    let mut poly2d: Vec<Vec2> = outer.iter().map(|&p| to2d(p)).collect();
    let mut poly3d: Vec<Vec3> = outer.to_vec();
    // Ориентация: внешний CCW, отверстия CW (в 2D).
    if signed_area(&poly2d) < 0.0 {
        poly2d.reverse();
        poly3d.reverse();
    }
    let mut hole_ranges: Vec<(usize, usize)> = Vec::new(); // (start, len) в poly
    for hole in holes {
        if hole.len() < 3 {
            continue;
        }
        let mut h2: Vec<Vec2> = hole.iter().map(|&p| to2d(p)).collect();
        let mut h3: Vec<Vec3> = hole.to_vec();
        if signed_area(&h2) > 0.0 {
            h2.reverse();
            h3.reverse();
        }
        // Мост: ближайшая пара (внешняя вершина, вершина отверстия).
        let mut best: Option<(usize, usize, f32)> = None;
        for (i, hp) in h2.iter().enumerate() {
            for (j, op) in poly2d.iter().enumerate() {
                let d = (*hp - *op).len2();
                if best.map_or(true, |(_, _, bd)| d < bd) {
                    best = Some((i, j, d));
                }
            }
        }
        let Some((hi, oi, _)) = best else { continue };
        // Вставляем отверстие после oi: [oi, hi, hi+1, ..., hi-1, oi] — мост дублирует вершины.
        let hole_start = oi + 1;
        let mut insert2: Vec<Vec2> = Vec::new();
        let mut insert3: Vec<Vec3> = Vec::new();
        for k in 0..h2.len() {
            let idx = (hi + k) % h2.len();
            insert2.push(h2[idx]);
            insert3.push(h3[idx]);
        }
        insert2.push(h2[hi]);
        insert3.push(h3[hi]);
        insert2.push(poly2d[oi]);
        insert3.push(poly3d[oi]);
        let hole_len = insert2.len();
        for (k, v) in insert2.into_iter().enumerate() {
            poly2d.insert(hole_start + k, v);
        }
        for (k, v) in insert3.into_iter().enumerate() {
            poly3d.insert(hole_start + k, v);
        }
        hole_ranges.push((hole_start, hole_len));
    }
    let _ = hole_ranges;
    let tris = ear_clip(&poly2d);
    if tris.len() < 3 {
        return;
    }
    // Создаём вершины в меше.
    let base = mesh.positions.len() as u32;
    for p in &poly3d {
        mesh.add_vertex(*p);
    }
    // Подразделение для кривых поверхностей.
    let subdivide = !surface.is_planar() && !matches!(surface, Surface::Unknown | Surface::Bspline);
    let mut tri_list: Vec<[u32; 3]> = tris
        .chunks_exact(3)
        .map(|t| [base + t[0] as u32, base + t[1] as u32, base + t[2] as u32])
        .collect();
    if subdivide {
        // 2 уровня подразделения с проекцией.
        for _ in 0..2 {
            let mut next: Vec<[u32; 3]> = Vec::with_capacity(tri_list.len() * 4);
            for t in tri_list {
                let [a, b, c] = t;
                let pa = mesh.positions[a as usize];
                let pb = mesh.positions[b as usize];
                let pc = mesh.positions[c as usize];
                let mab = surface.project((pa + pb) * 0.5);
                let mbc = surface.project((pb + pc) * 0.5);
                let mca = surface.project((pc + pa) * 0.5);
                let iab = mesh.add_vertex(mab);
                let ibc = mesh.add_vertex(mbc);
                let ica = mesh.add_vertex(mca);
                next.push([a, iab, ica]);
                next.push([iab, b, ibc]);
                next.push([ica, ibc, c]);
                next.push([iab, ibc, ica]);
            }
            tri_list = next;
        }
    }
    for t in tri_list {
        if flip {
            mesh.add_triangle(t[0], t[2], t[1]);
        } else {
            mesh.add_triangle(t[0], t[1], t[2]);
        }
    }
    let _ = normal;
}

// ---------------------------------------------------------------------------
// Извлечение граней из BREP
// ---------------------------------------------------------------------------

/// Собрать полилинию EDGE_LOOP (список рёбер).
fn loop_polyline(db: &StepDb, loop_id: u32) -> Vec<Vec3> {
    let Some(e) = db.get(loop_id) else {
        return Vec::new();
    };
    if e.ty == "POLY_LOOP" {
        // POLY_LOOP('', (#points))
        let mut pts = Vec::new();
        if let Some(list) = e.args.get(1).and_then(|v| v.as_list()) {
            for r in list {
                if let Some(id) = r.as_ref() {
                    if let Some(p) = db.point(id) {
                        pts.push(p);
                    }
                }
            }
        }
        return pts;
    }
    if e.ty == "VERTEX_LOOP" {
        if let Some(p) = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.vertex_point(r)) {
            return vec![p];
        }
        return Vec::new();
    }
    // EDGE_LOOP('', (#edges))
    let edges = e.args.get(1).and_then(|v| v.as_list()).unwrap_or(&[]);
    let mut pts: Vec<Vec3> = Vec::new();
    for edge_ref in edges {
        let Some(eid) = edge_ref.as_ref() else { continue };
        pts.extend(edge_polyline(db, eid));
    }
    // Убрать дублирующиеся стыки.
    pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    pts
}

/// Полилиния одного ребра (EDGE_CURVE или ORIENTED_EDGE).
fn edge_polyline(db: &StepDb, edge_id: u32) -> Vec<Vec3> {
    let Some(e) = db.get(edge_id) else {
        return Vec::new();
    };
    match e.ty.as_str() {
        "EDGE_CURVE" => {
            // EDGE_CURVE('', #v1, #v2, #curve, sense)
            let v1 = e.args.get(1).and_then(|v| v.as_ref()).and_then(|r| db.vertex_point(r)).unwrap_or(Vec3::zero());
            let v2 = e.args.get(2).and_then(|v| v.as_ref()).and_then(|r| db.vertex_point(r)).unwrap_or(Vec3::zero());
            let curve = e.args.get(3).and_then(|v| v.as_ref()).unwrap_or(0);
            let sense = e.args.get(4).and_then(|v| v.as_enum()).unwrap_or("T");
            let mut pts = tessellate_curve(db, curve, v1, v2);
            if sense == "F" {
                pts.reverse();
            }
            pts
        }
        "ORIENTED_EDGE" => {
            // ORIENTED_EDGE('', #edge, orientation)
            let inner = e.args.get(1).and_then(|v| v.as_ref()).unwrap_or(0);
            let orient = e.args.get(2).and_then(|v| v.as_enum()).unwrap_or("T");
            let mut pts = edge_polyline(db, inner);
            if orient == "F" {
                pts.reverse();
            }
            pts
        }
        _ => Vec::new(),
    }
}

/// Тесселяция одной ADVANCED_FACE.
fn tessellate_face(db: &StepDb, face: &Entity, mesh: &mut Mesh) {
    // ADVANCED_FACE('', (#bounds), #surface, same_sense)
    let bounds = face.args.get(1).and_then(|v| v.as_list()).unwrap_or(&[]);
    let surface_id = face.args.get(2).and_then(|v| v.as_ref()).unwrap_or(0);
    let surface = parse_surface(db, surface_id);
    let mut outer: Option<Vec<Vec3>> = None;
    let mut holes: Vec<Vec<Vec3>> = Vec::new();
    for b in bounds {
        let Some(bid) = b.as_ref() else { continue };
        let Some(be) = db.get(bid) else { continue };
        let loop_id = be.args.get(1).and_then(|v| v.as_ref()).unwrap_or(0);
        let poly = loop_polyline(db, loop_id);
        if poly.len() < 3 {
            continue;
        }
        if be.ty == "FACE_OUTER_BOUND" {
            outer = Some(poly);
        } else {
            holes.push(poly);
        }
    }
    let Some(outer) = outer else { return };
    // same_sense: .F. — нормаль грани противоположна нормали поверхности.
    let same_sense = face.args.get(3).and_then(|v| v.as_enum()).unwrap_or("T") == "T";
    triangulate_face(&outer, &holes, &surface, mesh, !same_sense);
}

// ---------------------------------------------------------------------------
// Главная загрузка
// ---------------------------------------------------------------------------

pub fn load(path: &Path) -> Result<Mesh, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
    load_from_str(&text, &path.display().to_string())
}

pub fn load_from_str(text: &str, source_name: &str) -> Result<Mesh, String> {
    if !text.contains("ISO-10303-21") && !text.contains("FILE_SCHEMA") {
        return Err("Не STEP-файл (нет заголовка ISO-10303-21)".into());
    }
    let entities = parse_entities(text)?;
    let db = StepDb { entities };
    let mut mesh = Mesh::new();

    // Собираем грани: сначала через MANIFOLD_SOLID_BREP, затем все ADVANCED_FACE.
    let mut faces: Vec<u32> = Vec::new();
    for brep in db.of_type("MANIFOLD_SOLID_BREP") {
        if let Some(shell_id) = brep.args.get(1).and_then(|v| v.as_ref()) {
            if let Some(shell) = db.get(shell_id) {
                if let Some(list) = shell.args.get(1).and_then(|v| v.as_list()) {
                    for f in list {
                        if let Some(fid) = f.as_ref() {
                            faces.push(fid);
                        }
                    }
                }
            }
        }
    }
    if faces.is_empty() {
        // Fallback: все ADVANCED_FACE.
        for f in db.of_type("ADVANCED_FACE") {
            faces.push(f.id);
        }
    }
    if faces.is_empty() {
        // Fallback: FACETED_BREP → POLY_LOOP.
        for brep in db.of_type("FACETED_BREP") {
            if let Some(shell_id) = brep.args.get(1).and_then(|v| v.as_ref()) {
                if let Some(shell) = db.get(shell_id) {
                    if let Some(list) = shell.args.get(1).and_then(|v| v.as_list()) {
                        for f in list {
                            if let Some(fid) = f.as_ref() {
                                if let Some(fe) = db.get(fid) {
                                    if let Some(bounds) = fe.args.get(1).and_then(|v| v.as_list()) {
                                        for b in bounds {
                                            if let Some(bid) = b.as_ref() {
                                                if let Some(be) = db.get(bid) {
                                                    let loop_id = be.args.get(1).and_then(|v| v.as_ref()).unwrap_or(0);
                                                    let poly = loop_polyline(&db, loop_id);
                                                    if poly.len() >= 3 {
                                                        triangulate_face(&poly, &[], &Surface::Unknown, &mut mesh, false);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Тесселяция собранных граней.
    for fid in &faces {
        if let Some(face) = db.get(*fid) {
            tessellate_face(&db, face, &mut mesh);
        }
    }
    if mesh.is_empty() {
        return Err("В STEP-файле не найдено граней для тесселяции".into());
    }
    mesh.weld();
    // Для замкнутых оболочек гарантируем внешнюю ориентацию.
    if mesh.is_closed() && mesh.volume() < 0.0 {
        mesh.flip_winding();
    }
    mesh.name = source_name.to_string();
    Ok(mesh)
}

// ---------------------------------------------------------------------------
// Запись AP214
// ---------------------------------------------------------------------------

pub fn save(mesh: &Mesh, path: &Path) -> Result<(), String> {
    let text = to_step_string(mesh);
    std::fs::write(path, text)
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
}

/// Имя для заголовка STEP: только имя файла, ASCII, без апострофов.
fn step_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let s: String = base.chars().map(|c| if c.is_ascii() && c != '\'' && !c.is_ascii_control() { c } else { '_' }).collect();
    if s.is_empty() {
        "model".into()
    } else {
        s
    }
}

/// Текущее время UTC в формате ISO 8601 (без внешних библиотек).
fn iso_timestamp_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Алгоритм civil_from_days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn to_step_string(mesh: &Mesh) -> String {
    let mut out = String::new();
    out.push_str("ISO-10303-21;\nHEADER;\n");
    out.push_str("FILE_DESCRIPTION(('AlphaCad STEP export'),'2;1');\n");
    out.push_str(&format!(
        "FILE_NAME('{}','{}',('AlphaCad'),(''),'AlphaCad {}','AlphaCad','');\n",
        step_name(&mesh.name),
        iso_timestamp_now(),
        crate::VERSION
    ));
    out.push_str("FILE_SCHEMA(('AP214IS'));\nENDSEC;\nDATA;\n");

    let mut next_id = 1u32;
    let alloc = |next_id: &mut u32| -> u32 {
        let id = *next_id;
        *next_id += 1;
        id
    };

    // DIRECTION Z и X (общие).
    let dir_z = alloc(&mut next_id);
    out.push_str(&format!("#{dir_z} = DIRECTION('',(0.0,0.0,1.0));\n"));
    let dir_x = alloc(&mut next_id);
    out.push_str(&format!("#{dir_x} = DIRECTION('',(1.0,0.0,0.0));\n"));

    let mut face_ids: Vec<u32> = Vec::new();
    let n = mesh.triangle_count();
    for i in 0..n {
        let [a, b, c] = mesh.tri(i);
        let nrm = mesh.face_normal(i).normalize();
        // Точки.
        let pa = alloc(&mut next_id);
        out.push_str(&format!("#{pa} = CARTESIAN_POINT('',({:.6},{:.6},{:.6}));\n", a.x, a.y, a.z));
        let pb = alloc(&mut next_id);
        out.push_str(&format!("#{pb} = CARTESIAN_POINT('',({:.6},{:.6},{:.6}));\n", b.x, b.y, b.z));
        let pc = alloc(&mut next_id);
        out.push_str(&format!("#{pc} = CARTESIAN_POINT('',({:.6},{:.6},{:.6}));\n", c.x, c.y, c.z));
        // Вершины.
        let va = alloc(&mut next_id);
        out.push_str(&format!("#{va} = VERTEX_POINT('',#{pa});\n"));
        let vb = alloc(&mut next_id);
        out.push_str(&format!("#{vb} = VERTEX_POINT('',#{pb});\n"));
        let vc = alloc(&mut next_id);
        out.push_str(&format!("#{vc} = VERTEX_POINT('',#{pc});\n"));
        // Линии: DIRECTION + VECTOR + LINE.
        let mut edge_ids = [0u32; 3];
        let pairs = [(va, vb, a, b), (vb, vc, b, c), (vc, va, c, a)];
        for (k, (v_from, v_to, p_from, p_to)) in pairs.iter().enumerate() {
            let d = *p_to - *p_from;
            let len = d.len().max(1e-9);
            let dn = d / len;
            let dir = alloc(&mut next_id);
            out.push_str(&format!("#{dir} = DIRECTION('',({:.6},{:.6},{:.6}));\n", dn.x, dn.y, dn.z));
            let vec = alloc(&mut next_id);
            out.push_str(&format!("#{vec} = VECTOR('',#{dir},{len:.6});\n"));
            let line = alloc(&mut next_id);
            out.push_str(&format!("#{line} = LINE('',#{pa},#{vec});\n"));
            let _ = v_to;
            let edge = alloc(&mut next_id);
            out.push_str(&format!("#{edge} = EDGE_CURVE('',#{v_from},#{v_to},#{line},.T.);\n"));
            edge_ids[k] = edge;
        }
        // EDGE_LOOP.
        let loop_id = alloc(&mut next_id);
        out.push_str(&format!(
            "#{loop_id} = EDGE_LOOP('',(#{},#{},#{}));\n",
            edge_ids[0], edge_ids[1], edge_ids[2]
        ));
        // FACE_OUTER_BOUND.
        let bound = alloc(&mut next_id);
        out.push_str(&format!("#{bound} = FACE_OUTER_BOUND('',#{loop_id},.T.);\n"));
        // Плоскость: AXIS2_PLACEMENT_3D + PLANE, ось Z = нормаль грани.
        let ndir = alloc(&mut next_id);
        out.push_str(&format!("#{ndir} = DIRECTION('',({:.6},{:.6},{:.6}));\n", nrm.x, nrm.y, nrm.z));
        let xdir = alloc(&mut next_id);
        let xa = if nrm.abs().x < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
        let xp = (xa - nrm * xa.dot(nrm)).normalize();
        out.push_str(&format!("#{xdir} = DIRECTION('',({:.6},{:.6},{:.6}));\n", xp.x, xp.y, xp.z));
        let axis = alloc(&mut next_id);
        out.push_str(&format!("#{axis} = AXIS2_PLACEMENT_3D('',#{pa},#{ndir},#{xdir});\n"));
        let plane = alloc(&mut next_id);
        out.push_str(&format!("#{plane} = PLANE('',#{axis});\n"));
        // ADVANCED_FACE (same_sense=.T. — контур согласован с плоскостью).
        let face = alloc(&mut next_id);
        out.push_str(&format!("#{face} = ADVANCED_FACE('',(#{bound}),#{plane},.T.);\n"));
        face_ids.push(face);
    }

    // CLOSED_SHELL + MANIFOLD_SOLID_BREP.
    let shell = alloc(&mut next_id);
    let faces_str = face_ids.iter().map(|f| format!("#{f}")).collect::<Vec<_>>().join(",");
    out.push_str(&format!("#{shell} = CLOSED_SHELL('',({faces_str}));\n"));
    let brep = alloc(&mut next_id);
    out.push_str(&format!("#{brep} = MANIFOLD_SOLID_BREP('',#{shell});\n"));

    // Минимальная структура продукта.
    let prod = alloc(&mut next_id);
    out.push_str(&format!("#{prod} = PRODUCT('AlphaCad Part','AlphaCad Part','',());\n"));
    let form = alloc(&mut next_id);
    out.push_str(&format!("#{form} = PRODUCT_FORMATION_WITH_SPECIFIED_SOURCE('','',#{prod},.NOT_KNOWN.);\n"));
    let shape_rep = alloc(&mut next_id);
    out.push_str(&format!("#{shape_rep} = SHAPE_REPRESENTATION('',(#{brep}),#{});\n", dir_z));
    let _ = shape_rep;
    let shape_def = alloc(&mut next_id);
    out.push_str(&format!("#{shape_def} = SHAPE_DEFINITION_REPRESENTATION(#{form},#{shape_rep});\n"));
    let prop_def = alloc(&mut next_id);
    out.push_str(&format!("#{prop_def} = PROPERTY_DEFINITION_REPRESENTATION(#{form},#{shape_rep});\n"));
    let _ = prop_def;

    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    const SAMPLE_STEP: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('test','2026-01-01T00:00:00',('Test'),(''),'','','');
FILE_SCHEMA(('AP214IS'));
ENDSEC;
DATA;
#1 = CARTESIAN_POINT('',(0.0,0.0,0.0));
#2 = CARTESIAN_POINT('',(10.0,0.0,0.0));
#3 = CARTESIAN_POINT('',(10.0,10.0,0.0));
#4 = CARTESIAN_POINT('',(0.0,10.0,0.0));
#5 = DIRECTION('',(0.0,0.0,1.0));
#6 = DIRECTION('',(1.0,0.0,0.0));
#7 = AXIS2_PLACEMENT_3D('',#1,#5,#6);
#8 = PLANE('',#7);
#10 = VERTEX_POINT('',#1);
#11 = VERTEX_POINT('',#2);
#12 = VERTEX_POINT('',#3);
#13 = VERTEX_POINT('',#4);
#20 = VECTOR('',#6,10.0);
#21 = LINE('',#1,#20);
#22 = DIRECTION('',(0.0,1.0,0.0));
#23 = VECTOR('',#22,10.0);
#24 = LINE('',#2,#23);
#25 = DIRECTION('',(-1.0,0.0,0.0));
#26 = VECTOR('',#25,10.0);
#27 = LINE('',#3,#26);
#28 = DIRECTION('',(0.0,-1.0,0.0));
#29 = VECTOR('',#28,10.0);
#30 = LINE('',#4,#29);
#40 = EDGE_CURVE('',#10,#11,#21,.T.);
#41 = EDGE_CURVE('',#11,#12,#24,.T.);
#42 = EDGE_CURVE('',#12,#13,#27,.T.);
#43 = EDGE_CURVE('',#13,#10,#30,.T.);
#50 = EDGE_LOOP('',(#40,#41,#42,#43));
#51 = FACE_OUTER_BOUND('',#50,.T.);
#60 = ADVANCED_FACE('',(#51),#8,.T.);
#70 = CLOSED_SHELL('',(#60));
#71 = MANIFOLD_SOLID_BREP('',#70);
ENDSEC;
END-ISO-10303-21;
"#;

    #[test]
    fn parse_sample() {
        let m = load_from_str(SAMPLE_STEP, "test").unwrap();
        assert_eq!(m.triangle_count(), 2);
        let b = m.aabb();
        assert!((b.size().x - 10.0).abs() < 1e-3);
        assert!((b.size().y - 10.0).abs() < 1e-3);
    }

    #[test]
    fn roundtrip() {
        let m = primitive_box(10.0, 10.0, 10.0);
        let text = to_step_string(&m);
        let back = load_from_str(&text, "rt").unwrap();
        assert_eq!(back.triangle_count(), 12);
        assert!((back.volume().abs() - 1000.0).abs() < 1.0, "vol={}", back.volume());
    }

    #[test]
    fn rejects_garbage() {
        assert!(load_from_str("hello", "x").is_err());
    }

    #[test]
    fn lexer_handles_enums_and_refs() {
        let ents = parse_entities(SAMPLE_STEP).unwrap();
        assert!(ents.contains_key(&60));
        let f = &ents[&60];
        assert_eq!(f.ty, "ADVANCED_FACE");
    }
}
