//! Формат проекта AlphaCad (`.acad`) — JSON с описанием сцены.
//!
//! Хранит тела (меши как массивы чисел с ограниченной точностью),
//! трансформации, материалы, эскизы и примитивы. JSON-парсер/сериализатор
//! написан вручную (без внешних зависимостей).

use crate::math::{Color, Mat4, Vec3};
use crate::mesh::{Mesh, Primitive};
use crate::scene::{Body, Scene};
use crate::sketch::{Sketch, SketchEntity, SketchPlane};
use std::path::Path;

// ---------------------------------------------------------------------------
// Минимальный JSON
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        if let Json::Obj(pairs) = self {
            pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

pub fn parse_json(text: &str) -> Result<Json, String> {
    let mut p = JsonParser { bytes: text.as_bytes(), pos: 0 };
    p.skip_ws();
    let v = p.parse_value()?;
    Ok(v)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && (self.bytes[self.pos] as char).is_whitespace() {
            self.pos += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_obj(),
            Some(b'[') => self.parse_arr(),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => {
                self.expect_lit("true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.expect_lit("false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.expect_lit("null")?;
                Ok(Json::Null)
            }
            Some(_) => self.parse_number(),
            None => Err("Неожиданный конец JSON".into()),
        }
    }
    fn expect_lit(&mut self, lit: &str) -> Result<(), String> {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(format!("Ожидалось '{lit}'"))
        }
    }
    fn parse_obj(&mut self) -> Result<Json, String> {
        self.pos += 1; // {
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err("Ожидалось ':' в объекте".into());
            }
            self.pos += 1;
            let val = self.parse_value()?;
            pairs.push((key, val));
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.pos += 1;
            }
        }
        Ok(Json::Obj(pairs))
    }
    fn parse_arr(&mut self) -> Result<Json, String> {
        self.pos += 1; // [
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                break;
            }
            items.push(self.parse_value()?);
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.pos += 1;
            }
        }
        Ok(Json::Arr(items))
    }
    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err("Ожидалась строка".into());
        }
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            self.pos += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or("Обрыв строки")?;
                    self.pos += 1;
                    match e {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'b' => s.push('\u{0008}'),
                        b'f' => s.push('\u{000C}'),
                        b'u' => {
                            let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4.min(self.bytes.len() - self.pos)])
                                .map_err(|_| "bad \\u")?;
                            let code = u32::from_str_radix(hex, 16).unwrap_or(0);
                            self.pos += 4;
                            s.push(char::from_u32(code).unwrap_or('?'));
                        }
                        _ => s.push(e as char),
                    }
                }
                _ => {
                    // UTF-8: копируем байты как есть (строка валидна, т.к. исходник &str).
                    // Простой путь: собираем байты и конвертируем в конце.
                    // Здесь push как char для ASCII; для многобайтных — через from_utf8 ниже.
                    // Чтобы не усложнять, читаем полный UTF-8 символ.
                    if c < 0x80 {
                        s.push(c as char);
                    } else {
                        // Откатываемся и читаем символ целиком.
                        self.pos -= 1;
                        let rest = &self.bytes[self.pos..];
                        let st = std::str::from_utf8(rest).map_err(|_| "bad utf8")?;
                        let ch = st.chars().next().unwrap();
                        s.push(ch);
                        self.pos += ch.len_utf8();
                    }
                }
            }
        }
        Ok(s)
    }
    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos] as char;
            if c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| "bad number")?;
        s.parse::<f64>().map(Json::Num).map_err(|_| format!("Не число: '{s}'"))
    }
}

pub fn to_json_string(v: &Json) -> String {
    let mut s = String::new();
    write_json(v, &mut s, 0);
    s
}

fn write_json(v: &Json, out: &mut String, indent: usize) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{n:.1}"));
            } else {
                out.push_str(&format!("{n}"));
            }
        }
        Json::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out, indent + 1);
            }
            out.push(']');
        }
        Json::Obj(pairs) => {
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(&Json::Str(k.clone()), out, indent + 1);
                out.push(':');
                write_json(val, out, indent + 1);
            }
            out.push('}');
        }
    }
}

// ---------------------------------------------------------------------------
// Сериализация сцены
// ---------------------------------------------------------------------------

fn vec3_json(v: Vec3) -> Json {
    Json::Arr(vec![Json::Num(v.x as f64), Json::Num(v.y as f64), Json::Num(v.z as f64)])
}

fn mat4_json(m: Mat4) -> Json {
    Json::Arr(m.m.iter().map(|&x| Json::Num(x as f64)).collect())
}

fn mesh_json(mesh: &Mesh) -> Json {
    let pos: Vec<Json> = mesh
        .positions
        .iter()
        .map(|p| Json::Arr(vec![Json::Num(round4(p.x)), Json::Num(round4(p.y)), Json::Num(round4(p.z))]))
        .collect();
    let idx: Vec<Json> = mesh.indices.iter().map(|&i| Json::Num(i as f64)).collect();
    Json::Obj(vec![
        ("name".into(), Json::Str(mesh.name.clone())),
        ("positions".into(), Json::Arr(pos)),
        ("indices".into(), Json::Arr(idx)),
    ])
}

fn round4(v: f32) -> f64 {
    ((v * 10000.0).round() / 10000.0) as f64
}

fn primitive_json(p: &Primitive) -> Json {
    match *p {
        Primitive::Box { w, h, d } => Json::Obj(vec![
            ("kind".into(), Json::Str("box".into())),
            ("w".into(), Json::Num(w as f64)),
            ("h".into(), Json::Num(h as f64)),
            ("d".into(), Json::Num(d as f64)),
        ]),
        Primitive::Cylinder { radius, height, segments } => Json::Obj(vec![
            ("kind".into(), Json::Str("cylinder".into())),
            ("radius".into(), Json::Num(radius as f64)),
            ("height".into(), Json::Num(height as f64)),
            ("segments".into(), Json::Num(segments as f64)),
        ]),
        Primitive::Sphere { radius, segments, rings } => Json::Obj(vec![
            ("kind".into(), Json::Str("sphere".into())),
            ("radius".into(), Json::Num(radius as f64)),
            ("segments".into(), Json::Num(segments as f64)),
            ("rings".into(), Json::Num(rings as f64)),
        ]),
        Primitive::Cone { radius_bottom, radius_top, height, segments } => Json::Obj(vec![
            ("kind".into(), Json::Str("cone".into())),
            ("radius_bottom".into(), Json::Num(radius_bottom as f64)),
            ("radius_top".into(), Json::Num(radius_top as f64)),
            ("height".into(), Json::Num(height as f64)),
            ("segments".into(), Json::Num(segments as f64)),
        ]),
        Primitive::Torus { major_radius, minor_radius, major_segments, minor_segments } => Json::Obj(vec![
            ("kind".into(), Json::Str("torus".into())),
            ("major_radius".into(), Json::Num(major_radius as f64)),
            ("minor_radius".into(), Json::Num(minor_radius as f64)),
            ("major_segments".into(), Json::Num(major_segments as f64)),
            ("minor_segments".into(), Json::Num(minor_segments as f64)),
        ]),
        Primitive::Capsule { radius, height, segments } => Json::Obj(vec![
            ("kind".into(), Json::Str("capsule".into())),
            ("radius".into(), Json::Num(radius as f64)),
            ("height".into(), Json::Num(height as f64)),
            ("segments".into(), Json::Num(segments as f64)),
        ]),
        Primitive::Tube { radius_outer, radius_inner, height, segments } => Json::Obj(vec![
            ("kind".into(), Json::Str("tube".into())),
            ("radius_outer".into(), Json::Num(radius_outer as f64)),
            ("radius_inner".into(), Json::Num(radius_inner as f64)),
            ("height".into(), Json::Num(height as f64)),
            ("segments".into(), Json::Num(segments as f64)),
        ]),
        Primitive::Wedge { w, h, d } => Json::Obj(vec![
            ("kind".into(), Json::Str("wedge".into())),
            ("w".into(), Json::Num(w as f64)),
            ("h".into(), Json::Num(h as f64)),
            ("d".into(), Json::Num(d as f64)),
        ]),
        Primitive::Pyramid { base, height, segments } => Json::Obj(vec![
            ("kind".into(), Json::Str("pyramid".into())),
            ("base".into(), Json::Num(base as f64)),
            ("height".into(), Json::Num(height as f64)),
            ("segments".into(), Json::Num(segments as f64)),
        ]),
    }
}

fn sketch_json(sk: &Sketch) -> Json {
    let ents: Vec<Json> = sk
        .entities
        .iter()
        .map(|e| match e {
            SketchEntity::Line { a, b } => Json::Obj(vec![
                ("kind".into(), Json::Str("line".into())),
                ("a".into(), vec3_json(*a)),
                ("b".into(), vec3_json(*b)),
            ]),
            SketchEntity::Circle { center, radius } => Json::Obj(vec![
                ("kind".into(), Json::Str("circle".into())),
                ("center".into(), vec3_json(*center)),
                ("radius".into(), Json::Num(*radius as f64)),
            ]),
            SketchEntity::Arc { center, radius, start_angle, end_angle } => Json::Obj(vec![
                ("kind".into(), Json::Str("arc".into())),
                ("center".into(), vec3_json(*center)),
                ("radius".into(), Json::Num(*radius as f64)),
                ("start_angle".into(), Json::Num(*start_angle as f64)),
                ("end_angle".into(), Json::Num(*end_angle as f64)),
            ]),
            SketchEntity::Rect { min, max } => Json::Obj(vec![
                ("kind".into(), Json::Str("rect".into())),
                ("min".into(), vec3_json(*min)),
                ("max".into(), vec3_json(*max)),
            ]),
        })
        .collect();
    Json::Obj(vec![
        ("name".into(), Json::Str(sk.name.clone())),
        ("plane".into(), Json::Str(sk.plane.name().into())),
        ("entities".into(), Json::Arr(ents)),
    ])
}

pub fn scene_to_json(scene: &Scene) -> Json {
    let bodies: Vec<Json> = scene
        .bodies
        .iter()
        .map(|b| {
            let mut pairs = vec![
                ("id".into(), Json::Num(b.id as f64)),
                ("name".into(), Json::Str(b.name.clone())),
                ("visible".into(), Json::Bool(b.visible)),
                ("transform".into(), mat4_json(b.transform)),
                ("color".into(), Json::Arr(vec![
                    Json::Num(b.material.color.r as f64),
                    Json::Num(b.material.color.g as f64),
                    Json::Num(b.material.color.b as f64),
                ])),
                ("mesh".into(), mesh_json(&b.mesh)),
            ];
            if let Some(p) = &b.primitive {
                pairs.push(("primitive".into(), primitive_json(p)));
            }
            if let Some(sk) = &b.sketch {
                pairs.push(("sketch".into(), sketch_json(sk)));
            }
            Json::Obj(pairs)
        })
        .collect();
    Json::Obj(vec![
        ("format".into(), Json::Str("alphacad-project".into())),
        ("version".into(), Json::Num(1.0)),
        ("bodies".into(), Json::Arr(bodies)),
    ])
}

pub fn save_project(scene: &Scene, path: &Path) -> Result<(), String> {
    let text = to_json_string(&scene_to_json(scene));
    std::fs::write(path, text)
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Десериализация
// ---------------------------------------------------------------------------

fn json_vec3(v: &Json) -> Vec3 {
    if let Some(a) = v.as_arr() {
        if a.len() >= 3 {
            return Vec3::new(
                a[0].as_f64().unwrap_or(0.0) as f32,
                a[1].as_f64().unwrap_or(0.0) as f32,
                a[2].as_f64().unwrap_or(0.0) as f32,
            );
        }
    }
    Vec3::zero()
}

fn json_mat4(v: &Json) -> Mat4 {
    if let Some(a) = v.as_arr() {
        if a.len() >= 16 {
            let mut m = [0.0f32; 16];
            for i in 0..16 {
                m[i] = a[i].as_f64().unwrap_or(0.0) as f32;
            }
            return Mat4 { m };
        }
    }
    Mat4::identity()
}

fn json_mesh(v: &Json) -> Mesh {
    let mut mesh = Mesh::new();
    mesh.name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if let Some(pos) = v.get("positions").and_then(|x| x.as_arr()) {
        for p in pos {
            mesh.positions.push(json_vec3(p));
        }
    }
    mesh.normals = vec![Vec3::zero(); mesh.positions.len()];
    if let Some(idx) = v.get("indices").and_then(|x| x.as_arr()) {
        for i in idx {
            mesh.indices.push(i.as_f64().unwrap_or(0.0) as u32);
        }
    }
    mesh.compute_smooth_normals();
    mesh
}

fn json_primitive(v: &Json) -> Option<Primitive> {
    let kind = v.get("kind")?.as_str()?;
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    let u = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) as u32;
    Some(match kind {
        "box" => Primitive::Box { w: f("w"), h: f("h"), d: f("d") },
        "cylinder" => Primitive::Cylinder { radius: f("radius"), height: f("height"), segments: u("segments") },
        "sphere" => Primitive::Sphere { radius: f("radius"), segments: u("segments"), rings: u("rings") },
        "cone" => Primitive::Cone { radius_bottom: f("radius_bottom"), radius_top: f("radius_top"), height: f("height"), segments: u("segments") },
        "torus" => Primitive::Torus { major_radius: f("major_radius"), minor_radius: f("minor_radius"), major_segments: u("major_segments"), minor_segments: u("minor_segments") },
        "capsule" => Primitive::Capsule { radius: f("radius"), height: f("height"), segments: u("segments") },
        "tube" => Primitive::Tube { radius_outer: f("radius_outer"), radius_inner: f("radius_inner"), height: f("height"), segments: u("segments") },
        "wedge" => Primitive::Wedge { w: f("w"), h: f("h"), d: f("d") },
        "pyramid" => Primitive::Pyramid { base: f("base"), height: f("height"), segments: u("segments") },
        _ => return None,
    })
}

fn json_sketch(v: &Json) -> Option<Sketch> {
    let plane = SketchPlane::from_name(v.get("plane").and_then(|x| x.as_str()).unwrap_or("XY"));
    let mut sk = Sketch::new(plane);
    sk.name = v.get("name").and_then(|x| x.as_str()).unwrap_or("Эскиз").to_string();
    if let Some(ents) = v.get("entities").and_then(|x| x.as_arr()) {
        for e in ents {
            let kind = e.get("kind").and_then(|x| x.as_str()).unwrap_or("");
            match kind {
                "line" => sk.entities.push(SketchEntity::Line {
                    a: json_vec3(e.get("a")?),
                    b: json_vec3(e.get("b")?),
                }),
                "circle" => sk.entities.push(SketchEntity::Circle {
                    center: json_vec3(e.get("center")?),
                    radius: e.get("radius").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                }),
                "arc" => sk.entities.push(SketchEntity::Arc {
                    center: json_vec3(e.get("center")?),
                    radius: e.get("radius").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                    start_angle: e.get("start_angle").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                    end_angle: e.get("end_angle").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                }),
                "rect" => sk.entities.push(SketchEntity::Rect {
                    min: json_vec3(e.get("min")?),
                    max: json_vec3(e.get("max")?),
                }),
                _ => {}
            }
        }
    }
    Some(sk)
}

pub fn load_project(path: &Path) -> Result<Scene, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
    let json = parse_json(&text)?;
    let fmt = json.get("format").and_then(|v| v.as_str()).unwrap_or("");
    if fmt != "alphacad-project" && fmt != "forgecad-project" {
        return Err("Не файл проекта AlphaCad".into());
    }
    let mut scene = Scene::new();
    if let Some(bodies) = json.get("bodies").and_then(|v| v.as_arr()) {
        for b in bodies {
            let mut body = Body::new(
                b.get("name").and_then(|v| v.as_str()).unwrap_or("Тело").to_string(),
                json_mesh(b.get("mesh").unwrap_or(&Json::Null)),
            );
            body.id = b.get("id").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
            body.visible = b.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
            body.transform = b.get("transform").map(json_mat4).unwrap_or(Mat4::identity());
            if let Some(c) = b.get("color").and_then(|v| v.as_arr()) {
                if c.len() >= 3 {
                    body.material.color = Color::rgb(
                        c[0].as_f64().unwrap_or(200.0) as u8,
                        c[1].as_f64().unwrap_or(200.0) as u8,
                        c[2].as_f64().unwrap_or(200.0) as u8,
                    );
                }
            }
            body.primitive = b.get("primitive").and_then(json_primitive);
            body.sketch = b.get("sketch").and_then(json_sketch);
            scene.bodies.push(body);
        }
    }
    scene.next_id = scene.bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1;
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn json_roundtrip() {
        let v = Json::Obj(vec![
            ("a".into(), Json::Num(1.5)),
            ("b".into(), Json::Str("привет".into())),
            ("c".into(), Json::Arr(vec![Json::Bool(true), Json::Null])),
        ]);
        let s = to_json_string(&v);
        let back = parse_json(&s).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn scene_roundtrip() {
        let mut scene = Scene::new();
        scene.add_body(Body::new("Куб", primitive_box(1.0, 2.0, 3.0)));
        let json = scene_to_json(&scene);
        let text = to_json_string(&json);
        let back = parse_json(&text).unwrap();
        assert_eq!(back.get("format").and_then(|v| v.as_str()), Some("alphacad-project"));
        let bodies = back.get("bodies").and_then(|v| v.as_arr()).unwrap();
        assert_eq!(bodies.len(), 1);
        let mesh = json_mesh(bodies[0].get("mesh").unwrap());
        assert_eq!(mesh.triangle_count(), 12);
    }
}
