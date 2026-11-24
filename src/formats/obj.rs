//! Wavefront OBJ: чтение и запись (позиции, нормали, UV, полигоны → треугольники).

use crate::math::{Color, Vec3};
use crate::mesh::Mesh;
use std::path::Path;

pub fn load(path: &Path) -> Result<Mesh, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
    parse(&text, &path.display().to_string())
}

pub fn parse(text: &str, source_name: &str) -> Result<Mesh, String> {
    let mut mesh = Mesh::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut has_vn = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(kind) = it.next() else { continue };
        match kind {
            "v" => {
                let x: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let y: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let z: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                mesh.add_vertex(Vec3::new(x, y, z));
            }
            "vn" => {
                has_vn = true;
                let x: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let y: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let z: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                normals.push(Vec3::new(x, y, z));
            }
            "f" => {
                // Форматы: v, v/vt, v//vn, v/vt/vn. Индексы 1-based, возможны отрицательные.
                let mut face: Vec<(u32, Option<u32>)> = Vec::new();
                for tok in it {
                    let parts: Vec<&str> = tok.split('/').collect();
                    let vi: i64 = parts[0].parse().unwrap_or(0);
                    let vi = if vi < 0 {
                        (mesh.positions.len() as i64 + vi) as u32
                    } else {
                        (vi - 1) as u32
                    };
                    let ni = if parts.len() >= 3 && !parts[2].is_empty() {
                        let n: i64 = parts[2].parse().unwrap_or(0);
                        let n = if n < 0 {
                            (normals.len() as i64 + n) as u32
                        } else {
                            (n - 1) as u32
                        };
                        Some(n)
                    } else {
                        None
                    };
                    face.push((vi, ni));
                }
                if face.len() < 3 {
                    continue;
                }
                // Триангуляция веером.
                for k in 1..face.len() - 1 {
                    let (a, na) = face[0];
                    let (b, nb) = face[k];
                    let (c, nc) = face[k + 1];
                    if let (Some(na), Some(nb), Some(nc)) = (na, nb, nc) {
                        if (na as usize) < normals.len()
                            && (nb as usize) < normals.len()
                            && (nc as usize) < normals.len()
                        {
                            mesh.normals[a as usize] = normals[na as usize];
                            mesh.normals[b as usize] = normals[nb as usize];
                            mesh.normals[c as usize] = normals[nc as usize];
                        }
                    }
                    mesh.add_triangle(a, b, c);
                }
            }
            _ => {}
        }
    }
    if mesh.is_empty() {
        return Err("OBJ не содержит треугольников".into());
    }
    if !has_vn {
        mesh.compute_smooth_normals();
    } else {
        for n in &mut mesh.normals {
            if n.len2() < 1e-12 {
                *n = Vec3::new(0.0, 0.0, 1.0);
            } else {
                *n = n.normalize();
            }
        }
    }
    mesh.name = source_name.to_string();
    Ok(mesh)
}

pub fn save(mesh: &Mesh, path: &Path) -> Result<(), String> {
    let text = to_string(mesh);
    std::fs::write(path, text)
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
}

pub fn to_string(mesh: &Mesh) -> String {
    let mut s = String::new();
    s.push_str(&format!("# ForgeCAD OBJ export: {}\n", mesh.name));
    for p in &mesh.positions {
        s.push_str(&format!("v {} {} {}\n", p.x, p.y, p.z));
    }
    for n in &mesh.normals {
        s.push_str(&format!("vn {} {} {}\n", n.x, n.y, n.z));
    }
    for t in mesh.indices.chunks_exact(3) {
        s.push_str(&format!(
            "f {}/{} {}/{} {}/{}\n",
            t[0] + 1, t[0] + 1,
            t[1] + 1, t[1] + 1,
            t[2] + 1, t[2] + 1
        ));
    }
    s
}

/// Простой цвет вершины из OBJ (если есть) — не используется, оставлен для API.
pub fn _unused(_c: Color) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn roundtrip() {
        let m = primitive_box(1.0, 1.0, 1.0);
        let text = to_string(&m);
        let back = parse(&text, "t").unwrap();
        assert_eq!(back.triangle_count(), 12);
        assert_eq!(back.vertex_count(), m.vertex_count());
    }

    #[test]
    fn parse_minimal() {
        let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let m = parse(text, "t").unwrap();
        assert_eq!(m.triangle_count(), 1);
    }

    #[test]
    fn parse_quad_fan() {
        let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        let m = parse(text, "t").unwrap();
        assert_eq!(m.triangle_count(), 2);
    }
}
