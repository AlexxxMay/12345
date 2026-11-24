//! PLY: ASCII и binary_little_endian. Чтение и запись.

use crate::math::Vec3;
use crate::mesh::Mesh;
use std::path::Path;

pub fn load(path: &Path) -> Result<Mesh, String> {
    let data = std::fs::read(path)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
    load_from_bytes(&data, &path.display().to_string())
}

pub fn load_from_bytes(data: &[u8], source_name: &str) -> Result<Mesh, String> {
    if !data.starts_with(b"ply") {
        return Err("Не PLY-файл (нет заголовка 'ply')".into());
    }
    // Найти конец заголовка.
    let header_end = find_header_end(data).ok_or("PLY: не найден конец заголовка")?;
    let header = String::from_utf8_lossy(&data[..header_end]).to_string();
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut format = String::new();
    let mut vertex_props: Vec<String> = Vec::new();
    let mut face_props: Vec<String> = Vec::new();
    let mut in_vertex = false;
    let mut in_face = false;
    for line in header.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0] == "element" {
            match parts[1] {
                "vertex" => {
                    vertex_count = parts[2].parse().unwrap_or(0);
                    in_vertex = true;
                    in_face = false;
                }
                "face" => {
                    face_count = parts[2].parse().unwrap_or(0);
                    in_face = true;
                    in_vertex = false;
                }
                _ => {
                    in_vertex = false;
                    in_face = false;
                }
            }
        } else if parts.len() >= 3 && parts[0] == "property" {
            if in_vertex {
                vertex_props.push(parts[2].to_string());
            } else if in_face {
                face_props.push(parts[2].to_string());
            }
        } else if parts.len() >= 2 && parts[0] == "format" {
            format = parts[1].to_string();
        }
    }
    if vertex_count == 0 {
        return Err("PLY: не найдено вершин".into());
    }
    let body = &data[header_end..];
    let mut mesh = Mesh::new();
    match format.as_str() {
        "ascii" => parse_ascii_body(body, vertex_count, face_count, &vertex_props, &face_props, &mut mesh)?,
        "binary_little_endian" => parse_binary_body(body, vertex_count, face_count, &vertex_props, &face_props, &mut mesh)?,
        "binary_big_endian" => return Err("PLY big-endian не поддерживается".into()),
        _ => return Err(format!("PLY: неизвестный формат '{format}'")),
    }
    if mesh.is_empty() {
        return Err("PLY не содержит треугольников".into());
    }
    mesh.compute_smooth_normals();
    mesh.name = source_name.to_string();
    Ok(mesh)
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    // Ищем строку "end_header\n".
    let needle = b"end_header";
    if data.len() < needle.len() {
        return None;
    }
    for i in 0..=data.len() - needle.len() {
        if &data[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            while j < data.len() && (data[j] == b'\n' || data[j] == b'\r') {
                j += 1;
            }
            return Some(j);
        }
    }
    None
}

fn prop_index(props: &[String], name: &str) -> Option<usize> {
    props.iter().position(|p| p == name)
}

fn parse_ascii_body(
    body: &[u8],
    vcount: usize,
    fcount: usize,
    vprops: &[String],
    fprops: &[String],
    mesh: &mut Mesh,
) -> Result<(), String> {
    let text = String::from_utf8_lossy(body);
    let mut lines = text.lines();
    let xi = prop_index(vprops, "x").unwrap_or(0);
    let yi = prop_index(vprops, "y").unwrap_or(1);
    let zi = prop_index(vprops, "z").unwrap_or(2);
    for _ in 0..vcount {
        let Some(line) = lines.next() else { break };
        let vals: Vec<f32> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if vals.len() > zi {
            mesh.add_vertex(Vec3::new(vals[xi], vals[yi], vals[zi]));
        }
    }
    let _ = fprops;
    for _ in 0..fcount {
        let Some(line) = lines.next() else { break };
        let vals: Vec<i64> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if vals.len() >= 4 {
            let n = vals[0] as usize;
            for k in 1..n - 1 {
                if k + 1 < vals.len() {
                    mesh.add_triangle(
                        vals[1] as u32,
                        vals[k + 1] as u32,
                        vals[k + 2] as u32,
                    );
                }
            }
        }
    }
    Ok(())
}

fn parse_binary_body(
    body: &[u8],
    vcount: usize,
    fcount: usize,
    vprops: &[String],
    fprops: &[String],
    mesh: &mut Mesh,
) -> Result<(), String> {
    let mut off = 0usize;
    let read_f32 = |body: &[u8], off: &mut usize| -> f32 {
        let v = f32::from_le_bytes([body[*off], body[*off + 1], body[*off + 2], body[*off + 3]]);
        *off += 4;
        v
    };
    let read_u8 = |body: &[u8], off: &mut usize| -> u8 {
        let v = body[*off];
        *off += 1;
        v
    };
    let _ = (vprops, fprops);
    // PLY binary: вершины обычно float32 x,y,z (+ опционально цвета u8).
    // Определяем stride по числу свойств вершины.
    let vstride = vprops.len();
    let xi = prop_index(vprops, "x").unwrap_or(0);
    let yi = prop_index(vprops, "y").unwrap_or(1);
    let zi = prop_index(vprops, "z").unwrap_or(2);
    for _ in 0..vcount {
        let mut vals: Vec<f32> = Vec::with_capacity(vstride);
        for _ in 0..vstride {
            // Эвристика: первые 3 свойства — float, остальные — u8.
            // Точнее: смотрим на позицию — x,y,z всегда float в наших файлах.
            // Для надёжности читаем как float, если индекс < 3, иначе u8.
            let idx = vals.len();
            if idx < 3 {
                vals.push(read_f32(body, &mut off));
            } else {
                vals.push(read_u8(body, &mut off) as f32);
            }
        }
        if vals.len() > zi {
            mesh.add_vertex(Vec3::new(vals[xi], vals[yi], vals[zi]));
        }
    }
    for _ in 0..fcount {
        let n = read_u8(body, &mut off) as usize;
        let mut idxs: Vec<u32> = Vec::with_capacity(n);
        for _ in 0..n {
            // Индексы обычно int32 (или uint8 для маленьких мешей — эвристика по размеру).
            // Пробуем int32 LE.
            let v = i32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]);
            off += 4;
            idxs.push(v as u32);
        }
        for k in 1..n.saturating_sub(1) {
            mesh.add_triangle(idxs[0], idxs[k], idxs[k + 1]);
        }
    }
    Ok(())
}

pub fn save(mesh: &Mesh, path: &Path) -> Result<(), String> {
    let text = to_ascii(mesh);
    std::fs::write(path, text)
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
}

pub fn to_ascii(mesh: &Mesh) -> String {
    let mut s = String::new();
    s.push_str("ply\nformat ascii 1.0\n");
    s.push_str(&format!("element vertex {}\n", mesh.vertex_count()));
    s.push_str("property float x\nproperty float y\nproperty float z\n");
    s.push_str(&format!("element face {}\n", mesh.triangle_count()));
    s.push_str("property list uchar int vertex_indices\nend_header\n");
    for p in &mesh.positions {
        s.push_str(&format!("{} {} {}\n", p.x, p.y, p.z));
    }
    for t in mesh.indices.chunks_exact(3) {
        s.push_str(&format!("3 {} {} {}\n", t[0], t[1], t[2]));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn roundtrip_ascii() {
        let m = primitive_box(1.0, 2.0, 3.0);
        let text = to_ascii(&m);
        let back = load_from_bytes(text.as_bytes(), "t").unwrap();
        assert_eq!(back.triangle_count(), 12);
        assert_eq!(back.vertex_count(), m.vertex_count());
    }

    #[test]
    fn rejects_garbage() {
        assert!(load_from_bytes(b"not a ply", "x").is_err());
    }
}
