//! 3MF (3D Manufacturing Format) — чтение и запись.
//!
//! 3MF — ZIP-архив (OPC) с XML-описанием модели `3D/3dmodel.model`.
//! Поддерживается:
//! - сетки (`<mesh>`), составные объекты (`<components>`) и элементы сборки
//!   (`<build><item>`) с матрицами преобразования;
//! - единицы измерения (`unit`: micron … meter) с переводом в миллиметры;
//! - цвета из `<basematerials>` и `<m:colorgroup>` (в цвета вершин);
//! - запись: одна сетка, цвета вершин — через `<m:colorgroup>`.
//!
//! Все объекты сборки объединяются в один меш (как при импорте STL/OBJ).

use super::zip;
use crate::math::{Color, Mat4, Vec3};
use crate::mesh::Mesh;
use std::collections::HashMap;
use std::path::Path;

const NS_CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const NS_MATERIAL: &str = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02";
const REL_MODEL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

pub fn load(path: &Path) -> Result<Mesh, String> {
    let data = std::fs::read(path).map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
    let mut m = load_from_bytes(&data)?;
    m.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "3MF".into());
    Ok(m)
}

pub fn save(mesh: &Mesh, path: &Path) -> Result<(), String> {
    std::fs::write(path, to_bytes(mesh)).map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Мини-парсер XML (элементы и атрибуты; текстовое содержимое не нужно)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Tag<'a> {
    /// Локальное имя без префикса пространства имён.
    name: &'a str,
    attrs: Vec<(&'a str, String)>,
    closing: bool,
    self_closing: bool,
}

impl Tag<'_> {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }
    fn num<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.attr(key).and_then(|v| v.trim().parse().ok())
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Последовательный разбор тегов.
struct XmlTags<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Iterator for XmlTags<'a> {
    type Item = Tag<'a>;
    fn next(&mut self) -> Option<Tag<'a>> {
        let b = self.s.as_bytes();
        loop {
            let start = self.pos + self.s[self.pos..].find('<')?;
            let rest = &self.s[start..];
            if rest.starts_with("<!--") {
                self.pos = start + rest.find("-->").map(|e| e + 3).unwrap_or(rest.len());
                continue;
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                self.pos = start + rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
                continue;
            }
            let mut i = start + 1;
            let closing = b.get(i) == Some(&b'/');
            if closing {
                i += 1;
            }
            let ns = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
                i += 1;
            }
            let name = local(&self.s[ns..i]);
            let mut attrs = Vec::new();
            let mut self_closing = false;
            loop {
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i >= b.len() {
                    self.pos = b.len();
                    break;
                }
                if b[i] == b'>' {
                    i += 1;
                    break;
                }
                if b[i] == b'/' {
                    self_closing = true;
                    i += 1;
                    continue;
                }
                let ks = i;
                while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                let key = local(&self.s[ks..i]);
                while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'=') {
                    i += 1;
                }
                if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                    let q = b[i];
                    let vs = i + 1;
                    let ve = vs + self.s[vs..].bytes().position(|c| c == q).unwrap_or(self.s.len() - vs);
                    attrs.push((key, unescape(&self.s[vs..ve])));
                    i = (ve + 1).min(b.len());
                }
            }
            self.pos = i.min(b.len());
            return Some(Tag { name, attrs, closing, self_closing });
        }
    }
}

fn tags(s: &str) -> XmlTags<'_> {
    XmlTags { s, pos: 0 }
}

// ---------------------------------------------------------------------------
// Чтение
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ObjMesh {
    verts: Vec<Vec3>,
    tris: Vec<[u32; 3]>,
    /// Цвет каждого угла треугольника (если задан).
    tri_colors: Vec<Option<[Color; 3]>>,
}

#[derive(Default)]
struct Object {
    mesh: Option<ObjMesh>,
    components: Vec<(u32, Mat4)>,
}

fn parse_color(s: &str) -> Option<Color> {
    let h = s.trim().strip_prefix('#')?;
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 | 8 => Some(Color::rgb(v(0)?, v(2)?, v(4)?)),
        _ => None,
    }
}

/// Матрица 3MF «m00 m01 m02 m10 … m32» (векторы-строки) → Mat4 (векторы-столбцы).
fn parse_transform(s: Option<&str>) -> Mat4 {
    let Some(s) = s else { return Mat4::identity() };
    let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if v.len() != 12 {
        return Mat4::identity();
    }
    let mut m = Mat4::identity();
    for r in 0..3 {
        for c in 0..3 {
            m.m[c * 4 + r] = v[c * 3 + r];
        }
        m.m[12 + r] = v[9 + r];
    }
    m
}

fn unit_scale(u: &str) -> f32 {
    match u {
        "micron" => 0.001,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        _ => 1.0,
    }
}

/// Путь к основной модели из `_rels/.rels`.
fn model_path(entries: &[zip::ZipEntry], data: &[u8]) -> Option<String> {
    let rels = entries.iter().find(|e| e.name.eq_ignore_ascii_case("_rels/.rels"))?;
    let text = String::from_utf8(zip::extract(data, rels).ok()?).ok()?;
    for t in tags(&text) {
        if t.name == "Relationship" && t.attr("Type").map(|v| v.trim_end_matches('/').ends_with("3dmodel")).unwrap_or(false) {
            return t.attr("Target").map(|v| v.trim_start_matches('/').to_string());
        }
    }
    None
}

pub fn load_from_bytes(data: &[u8]) -> Result<Mesh, String> {
    let entries = zip::list(data).map_err(|e| format!("3MF: {e}"))?;
    let path = model_path(&entries, data);
    let entry = path
        .as_ref()
        .and_then(|p| entries.iter().find(|e| e.name.eq_ignore_ascii_case(p)))
        .or_else(|| entries.iter().find(|e| e.name.to_ascii_lowercase().ends_with(".model")))
        .ok_or("3MF: в архиве нет файла модели (3D/3dmodel.model)")?;
    let xml = zip::extract(data, entry).map_err(|e| format!("3MF: {e}"))?;
    let text = String::from_utf8_lossy(&xml);
    parse_model(&text)
}

fn parse_model(text: &str) -> Result<Mesh, String> {
    let mut scale = 1.0f32;
    let mut objects: HashMap<u32, Object> = HashMap::new();
    let mut colors: HashMap<u32, Vec<Color>> = HashMap::new();
    let mut build: Vec<(u32, Mat4)> = Vec::new();

    let mut cur_obj: Option<(u32, Object, Option<(u32, u32)>)> = None;
    let mut cur_group: Option<(u32, Vec<Color>)> = None;
    let mut in_build = false;

    for t in tags(text) {
        if t.closing {
            match t.name {
                "object" => {
                    if let Some((id, o, _)) = cur_obj.take() {
                        objects.insert(id, o);
                    }
                }
                "basematerials" | "colorgroup" => {
                    if let Some((id, g)) = cur_group.take() {
                        colors.insert(id, g);
                    }
                }
                "build" => in_build = false,
                _ => {}
            }
            continue;
        }
        match t.name {
            "model" => scale = unit_scale(t.attr("unit").unwrap_or("millimeter")),
            "basematerials" | "colorgroup" => {
                cur_group = Some((t.num("id").unwrap_or(0), Vec::new()));
                if t.self_closing {
                    if let Some((id, g)) = cur_group.take() {
                        colors.insert(id, g);
                    }
                }
            }
            "base" => {
                if let Some((_, g)) = cur_group.as_mut() {
                    g.push(t.attr("displaycolor").and_then(parse_color).unwrap_or(Color::rgb(200, 200, 200)));
                }
            }
            "color" => {
                if let Some((_, g)) = cur_group.as_mut() {
                    g.push(t.attr("color").and_then(parse_color).unwrap_or(Color::rgb(200, 200, 200)));
                }
            }
            "object" => {
                let id = t.num("id").unwrap_or(0);
                let def = match (t.num::<u32>("pid"), t.num::<u32>("pindex")) {
                    (Some(p), Some(i)) => Some((p, i)),
                    (Some(p), None) => Some((p, 0)),
                    _ => None,
                };
                cur_obj = Some((id, Object::default(), def));
                if t.self_closing {
                    if let Some((id, o, _)) = cur_obj.take() {
                        objects.insert(id, o);
                    }
                }
            }
            "mesh" => {
                if let Some((_, o, _)) = cur_obj.as_mut() {
                    o.mesh = Some(ObjMesh::default());
                }
            }
            "vertex" => {
                if let Some((_, Object { mesh: Some(m), .. }, _)) = cur_obj.as_mut() {
                    m.verts.push(Vec3::new(t.num("x").unwrap_or(0.0), t.num("y").unwrap_or(0.0), t.num("z").unwrap_or(0.0)));
                }
            }
            "triangle" => {
                if let Some((_, Object { mesh: Some(m), .. }, def)) = cur_obj.as_mut() {
                    let (Some(a), Some(b), Some(c)) = (t.num::<u32>("v1"), t.num::<u32>("v2"), t.num::<u32>("v3")) else { continue };
                    m.tris.push([a, b, c]);
                    let pid = t.num::<u32>("pid").or(def.map(|d| d.0));
                    let p1 = t.num::<u32>("p1").or(if t.attr("pid").is_none() { def.map(|d| d.1) } else { None });
                    let col = match (pid.and_then(|p| colors.get(&p)), p1) {
                        (Some(g), Some(p1)) => {
                            let p2 = t.num::<u32>("p2").unwrap_or(p1);
                            let p3 = t.num::<u32>("p3").unwrap_or(p1);
                            let get = |i: u32| g.get(i as usize).copied();
                            match (get(p1), get(p2), get(p3)) {
                                (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    m.tri_colors.push(col);
                }
            }
            "component" => {
                if let Some((_, o, _)) = cur_obj.as_mut() {
                    if let Some(oid) = t.num("objectid") {
                        o.components.push((oid, parse_transform(t.attr("transform"))));
                    }
                }
            }
            "build" => in_build = true,
            "item" if in_build => {
                if let Some(oid) = t.num("objectid") {
                    build.push((oid, parse_transform(t.attr("transform"))));
                }
            }
            _ => {}
        }
    }

    // Если сборка не задана — все объекты, на которые никто не ссылается.
    if build.is_empty() {
        let referenced: std::collections::HashSet<u32> = objects.values().flat_map(|o| o.components.iter().map(|c| c.0)).collect();
        let mut ids: Vec<u32> = objects.keys().copied().filter(|k| !referenced.contains(k)).collect();
        ids.sort_unstable();
        build = ids.into_iter().map(|i| (i, Mat4::identity())).collect();
    }

    let mut out = Mesh::with_name("3MF");
    let mut out_colors: Vec<Option<Color>> = Vec::new();
    let unit = Mat4::scale(Vec3::new(scale, scale, scale));
    fn emit(objects: &HashMap<u32, Object>, id: u32, tf: Mat4, depth: u32, out: &mut Mesh, cols: &mut Vec<Option<Color>>) -> Result<(), String> {
        if depth > 32 {
            return Err("3MF: слишком глубокая вложенность компонентов".into());
        }
        let o = objects.get(&id).ok_or_else(|| format!("3MF: нет объекта {id}"))?;
        if let Some(m) = &o.mesh {
            let base = out.positions.len() as u32;
            for &v in &m.verts {
                out.positions.push(tf.transform_point(v));
                cols.push(None);
            }
            let n = m.verts.len() as u32;
            for (k, tri) in m.tris.iter().enumerate() {
                if tri.iter().any(|&i| i >= n) || tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
                    continue;
                }
                out.indices.extend_from_slice(&[base + tri[0], base + tri[1], base + tri[2]]);
                if let Some(Some(c)) = m.tri_colors.get(k) {
                    for j in 0..3 {
                        let slot = &mut cols[(base + tri[j]) as usize];
                        if slot.is_none() {
                            *slot = Some(c[j]);
                        }
                    }
                }
            }
        }
        for (cid, ctf) in &o.components {
            emit(objects, *cid, tf * *ctf, depth + 1, out, cols)?;
        }
        Ok(())
    }
    for (id, tf) in &build {
        emit(&objects, *id, unit * *tf, 0, &mut out, &mut out_colors)?;
    }
    if out.indices.is_empty() {
        return Err("3MF: модель не содержит треугольников".into());
    }
    if out_colors.iter().any(|c| c.is_some()) {
        out.colors = Some(out_colors.into_iter().map(|c| c.unwrap_or(Color::rgb(200, 200, 200))).collect());
    }
    // Матрица с отрицательным определителем выворачивает грани.
    out.compute_smooth_normals();
    Ok(out)
}

// ---------------------------------------------------------------------------
// Запись
// ---------------------------------------------------------------------------

fn model_xml(mesh: &Mesh) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(mesh.positions.len() * 48 + mesh.indices.len() * 16 + 1024);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(s, "<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"{NS_CORE}\" xmlns:m=\"{NS_MATERIAL}\">");
    let _ = writeln!(s, " <metadata name=\"Application\">AlphaCad {}</metadata>", crate::VERSION);
    let name: String = mesh.name.chars().filter(|c| !matches!(c, '<' | '>' | '&' | '"')).collect();
    if !name.is_empty() {
        let _ = writeln!(s, " <metadata name=\"Title\">{name}</metadata>");
    }
    s.push_str(" <resources>\n");
    // Цвета вершин → colorgroup с уникальными цветами.
    let colors = mesh.colors.as_ref().filter(|c| c.len() == mesh.positions.len());
    let mut color_idx: Vec<u32> = Vec::new();
    if let Some(cs) = colors {
        let mut map: HashMap<(u8, u8, u8), u32> = HashMap::new();
        let mut list = Vec::new();
        for c in cs {
            let k = (c.r, c.g, c.b);
            let n = map.len() as u32;
            let i = *map.entry(k).or_insert_with(|| {
                list.push(k);
                n
            });
            color_idx.push(i);
        }
        s.push_str("  <m:colorgroup id=\"2\">\n");
        for (r, g, b) in list {
            let _ = writeln!(s, "   <m:color color=\"#{r:02X}{g:02X}{b:02X}\"/>");
        }
        s.push_str("  </m:colorgroup>\n");
    }
    let _ = writeln!(s, "  <object id=\"1\" type=\"model\"{}{}>", if name.is_empty() { String::new() } else { format!(" name=\"{name}\"") }, if colors.is_some() { " pid=\"2\" pindex=\"0\"" } else { "" });
    s.push_str("   <mesh>\n    <vertices>\n");
    for p in &mesh.positions {
        let _ = writeln!(s, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", p.x, p.y, p.z);
    }
    s.push_str("    </vertices>\n    <triangles>\n");
    for t in mesh.indices.chunks_exact(3) {
        if colors.is_some() {
            let (a, b, c) = (color_idx[t[0] as usize], color_idx[t[1] as usize], color_idx[t[2] as usize]);
            let _ = writeln!(s, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"2\" p1=\"{a}\" p2=\"{b}\" p3=\"{c}\"/>", t[0], t[1], t[2]);
        } else {
            let _ = writeln!(s, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]);
        }
    }
    s.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build>\n  <item objectid=\"1\"/>\n </build>\n</model>\n");
    s
}

pub fn to_bytes(mesh: &Mesh) -> Vec<u8> {
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n</Types>\n";
    let rels = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"{REL_MODEL}\"/>\n</Relationships>\n");
    // 3MF требует общих вершин (иначе слайсеры видят «несшитую» сетку)
    // и запрещает вырожденные треугольники — сшиваем копию.
    let mut welded = mesh.clone();
    welded.weld();
    let model = model_xml(&welded);
    zip::create(&[("[Content_Types].xml", content_types.as_bytes()), ("_rels/.rels", rels.as_bytes()), ("3D/3dmodel.model", model.as_bytes())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_box() {
        let mut m = crate::mesh::primitive_box(10.0, 20.0, 30.0);
        m.name = "Box".into();
        let bytes = to_bytes(&m);
        let back = load_from_bytes(&bytes).unwrap();
        assert_eq!(back.triangle_count(), m.triangle_count());
        assert!((back.volume() - 6000.0).abs() < 1e-2, "volume {}", back.volume());
        assert!(back.colors.is_none());
    }

    #[test]
    fn roundtrip_colors() {
        let mut m = crate::mesh::primitive_box(1.0, 1.0, 1.0);
        let n = m.positions.len();
        m.colors = Some((0..n).map(|i| if i % 2 == 0 { Color::rgb(255, 0, 0) } else { Color::rgb(0, 0, 255) }).collect());
        let back = load_from_bytes(&to_bytes(&m)).unwrap();
        let c = back.colors.unwrap();
        assert_eq!(c[0], Color::rgb(255, 0, 0));
        assert_eq!(c[1], Color::rgb(0, 0, 255));
    }

    #[test]
    fn components_transforms_and_units() {
        let xml = r##"<?xml version="1.0"?>
<model unit="centimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources>
  <basematerials id="5"><base name="r" displaycolor="#FF0000FF"/></basematerials>
  <object id="1" type="model" pid="5" pindex="0"><mesh>
   <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
   <triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3"/></triangles>
  </mesh></object>
  <object id="2" type="model"><components>
   <component objectid="1"/>
   <component objectid="1" transform="1 0 0 0 1 0 0 0 1 5 0 0"/>
  </components></object>
 </resources>
 <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 0 2"/></build>
</model>"##;
        let m = parse_model(xml).unwrap();
        assert_eq!(m.triangle_count(), 8);
        let b = m.aabb();
        // Сдвиг 5 см по X и 2 см по Z, единицы — сантиметры.
        assert!((b.max.x - 60.0).abs() < 1e-4 && (b.min.z - 20.0).abs() < 1e-4, "{b:?}");
        assert_eq!(m.colors.unwrap()[0], Color::rgb(255, 0, 0));
    }

    #[test]
    fn reads_external_deflated_package() {
        // Архив создан Python zipfile (deflate), модель — не по стандартному пути.
        let m = load_from_bytes(include_bytes!("testdata/tetra.3mf")).unwrap();
        assert_eq!(m.triangle_count(), 160);
        assert!((m.volume() - 40.0 / 6.0).abs() < 1e-3, "volume {}", m.volume());
    }

    #[test]
    fn rejects_non_zip() {
        assert!(load_from_bytes(b"solid x").is_err());
    }
}
