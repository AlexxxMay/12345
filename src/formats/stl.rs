//! STL: бинарный и ASCII. Чтение и запись.
//!
//! Бинарный STL: 80 байт заголовка, u32 число треугольников,
//! на треугольник: нормаль (3×f32), 3 вершины (9×f32), u16 атрибут.
//! ASCII STL: `solid` / `facet normal` / `outer loop` / `vertex` / `endloop`.

use crate::math::Vec3;
use crate::mesh::Mesh;
use std::path::Path;

pub fn load(path: &Path) -> Result<Mesh, String> {
    let data = std::fs::read(path)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
    load_from_bytes(&data, &path.display().to_string())
}

pub fn load_from_bytes(data: &[u8], source_name: &str) -> Result<Mesh, String> {
    // Эвристика: бинарный STL имеет размер 84 + 50*n.
    let is_binary = if data.len() >= 84 {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        data.len() == 84 + n * 50
    } else {
        false
    };
    let mut mesh = if is_binary {
        parse_binary(data)?
    } else {
        parse_ascii(data)?
    };
    mesh.name = source_name.to_string();
    Ok(mesh)
}

fn parse_binary(data: &[u8]) -> Result<Mesh, String> {
    if data.len() < 84 {
        return Err("Файл слишком короткий для бинарного STL".into());
    }
    let count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
    let expected = 84 + count * 50;
    if data.len() < expected {
        return Err(format!(
            "Бинарный STL повреждён: ожидалось {expected} байт, получено {}",
            data.len()
        ));
    }
    let mut mesh = Mesh::new();
    mesh.positions.reserve(count * 3);
    mesh.indices.reserve(count * 3);
    let mut off = 84usize;
    let read_f32 = |d: &[u8], o: usize| -> f32 {
        f32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
    };
    for _ in 0..count {
        // Нормаль (12 байт) пропускаем — пересчитаем.
        let base = mesh.positions.len() as u32;
        for v in 0..3 {
            let o = off + 12 + v * 12;
            mesh.add_vertex(Vec3::new(read_f32(data, o), read_f32(data, o + 4), read_f32(data, o + 8)));
        }
        mesh.add_triangle(base, base + 1, base + 2);
        off += 50;
    }
    mesh.compute_flat_normals();
    Ok(mesh)
}

fn parse_ascii(data: &[u8]) -> Result<Mesh, String> {
    let text = String::from_utf8_lossy(data);
    let mut mesh = Mesh::new();
    let mut verts: Vec<Vec3> = Vec::with_capacity(3);
    let mut found_solid = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("solid") {
            found_solid = true;
        } else if let Some(rest) = line.strip_prefix("vertex") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let x: f32 = parts[0].parse().unwrap_or(0.0);
                let y: f32 = parts[1].parse().unwrap_or(0.0);
                let z: f32 = parts[2].parse().unwrap_or(0.0);
                verts.push(Vec3::new(x, y, z));
            }
        } else if line.starts_with("endfacet") {
            if verts.len() == 3 {
                mesh.push_tri(verts[0], verts[1], verts[2]);
            }
            verts.clear();
        }
    }
    if !found_solid && mesh.is_empty() {
        return Err("Не удалось распознать STL (ни бинарный, ни ASCII)".into());
    }
    if mesh.is_empty() {
        return Err("STL не содержит треугольников".into());
    }
    mesh.compute_flat_normals();
    Ok(mesh)
}

pub fn save(mesh: &Mesh, path: &Path) -> Result<(), String> {
    let data = to_binary(mesh);
    std::fs::write(path, data)
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
}

pub fn to_binary(mesh: &Mesh) -> Vec<u8> {
    let n = mesh.triangle_count();
    let mut out = Vec::with_capacity(84 + n * 50);
    out.extend_from_slice(b"AlphaCad STL");
    out.resize(80, 0);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    for i in 0..n {
        let [a, b, c] = mesh.tri(i);
        let nrm = mesh.face_normal(i).normalize();
        for v in [nrm, a, b, c] {
            out.extend_from_slice(&v.x.to_le_bytes());
            out.extend_from_slice(&v.y.to_le_bytes());
            out.extend_from_slice(&v.z.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

pub fn to_ascii(mesh: &Mesh) -> String {
    let mut s = String::new();
    s.push_str(&format!("solid {}\n", mesh.name));
    for i in 0..mesh.triangle_count() {
        let n = mesh.face_normal(i).normalize();
        let [a, b, c] = mesh.tri(i);
        s.push_str(&format!("  facet normal {} {} {}\n", n.x, n.y, n.z));
        s.push_str("    outer loop\n");
        for v in [a, b, c] {
            s.push_str(&format!("      vertex {} {} {}\n", v.x, v.y, v.z));
        }
        s.push_str("    endloop\n  endfacet\n");
    }
    s.push_str("endsolid\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn roundtrip_binary() {
        let m = primitive_box(10.0, 20.0, 30.0);
        let bytes = to_binary(&m);
        let back = load_from_bytes(&bytes, "test").unwrap();
        assert_eq!(back.triangle_count(), m.triangle_count());
        assert!((back.volume() - m.volume()).abs() < 1e-2);
    }

    #[test]
    fn roundtrip_ascii() {
        let m = primitive_box(1.0, 2.0, 3.0);
        let text = to_ascii(&m);
        let back = load_from_bytes(text.as_bytes(), "test").unwrap();
        assert_eq!(back.triangle_count(), 12);
    }

    #[test]
    fn rejects_garbage() {
        assert!(load_from_bytes(b"hello world", "x").is_err());
    }
}
