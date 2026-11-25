//! Математика для 3D: векторы, матрицы, кватернионы, AABB, лучи.
//!
//! Всё на `f32` (достаточно для CAD-визуализации и в 2 раза быстрее `f64`
//! на слабых CPU, лучше кэшируется). Матрицы — column-major, как в OpenGL.

use std::ops::{Add, Mul, Neg, Sub};

pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;
pub const FRAC_PI_2: f32 = std::f32::consts::FRAC_PI_2;
pub const DEG2RAD: f32 = PI / 180.0;
pub const RAD2DEG: f32 = 180.0 / PI;

// ---------------------------------------------------------------------------
// Vec2 / Vec3 / Vec4
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn len2(self) -> f32 {
        self.dot(self)
    }
    pub fn normalize(self) -> Self {
        let l = self.len();
        if l > f32::EPSILON {
            self * (1.0 / l)
        } else {
            Self::zero()
        }
    }
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }
    pub fn distance(self, o: Self) -> f32 {
        (self - o).len()
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }
    pub fn one() -> Self {
        Self { x: 1.0, y: 1.0, z: 1.0 }
    }
    pub fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn len2(self) -> f32 {
        self.dot(self)
    }
    pub fn normalize(self) -> Self {
        let l = self.len();
        if l > 1e-12 {
            self * (1.0 / l)
        } else {
            Self::zero()
        }
    }
    pub fn distance(self, o: Self) -> f32 {
        (self - o).len()
    }
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self + (o - self) * t
    }
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }
    pub fn min(self, o: Self) -> Self {
        Self::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max(self, o: Self) -> Self {
        Self::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    /// Компонента по индексу (0=x, 1=y, 2=z).
    pub fn get(self, i: usize) -> f32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
    pub fn set(&mut self, i: usize, v: f32) {
        match i {
            0 => self.x = v,
            1 => self.y = v,
            _ => self.z = v,
        }
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}
impl std::ops::Div<f32> for Vec3 {
    type Output = Self;
    fn div(self, s: f32) -> Self {
        Self::new(self.x / s, self.y / s, self.z / s)
    }
}
impl std::ops::AddAssign for Vec3 {
    fn add_assign(&mut self, o: Self) {
        self.x += o.x;
        self.y += o.y;
        self.z += o.z;
    }
}
impl std::ops::SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Self) {
        self.x -= o.x;
        self.y -= o.y;
        self.z -= o.z;
    }
}
impl std::ops::MulAssign<f32> for Vec3 {
    fn mul_assign(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
        self.z *= s;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
    pub fn xyz(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
    pub fn perspective_divide(self) -> Vec3 {
        if self.w.abs() > 1e-12 {
            Vec3::new(self.x / self.w, self.y / self.w, self.z / self.w)
        } else {
            Vec3::new(self.x, self.y, self.z)
        }
    }
}

// ---------------------------------------------------------------------------
// Mat4 (column-major)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    /// Column-major: m[col*4 + row].
    pub m: [f32; 16],
}

impl Mat4 {
    pub fn identity() -> Self {
        Self {
            m: [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn zero() -> Self {
        Self { m: [0.0; 16] }
    }

    pub fn from_cols(c0: Vec4, c1: Vec4, c2: Vec4, c3: Vec4) -> Self {
        Self {
            m: [
                c0.x, c0.y, c0.z, c0.w, //
                c1.x, c1.y, c1.z, c1.w, //
                c2.x, c2.y, c2.z, c2.w, //
                c3.x, c3.y, c3.z, c3.w,
            ],
        }
    }

    pub fn get(&self, col: usize, row: usize) -> f32 {
        self.m[col * 4 + row]
    }

    pub fn set(&mut self, col: usize, row: usize, v: f32) {
        self.m[col * 4 + row] = v;
    }

    pub fn translation(t: Vec3) -> Self {
        let mut r = Self::identity();
        r.m[12] = t.x;
        r.m[13] = t.y;
        r.m[14] = t.z;
        r
    }

    pub fn scale(s: Vec3) -> Self {
        let mut r = Self::zero();
        r.m[0] = s.x;
        r.m[5] = s.y;
        r.m[10] = s.z;
        r.m[15] = 1.0;
        r
    }

    pub fn scale_uniform(s: f32) -> Self {
        Self::scale(Vec3::splat(s))
    }

    /// Поворот вокруг оси (ось нормализуется внутри).
    pub fn rotation_axis(axis: Vec3, angle_rad: f32) -> Self {
        let a = axis.normalize();
        let (x, y, z) = (a.x, a.y, a.z);
        let c = angle_rad.cos();
        let s = angle_rad.sin();
        let t = 1.0 - c;
        // Column-major rotation matrix (right-handed).
        Self {
            m: [
                t * x * x + c,     t * x * y + s * z, t * x * z - s * y, 0.0, //
                t * x * y - s * z, t * y * y + c,     t * y * z + s * x, 0.0, //
                t * x * z + s * y, t * y * z - s * x, t * z * z + c,     0.0, //
                0.0,               0.0,               0.0,               1.0,
            ],
        }
    }

    pub fn rotation_x(angle: f32) -> Self {
        Self::rotation_axis(Vec3::new(1.0, 0.0, 0.0), angle)
    }
    pub fn rotation_y(angle: f32) -> Self {
        Self::rotation_axis(Vec3::new(0.0, 1.0, 0.0), angle)
    }
    pub fn rotation_z(angle: f32) -> Self {
        Self::rotation_axis(Vec3::new(0.0, 0.0, 1.0), angle)
    }

    /// Эйлеры XYZ (порядок: Z, затем Y, затем X — как в большинстве CAD).
    pub fn rotation_euler_deg(e: Vec3) -> Self {
        Self::rotation_x(e.x * DEG2RAD)
            * Self::rotation_y(e.y * DEG2RAD)
            * Self::rotation_z(e.z * DEG2RAD)
    }

    pub fn perspective(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fov_y_rad * 0.5).tan();
        let mut r = Self::zero();
        r.m[0] = f / aspect;
        r.m[5] = f;
        r.m[10] = (far + near) / (near - far);
        r.m[11] = -1.0;
        r.m[14] = (2.0 * far * near) / (near - far);
        r
    }

    pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let mut r = Self::zero();
        r.m[0] = 2.0 / (right - left);
        r.m[5] = 2.0 / (top - bottom);
        r.m[10] = -2.0 / (far - near);
        r.m[12] = -(right + left) / (right - left);
        r.m[13] = -(top + bottom) / (top - bottom);
        r.m[14] = -(far + near) / (far - near);
        r.m[15] = 1.0;
        r
    }

    /// Камера смотрит из `eye` на `target`, верх — `up`.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = (target - eye).normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);
        let mut r = Self::identity();
        r.m[0] = s.x;
        r.m[1] = u.x;
        r.m[2] = -f.x;
        r.m[4] = s.y;
        r.m[5] = u.y;
        r.m[6] = -f.y;
        r.m[8] = s.z;
        r.m[9] = u.z;
        r.m[10] = -f.z;
        r.m[12] = -s.dot(eye);
        r.m[13] = -u.dot(eye);
        r.m[14] = f.dot(eye);
        r
    }

    /// Умножение матриц: `self * o` (применяется сначала `o`, потом `self`).
    pub fn mul(self, o: Self) -> Self {
        let mut r = Self::zero();
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.m[k * 4 + row] * o.m[col * 4 + k];
                }
                r.m[col * 4 + row] = sum;
            }
        }
        r
    }

    /// Трансформация точки (с делением на w).
    pub fn transform_point(self, p: Vec3) -> Vec3 {
        let v = Vec4::new(p.x, p.y, p.z, 1.0);
        self.transform_vec4(v).perspective_divide()
    }

    /// Трансформация направления (без переноса, без деления на w).
    pub fn transform_vector(self, p: Vec3) -> Vec3 {
        let v = Vec4::new(p.x, p.y, p.z, 0.0);
        self.transform_vec4(v).xyz()
    }

    pub fn transform_vec4(self, v: Vec4) -> Vec4 {
        Vec4::new(
            self.m[0] * v.x + self.m[4] * v.y + self.m[8] * v.z + self.m[12] * v.w,
            self.m[1] * v.x + self.m[5] * v.y + self.m[9] * v.z + self.m[13] * v.w,
            self.m[2] * v.x + self.m[6] * v.y + self.m[10] * v.z + self.m[14] * v.w,
            self.m[3] * v.x + self.m[7] * v.y + self.m[11] * v.z + self.m[15] * v.w,
        )
    }

    pub fn transpose(self) -> Self {
        let mut r = Self::zero();
        for c in 0..4 {
            for row in 0..4 {
                r.m[c * 4 + row] = self.m[row * 4 + c];
            }
        }
        r
    }

    pub fn determinant(self) -> f32 {
        // Разложение по первой строке (достаточно для проверки вырожденности).
        let a = &self.m;
        let det2 = |a: f32, b: f32, c: f32, d: f32| a * d - b * c;
        let m00 = a[0];
        let m01 = a[1];
        let m02 = a[2];
        let m03 = a[3];
        let m10 = a[4];
        let m11 = a[5];
        let m12 = a[6];
        let m13 = a[7];
        let m20 = a[8];
        let m21 = a[9];
        let m22 = a[10];
        let m23 = a[11];
        let m30 = a[12];
        let m31 = a[13];
        let m32 = a[14];
        let m33 = a[15];
        m00 * (m11 * det2(m22, m23, m32, m33) - m12 * det2(m21, m23, m31, m33) + m13 * det2(m21, m22, m31, m32))
            - m01 * (m10 * det2(m22, m23, m32, m33) - m12 * det2(m20, m23, m30, m33) + m13 * det2(m20, m22, m30, m32))
            + m02 * (m10 * det2(m21, m23, m31, m33) - m11 * det2(m20, m23, m30, m33) + m13 * det2(m20, m21, m30, m31))
            - m03 * (m10 * det2(m21, m22, m31, m32) - m11 * det2(m20, m22, m30, m32) + m12 * det2(m20, m21, m30, m31))
    }

    /// Обратная матрица (для нормалей: транспонированная обратная).
    /// Возвращает None, если матрица вырождена.
    pub fn inverse(self) -> Option<Self> {
        let a = self.m;
        // Присоединённая матрица через миноры.
        let minor = |rows: [usize; 3], cols: [usize; 3]| -> f32 {
            let g = |r: usize, c: usize| a[c * 4 + r];
            let (r0, r1, r2) = (rows[0], rows[1], rows[2]);
            let (c0, c1, c2) = (cols[0], cols[1], cols[2]);
            g(r0, c0) * (g(r1, c1) * g(r2, c2) - g(r1, c2) * g(r2, c1))
                - g(r0, c1) * (g(r1, c0) * g(r2, c2) - g(r1, c2) * g(r2, c0))
                + g(r0, c2) * (g(r1, c0) * g(r2, c1) - g(r1, c1) * g(r2, c0))
        };
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let inv_det = 1.0 / det;
        let mut r = Self::zero();
        for c in 0..4 {
            for row in 0..4 {
                // inverse[row][c] = C[c][row] / det (транспонирование кофакторов).
                let rows: [usize; 3] = (0..4).filter(|&i| i != c).collect::<Vec<_>>().try_into().unwrap();
                let cols: [usize; 3] = (0..4).filter(|&i| i != row).collect::<Vec<_>>().try_into().unwrap();
                let sign = if (row + c) % 2 == 0 { 1.0 } else { -1.0 };
                r.m[c * 4 + row] = sign * minor(rows, cols) * inv_det;
            }
        }
        Some(r)
    }

    /// Верхние 3x3 как матрица нормалей (транспонированная обратная, без переноса).
    pub fn normal_matrix(self) -> Self {
        match self.inverse() {
            Some(inv) => inv.transpose(),
            None => self,
        }
    }

    pub fn translation_part(self) -> Vec3 {
        Vec3::new(self.m[12], self.m[13], self.m[14])
    }

    pub fn set_translation(&mut self, t: Vec3) {
        self.m[12] = t.x;
        self.m[13] = t.y;
        self.m[14] = t.z;
    }
}

impl std::ops::Mul for Mat4 {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        self.mul(o)
    }
}

// ---------------------------------------------------------------------------
// Quaternion (для плавного вращения камеры и тел)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub fn identity() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
    }
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
        let a = axis.normalize();
        let s = (angle * 0.5).sin();
        Self { x: a.x * s, y: a.y * s, z: a.z * s, w: (angle * 0.5).cos() }
    }
    pub fn normalize(self) -> Self {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if l > 1e-12 {
            Self { x: self.x / l, y: self.y / l, z: self.z / l, w: self.w / l }
        } else {
            Self::identity()
        }
    }
    pub fn mul(self, o: Self) -> Self {
        Self {
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        }
    }
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = self.normalize();
        let u = Vec3::new(q.x, q.y, q.z);
        let uv = u.cross(v);
        let uuv = u.cross(uv);
        v + uv * (2.0 * q.w) + uuv * 2.0
    }
    pub fn to_mat4(self) -> Mat4 {
        let q = self.normalize();
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        Mat4 {
            m: [
                1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w),       2.0 * (x * z - y * w),       0.0, //
                2.0 * (x * y - z * w),       1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w),       0.0, //
                2.0 * (x * z + y * w),       2.0 * (y * z - x * w),       1.0 - 2.0 * (x * x + y * y), 0.0, //
                0.0,                         0.0,                         0.0,                         1.0,
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// AABB
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }
    pub fn empty() -> Self {
        Self {
            min: Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
            max: Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }
    pub fn from_point(p: Vec3) -> Self {
        Self { min: p, max: p }
    }
    pub fn include(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    pub fn union(&mut self, o: &Aabb) {
        if o.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = *o;
            return;
        }
        self.include(o.min);
        self.include(o.max);
    }
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    pub fn diagonal(&self) -> f32 {
        self.size().len()
    }
    pub fn volume(&self) -> f32 {
        let s = self.size();
        s.x.max(0.0) * s.y.max(0.0) * s.z.max(0.0)
    }
    pub fn contains(&self, p: Vec3) -> bool {
        p.x >= self.min.x && p.x <= self.max.x
            && p.y >= self.min.y && p.y <= self.max.y
            && p.z >= self.min.z && p.z <= self.max.z
    }
    /// Пересечение луча с AABB (slab method). Возвращает t входа.
    pub fn ray_intersect(&self, ray: &Ray) -> Option<f32> {
        let mut tmin = 0.0f32;
        let mut tmax = f32::INFINITY;
        for i in 0..3 {
            let o = ray.origin.get(i);
            let d = ray.dir.get(i);
            let lo = self.min.get(i);
            let hi = self.max.get(i);
            if d.abs() < 1e-12 {
                if o < lo || o > hi {
                    return None;
                }
            } else {
                let inv = 1.0 / d;
                let mut t0 = (lo - o) * inv;
                let mut t1 = (hi - o) * inv;
                if inv < 0.0 {
                    std::mem::swap(&mut t0, &mut t1);
                }
                tmin = tmin.max(t0);
                tmax = tmax.min(t1);
                if tmax < tmin {
                    return None;
                }
            }
        }
        Some(tmin)
    }
    /// Трансформация AABB матрицей (точный метод через 8 углов).
    pub fn transformed(&self, m: Mat4) -> Aabb {
        if self.is_empty() {
            return *self;
        }
        let mut r = Aabb::empty();
        for i in 0..8 {
            let p = Vec3::new(
                if i & 1 != 0 { self.max.x } else { self.min.x },
                if i & 2 != 0 { self.max.y } else { self.min.y },
                if i & 4 != 0 { self.max.z } else { self.min.z },
            );
            r.include(m.transform_point(p));
        }
        r
    }
}

// ---------------------------------------------------------------------------
// Ray
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3, // нормализованный
}

impl Ray {
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self { origin, dir: dir.normalize() }
    }
    pub fn point_at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
    /// Пересечение луча с треугольником (Möller–Trumbore).
    /// Возвращает (t, u, v) — барицентрические координаты.
    pub fn intersect_triangle(&self, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
        let e1 = b - a;
        let e2 = c - a;
        let p = self.dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-10 {
            return None;
        }
        let inv = 1.0 / det;
        let tv = self.origin - a;
        let u = tv.dot(p) * inv;
        if u < -1e-6 || u > 1.0 + 1e-6 {
            return None;
        }
        let q = tv.cross(e1);
        let v = self.dir.dot(q) * inv;
        if v < -1e-6 || u + v > 1.0 + 1e-6 {
            return None;
        }
        let t = e2.dot(q) * inv;
        if t < 1e-6 {
            return None;
        }
        Some((t, u, v))
    }
}

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
    pub fn white() -> Self {
        Self::rgb(255, 255, 255)
    }
    pub fn black() -> Self {
        Self::rgb(0, 0, 0)
    }
    pub fn to_f32(self) -> (f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
        )
    }
    /// Умножение яркости (для затенения), с клампом.
    pub fn scaled(self, s: f32) -> Self {
        Self {
            r: (self.r as f32 * s).round().clamp(0.0, 255.0) as u8,
            g: (self.g as f32 * s).round().clamp(0.0, 255.0) as u8,
            b: (self.b as f32 * s).round().clamp(0.0, 255.0) as u8,
            a: self.a,
        }
    }
    pub fn lerp(self, o: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: (self.r as f32 + (o.r as f32 - self.r as f32) * t).round() as u8,
            g: (self.g as f32 + (o.g as f32 - self.g as f32) * t).round() as u8,
            b: (self.b as f32 + (o.b as f32 - self.b as f32) * t).round() as u8,
            a: (self.a as f32 + (o.a as f32 - self.a as f32) * t).round() as u8,
        }
    }
    /// Альфа-смешивание поверх фона.
    pub fn blend_over(self, bg: Self) -> Self {
        let a = self.a as f32 / 255.0;
        Self {
            r: (self.r as f32 * a + bg.r as f32 * (1.0 - a)).round() as u8,
            g: (self.g as f32 * a + bg.g as f32 * (1.0 - a)).round() as u8,
            b: (self.b as f32 * a + bg.b as f32 * (1.0 - a)).round() as u8,
            a: 255,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec3_ops() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a.dot(b), 32.0);
        let c = Vec3::new(1.0, 0.0, 0.0).cross(Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(c, Vec3::new(0.0, 0.0, 1.0));
        assert!((Vec3::new(3.0, 4.0, 0.0).len() - 5.0).abs() < 1e-5);
    }

    #[test]
    fn mat4_transform() {
        let m = Mat4::translation(Vec3::new(10.0, 0.0, 0.0)) * Mat4::scale_uniform(2.0);
        let p = m.transform_point(Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(p, Vec3::new(12.0, 2.0, 2.0));
        let r = Mat4::rotation_z(PI / 2.0);
        let p2 = r.transform_point(Vec3::new(1.0, 0.0, 0.0));
        assert!((p2.x).abs() < 1e-5 && (p2.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mat4_inverse_roundtrip() {
        let m = Mat4::translation(Vec3::new(1.0, 2.0, 3.0))
            * Mat4::rotation_euler_deg(Vec3::new(10.0, 20.0, 30.0))
            * Mat4::scale(Vec3::new(2.0, 3.0, 4.0));
        let inv = m.inverse().expect("invertible");
        let p = Vec3::new(0.5, -1.0, 2.0);
        let back = inv.transform_point(m.transform_point(p));
        assert!(back.distance(p) < 1e-3, "back={back:?} p={p:?}");
    }

    #[test]
    fn ray_triangle() {
        let ray = Ray::new(Vec3::new(0.25, 0.25, 5.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = ray.intersect_triangle(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        );
        assert!(hit.is_some());
        let (t, _, _) = hit.unwrap();
        assert!((t - 5.0).abs() < 1e-4);
    }

    #[test]
    fn aabb_ray() {
        let b = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(b.ray_intersect(&ray), Some(4.0));
        let miss = Ray::new(Vec3::new(5.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(b.ray_intersect(&miss), None);
    }
}
