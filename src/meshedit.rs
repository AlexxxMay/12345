//! Ручная правка сетки: топология по совпадающим позициям, выбор граней,
//! рёбер и вершин, перемещение и вытягивание граней.
//!
//! Меши в программе индексированные, но вершины на острых рёбрах часто
//! продублированы (у каждой грани параллелепипеда свои вершины с нормалями).
//! Поэтому топология строится по «группам» — вершинам с одинаковой позицией:
//! перемещение вершины двигает все её копии, и сетка не разрывается.

use crate::math::Vec3;
use crate::mesh::Mesh;
use std::collections::{HashMap, HashSet};

/// Топология меша по группам совпадающих вершин.
#[derive(Clone, Debug, Default)]
pub struct MeshTopo {
    /// Индекс вершины → номер группы.
    pub group: Vec<u32>,
    /// Группа → индексы вершин меша.
    pub groups: Vec<Vec<u32>>,
    /// Ребро (меньшая группа, большая группа) → треугольники.
    pub edge_tris: HashMap<(u32, u32), Vec<u32>>,
    /// Единичные нормали треугольников (нулевые — для вырожденных).
    pub tri_normals: Vec<Vec3>,
}

fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

impl MeshTopo {
    pub fn build(mesh: &Mesh) -> MeshTopo {
        let bb = mesh.aabb();
        let q = (bb.diagonal() * 1e-6).max(1e-9);
        let mut map: HashMap<(i64, i64, i64), u32> = HashMap::with_capacity(mesh.positions.len());
        let mut group = Vec::with_capacity(mesh.positions.len());
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for (i, p) in mesh.positions.iter().enumerate() {
            let key = ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
            let g = *map.entry(key).or_insert_with(|| {
                groups.push(Vec::new());
                (groups.len() - 1) as u32
            });
            groups[g as usize].push(i as u32);
            group.push(g);
        }
        let tri_n = mesh.triangle_count();
        let mut edge_tris: HashMap<(u32, u32), Vec<u32>> = HashMap::with_capacity(tri_n * 2);
        let mut tri_normals = Vec::with_capacity(tri_n);
        for t in 0..tri_n {
            let [a, b, c] = mesh.tri(t);
            let n = (b - a).cross(c - a);
            tri_normals.push(if n.len2() > 0.0 { n.normalize() } else { Vec3::zero() });
            let idx = mesh.tri_indices(t);
            for k in 0..3 {
                let ga = group[idx[k] as usize];
                let gb = group[idx[(k + 1) % 3] as usize];
                if ga != gb {
                    edge_tris.entry(ekey(ga, gb)).or_default().push(t as u32);
                }
            }
        }
        MeshTopo { group, groups, edge_tris, tri_normals }
    }

    /// Группы вершин треугольника.
    pub fn tri_groups(&self, mesh: &Mesh, t: usize) -> [u32; 3] {
        let i = mesh.tri_indices(t);
        [self.group[i[0] as usize], self.group[i[1] as usize], self.group[i[2] as usize]]
    }

    /// Грань — связная область треугольников, компланарных с `seed`
    /// (отклонение нормалей не больше `max_deg`).
    pub fn face_region(&self, mesh: &Mesh, seed: usize, max_deg: f32) -> Vec<u32> {
        let n0 = self.tri_normals.get(seed).copied().unwrap_or(Vec3::zero());
        let cos = max_deg.to_radians().cos();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut out = vec![seed as u32];
        seen.insert(seed as u32);
        let mut i = 0;
        while i < out.len() {
            let t = out[i] as usize;
            i += 1;
            let g = self.tri_groups(mesh, t);
            for k in 0..3 {
                let Some(list) = self.edge_tris.get(&ekey(g[k], g[(k + 1) % 3])) else { continue };
                for &o in list {
                    if seen.contains(&o) {
                        continue;
                    }
                    if self.tri_normals[o as usize].dot(n0) >= cos {
                        seen.insert(o);
                        out.push(o);
                    }
                }
            }
            if out.len() > 200_000 {
                break;
            }
        }
        out
    }

    /// Граничные рёбра области (в направлении обхода треугольников области),
    /// как пары групп.
    pub fn region_boundary(&self, mesh: &Mesh, tris: &[u32]) -> Vec<(u32, u32)> {
        let set: HashSet<u32> = tris.iter().copied().collect();
        let mut out = Vec::new();
        for &t in tris {
            let g = self.tri_groups(mesh, t as usize);
            for k in 0..3 {
                let (a, b) = (g[k], g[(k + 1) % 3]);
                if a == b {
                    continue;
                }
                let inside = self.edge_tris.get(&ekey(a, b)).map(|l| l.iter().filter(|o| set.contains(o)).count()).unwrap_or(0);
                if inside < 2 {
                    out.push((a, b));
                }
            }
        }
        out
    }

    /// Все индексы вершин меша, входящие в группы.
    pub fn verts_of_groups(&self, groups: impl IntoIterator<Item = u32>) -> Vec<u32> {
        let mut v: Vec<u32> = Vec::new();
        let mut seen = HashSet::new();
        for g in groups {
            if seen.insert(g) {
                v.extend_from_slice(&self.groups[g as usize]);
            }
        }
        v
    }

    /// Группы треугольников области.
    pub fn groups_of_tris(&self, mesh: &Mesh, tris: &[u32]) -> Vec<u32> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for &t in tris {
            for g in self.tri_groups(mesh, t as usize) {
                if seen.insert(g) {
                    out.push(g);
                }
            }
        }
        out
    }

    /// Позиция группы (по последней вершине: при вытягивании новые вершины
    /// добавляются в конец и именно они перемещаются).
    pub fn group_pos(&self, mesh: &Mesh, g: u32) -> Vec3 {
        let v = self.groups[g as usize].last().copied().unwrap_or(0);
        mesh.positions.get(v as usize).copied().unwrap_or_default()
    }
}

/// Сдвиг вершин меша.
pub fn move_vertices(mesh: &mut Mesh, verts: &[u32], delta: Vec3) {
    for &v in verts {
        if let Some(p) = mesh.positions.get_mut(v as usize) {
            *p += delta;
        }
    }
}

/// Вытягивание области треугольников: у области появляются собственные
/// вершины, а по её границе — боковые стенки (пока нулевой высоты).
/// Возвращает индексы новых вершин области — их и нужно сдвигать.
pub fn extrude_region(mesh: &mut Mesh, topo: &MeshTopo, tris: &[u32]) -> Vec<u32> {
    if tris.is_empty() {
        return Vec::new();
    }
    let boundary = topo.region_boundary(mesh, tris);
    let groups = topo.groups_of_tris(mesh, tris);
    let mut new_of: HashMap<u32, u32> = HashMap::with_capacity(groups.len());
    let has_normals = mesh.normals.len() == mesh.positions.len();
    for &g in &groups {
        let src = topo.groups[g as usize][0] as usize;
        let p = mesh.positions[src];
        mesh.positions.push(p);
        if has_normals {
            let n = mesh.normals[src];
            mesh.normals.push(n);
        }
        if let Some(c) = mesh.colors.as_mut() {
            let col = c.get(src).copied().unwrap_or_default();
            c.push(col);
        }
        new_of.insert(g, (mesh.positions.len() - 1) as u32);
    }
    // Треугольники области переходят на новые вершины.
    for &t in tris {
        for k in 0..3 {
            let vi = mesh.indices[t as usize * 3 + k];
            let g = topo.group[vi as usize];
            mesh.indices[t as usize * 3 + k] = new_of[&g];
        }
    }
    // Боковые стенки: старое ребро a→b (как в области) и новое b'→a'.
    for (a, b) in boundary {
        let (oa, ob) = (topo.groups[a as usize][0], topo.groups[b as usize][0]);
        let (na, nb) = (new_of[&a], new_of[&b]);
        mesh.indices.extend_from_slice(&[oa, ob, nb, oa, nb, na]);
    }
    let mut out: Vec<u32> = new_of.values().copied().collect();
    out.sort_unstable();
    out
}

/// Удаление треугольников нулевой площади (после вытягивания на 0).
pub fn drop_degenerate(mesh: &mut Mesh) {
    let mut idx = Vec::with_capacity(mesh.indices.len());
    for t in 0..mesh.triangle_count() {
        let [a, b, c] = mesh.tri(t);
        if (b - a).cross(c - a).len2() > 1e-14 {
            idx.extend_from_slice(&mesh.tri_indices(t));
        }
    }
    mesh.indices = idx;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    fn top_face(m: &Mesh, topo: &MeshTopo) -> Vec<u32> {
        let seed = (0..m.triangle_count()).find(|&t| topo.tri_normals[t].z > 0.99).unwrap();
        topo.face_region(m, seed, 1.0)
    }

    #[test]
    fn box_topology_and_faces() {
        let m = primitive_box(10.0, 10.0, 10.0);
        let topo = MeshTopo::build(&m);
        assert_eq!(topo.groups.len(), 8, "8 углов параллелепипеда");
        let face = top_face(&m, &topo);
        assert_eq!(face.len(), 2);
        let b = topo.region_boundary(&m, &face);
        assert_eq!(b.len(), 4, "граница грани — 4 ребра, без диагонали");
        assert_eq!(topo.groups_of_tris(&m, &face).len(), 4);
    }

    #[test]
    fn move_face_stretches_box() {
        let mut m = primitive_box(10.0, 10.0, 10.0);
        let topo = MeshTopo::build(&m);
        let face = top_face(&m, &topo);
        let verts = topo.verts_of_groups(topo.groups_of_tris(&m, &face));
        let v0 = m.volume().abs();
        move_vertices(&mut m, &verts, Vec3::new(0.0, 0.0, 5.0));
        assert!((m.volume().abs() - v0 * 1.5).abs() < 1e-2, "{}", m.volume());
        assert!((m.aabb().size().z - 15.0).abs() < 1e-4);
    }

    #[test]
    fn extrude_face_keeps_mesh_closed() {
        let mut m = primitive_box(10.0, 10.0, 10.0);
        let topo = MeshTopo::build(&m);
        let face = top_face(&m, &topo);
        let tris0 = m.triangle_count();
        let new_verts = extrude_region(&mut m, &topo, &face);
        assert_eq!(new_verts.len(), 4);
        assert_eq!(m.triangle_count(), tris0 + 8);
        move_vertices(&mut m, &new_verts, Vec3::new(0.0, 0.0, 10.0));
        assert!((m.volume().abs() - 2000.0).abs() < 1e-2, "{}", m.volume());
        let mut w = m.clone();
        w.weld();
        assert!(w.is_closed(), "после вытягивания сетка замкнута");
        assert!(w.volume() > 0.0, "нормали наружу");
    }

    #[test]
    fn drop_degenerate_after_zero_extrude() {
        let mut m = primitive_box(10.0, 10.0, 10.0);
        let topo = MeshTopo::build(&m);
        let face = top_face(&m, &topo);
        let tris0 = m.triangle_count();
        extrude_region(&mut m, &topo, &face);
        drop_degenerate(&mut m);
        assert_eq!(m.triangle_count(), tris0);
    }
}
