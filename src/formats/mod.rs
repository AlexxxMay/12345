//! Чтение и запись 3D-форматов: STL, STEP, 3MF, OBJ, PLY, проект AlphaCad.
//!
//! Поддерживается:
//! - **STL** (бинарный и ASCII) — чтение и запись.
//! - **STEP AP214/AP203** — чтение с тесселяцией в меш, запись AP214.
//! - **3MF** — чтение (сетки, компоненты, сборка, единицы, цвета) и запись.
//! - **OBJ** (Wavefront) — чтение и запись.
//! - **PLY** (ASCII и binary) — чтение и запись.
//! - **.acad** — собственный формат проекта (JSON).

pub mod obj;
pub mod ply;
pub mod project;
pub mod step;
pub mod stl;
pub mod threemf;
pub mod zip;

use crate::mesh::Mesh;
use std::path::Path;

/// Поддерживаемый формат модели.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFormat {
    Stl,
    Step,
    ThreeMf,
    Obj,
    Ply,
    Project,
}

impl ModelFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "stl" => Some(ModelFormat::Stl),
            "step" | "stp" => Some(ModelFormat::Step),
            "3mf" => Some(ModelFormat::ThreeMf),
            "obj" => Some(ModelFormat::Obj),
            "ply" => Some(ModelFormat::Ply),
            "acad" | "forge" => Some(ModelFormat::Project),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ModelFormat::Stl => "stl",
            ModelFormat::Step => "step",
            ModelFormat::ThreeMf => "3mf",
            ModelFormat::Obj => "obj",
            ModelFormat::Ply => "ply",
            ModelFormat::Project => "acad",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ModelFormat::Stl => "STL",
            ModelFormat::Step => "STEP",
            ModelFormat::ThreeMf => "3MF",
            ModelFormat::Obj => "OBJ",
            ModelFormat::Ply => "PLY",
            ModelFormat::Project => "AlphaCad Project",
        }
    }

    /// Фильтр для диалогов открытия (имя; расширения).
    pub fn dialog_filter(&self) -> (&'static str, &'static [&'static str]) {
        match self {
            ModelFormat::Stl => ("STL (*.stl)", &["stl"]),
            ModelFormat::Step => ("STEP (*.step;*.stp)", &["step", "stp"]),
            ModelFormat::ThreeMf => ("3MF (*.3mf)", &["3mf"]),
            ModelFormat::Obj => ("OBJ (*.obj)", &["obj"]),
            ModelFormat::Ply => ("PLY (*.ply)", &["ply"]),
            ModelFormat::Project => ("AlphaCad (*.acad)", &["acad"]),
        }
    }
}

/// Определить формат по расширению файла.
pub fn format_of_path(path: &Path) -> Option<ModelFormat> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(ModelFormat::from_extension)
}

/// Загрузить модель из файла (меш). Для .acad возвращает ошибку —
/// проекты открываются через [`project::load_project`].
pub fn load_mesh(path: &Path) -> Result<Mesh, String> {
    let fmt = format_of_path(path)
        .ok_or_else(|| format!("Неподдерживаемое расширение файла: {:?}", path.extension()))?;
    load_mesh_as(path, fmt)
}

pub fn load_mesh_as(path: &Path, fmt: ModelFormat) -> Result<Mesh, String> {
    match fmt {
        ModelFormat::Stl => stl::load(path),
        ModelFormat::Step => step::load(path),
        ModelFormat::ThreeMf => threemf::load(path),
        ModelFormat::Obj => obj::load(path),
        ModelFormat::Ply => ply::load(path),
        ModelFormat::Project => {
            Err("Файл проекта .acad открывается как проект, а не как модель".into())
        }
    }
}

/// Сохранить меш в файл по расширению.
pub fn save_mesh(mesh: &Mesh, path: &Path) -> Result<(), String> {
    let fmt = format_of_path(path)
        .ok_or_else(|| format!("Неподдерживаемое расширение файла: {:?}", path.extension()))?;
    save_mesh_as(mesh, path, fmt)
}

pub fn save_mesh_as(mesh: &Mesh, path: &Path, fmt: ModelFormat) -> Result<(), String> {
    match fmt {
        ModelFormat::Stl => stl::save(mesh, path),
        ModelFormat::Step => step::save(mesh, path),
        ModelFormat::ThreeMf => threemf::save(mesh, path),
        ModelFormat::Obj => obj::save(mesh, path),
        ModelFormat::Ply => ply::save(mesh, path),
        ModelFormat::Project => {
            Err("Проект сохраняется через меню «Сохранить проект»".into())
        }
    }
}

/// Информация о модели (для CLI и UI).
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub vertices: usize,
    pub triangles: usize,
    pub aabb: crate::math::Aabb,
    pub volume: f32,
    pub area: f32,
    pub center_of_mass: crate::math::Vec3,
    pub closed: bool,
    pub issues: Vec<String>,
}

impl ModelInfo {
    pub fn from_mesh(mesh: &Mesh) -> Self {
        Self {
            name: mesh.name.clone(),
            vertices: mesh.vertex_count(),
            triangles: mesh.triangle_count(),
            aabb: mesh.aabb(),
            volume: mesh.volume(),
            area: mesh.surface_area(),
            center_of_mass: mesh.center_of_mass(),
            closed: mesh.is_closed(),
            issues: mesh.validate(),
        }
    }

    pub fn summary(&self) -> String {
        let s = self.aabb.size();
        let mut out = format!(
            "Модель: {}\n\
             Вершин: {}\n\
             Треугольников: {}\n\
             Габариты (мм): {:.3} × {:.3} × {:.3}\n\
             Объём: {:.3} мм³\n\
             Площадь: {:.3} мм²\n\
             Центр масс: ({:.3}, {:.3}, {:.3})\n\
             Замкнутая: {}",
            self.name,
            self.vertices,
            self.triangles,
            s.x, s.y, s.z,
            self.volume,
            self.area,
            self.center_of_mass.x, self.center_of_mass.y, self.center_of_mass.z,
            if self.closed { "да" } else { "нет" },
        );
        if !self.issues.is_empty() {
            out.push_str("\nПроблемы:\n");
            for i in &self.issues {
                out.push_str(&format!("  - {i}\n"));
            }
        }
        out
    }
}
