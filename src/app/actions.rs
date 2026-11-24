//! Команды приложения: файлы, создание и изменение тел, вид.

use super::dialogs::{extrude_mesh, mirrored_world, Dialog};
use super::view::ViewDir;
use super::{t, NavMode, State, Tab, Tool};
use crate::csg::{self, BoolOp};
use crate::formats::{self, ModelFormat};
use crate::math::{Mat4, Vec3};
use crate::mesh::{Mesh, Primitive};
use crate::renderer::software::{Camera, DisplayMode};
use crate::scene::Body;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    StlBinary,
    StlAscii,
    Step,
    ThreeMf,
    Obj,
    Ply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimKind {
    Box,
    Cylinder,
    Sphere,
    Torus,
    Cone,
    Tube,
    Wedge,
    Pyramid,
    Capsule,
}

impl PrimKind {
    pub fn default_primitive(self) -> Primitive {
        match self {
            PrimKind::Box => Primitive::Box { w: 20.0, h: 20.0, d: 20.0 },
            PrimKind::Cylinder => Primitive::Cylinder { radius: 10.0, height: 20.0, segments: 48 },
            PrimKind::Sphere => Primitive::Sphere { radius: 10.0, segments: 48, rings: 24 },
            PrimKind::Torus => Primitive::Torus { major_radius: 15.0, minor_radius: 4.0, major_segments: 48, minor_segments: 24 },
            PrimKind::Cone => Primitive::Cone { radius_bottom: 10.0, radius_top: 0.0, height: 20.0, segments: 48 },
            PrimKind::Tube => Primitive::Tube { radius_outer: 10.0, radius_inner: 7.0, height: 20.0, segments: 48 },
            PrimKind::Wedge => Primitive::Wedge { w: 20.0, h: 10.0, d: 20.0 },
            PrimKind::Pyramid => Primitive::Pyramid { base: 20.0, height: 20.0, segments: 4 },
            PrimKind::Capsule => Primitive::Capsule { radius: 6.0, height: 24.0, segments: 32 },
        }
    }
}

/// Действие пользователя (меню, кнопки, горячие клавиши).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    New,
    /// Новое окно (отдельный процесс) с пустым рабочим пространством.
    NewWindow,
    Open,
    Import,
    Save,
    SaveAs,
    Export(ExportKind),
    Screenshot,
    OpenRecent(usize),
    Quit,
    Undo,
    Redo,
    Help,
    Settings,
    ToggleLang,
    CreatePrimitive(PrimKind),
    SetTool(Tool),
    Combine(BoolOp),
    Mirror,
    Duplicate,
    Delete,
    NewSketch,
    EditSketch,
    Extrude,
    SketchUndo,
    FinishSketch,
    CancelSketch,
    Weld,
    FixOrientation,
    FlipNormals,
    Reduce,
    SetShading(u8),
    DropToFloor,
    CenterOrigin,
    BakeTransform,
    ShowInfo,
    SetNav(NavMode),
    Fit,
    ViewHome,
    DisplayMenu,
    ToggleGrid,
    ToggleOrtho,
    SetMode(DisplayMode),
    CycleMode,
    ToggleBackground,
    SetView(ViewDir),
    FocusSelected,
    HideSelected,
    RenameSelected,
    Escape,
}

const MODEL_EXTS: &[&str] = &["stl", "step", "stp", "3mf", "obj", "ply", "acad", "forge"];

impl State {
    fn view_aspect(&self) -> f32 {
        let v = self.layout.view;
        if v.h > 1.0 {
            v.w / v.h
        } else {
            1.6
        }
    }

    fn fit_bounds(&mut self, b: crate::math::Aabb, animate: bool) {
        if b.is_empty() {
            return;
        }
        let mut to = self.cam;
        to.fit(&b, self.view_aspect());
        if animate {
            self.animate_camera(to);
        } else {
            self.cam = to;
        }
        self.invalidate_view();
    }

    fn selected_id(&self) -> Option<u64> {
        self.scene.selected.filter(|id| self.scene.body(*id).is_some())
    }

    fn selected_solid(&self) -> Option<u64> {
        self.selected_id().filter(|id| self.scene.body(*id).map(|b| !b.mesh.is_empty()).unwrap_or(false))
    }

    /// Изменение меша выбранного тела (с записью в историю).
    fn modify_selected_mesh(&mut self, label: &str, f: impl FnOnce(&mut Mesh) -> String) {
        let Some(id) = self.selected_solid() else {
            self.toast(t("Выберите тело", "Select a body"), true);
            return;
        };
        let Some(b) = self.scene.body(id) else { return };
        let mut body = b.clone();
        let msg = f(&mut body.mesh);
        body.primitive = None;
        self.scene.group(label, |sc| sc.replace_body(body));
        if !msg.is_empty() {
            self.toast(msg, false);
        }
        self.invalidate_view();
    }

    pub fn run_action(&mut self, a: Action) {
        self.menu = None;
        match a {
            Action::New => {
                if self.is_dirty() {
                    self.dialog = Some(Dialog::ConfirmDiscard { then: Action::New });
                } else {
                    self.reset_document();
                }
            }
            Action::NewWindow => {
                if let Err(e) = spawn_new_window() {
                    self.toast(&format!("{}: {e}", t("Не удалось открыть новое окно", "Could not open a new window")), true);
                }
            }
            Action::Open => {
                if self.is_dirty() {
                    self.dialog = Some(Dialog::ConfirmDiscard { then: Action::Open });
                    return;
                }
                let files = crate::platform::open_file_dialog(t("Открыть", "Open"), &self.open_filters(), false);
                if !files.is_empty() {
                    self.open_files(files, true);
                }
            }
            Action::Import => {
                let files = crate::platform::open_file_dialog(t("Импорт моделей", "Import models"), &self.open_filters(), true);
                if !files.is_empty() {
                    self.open_files(files, false);
                }
            }
            Action::OpenRecent(i) => {
                let Some(p) = self.cfg.recent.get(i).cloned() else { return };
                let path = PathBuf::from(&p);
                let is_project = formats::format_of_path(&path) == Some(ModelFormat::Project);
                if is_project && self.is_dirty() {
                    self.dialog = Some(Dialog::ConfirmDiscard { then: a });
                    return;
                }
                if !path.exists() {
                    self.cfg.recent.retain(|r| r != &p);
                    self.toast(format!("{}: {p}", t("Файл не найден", "File not found")), true);
                    return;
                }
                self.open_files(vec![path], true);
            }
            Action::Save => {
                self.save_project(false);
            }
            Action::SaveAs => {
                self.save_project(true);
            }
            Action::Export(kind) => self.export(kind),
            Action::Screenshot => self.screenshot(),
            Action::Quit => self.request_quit(),
            Action::Undo => {
                if self.sketch.is_some() {
                    self.run_action(Action::SketchUndo);
                } else if self.scene.can_undo() {
                    self.scene.undo();
                    self.invalidate_view();
                }
            }
            Action::Redo => {
                if self.sketch.is_none() && self.scene.can_redo() {
                    self.scene.redo();
                    self.invalidate_view();
                }
            }
            Action::Help => self.dialog = Some(Dialog::Help),
            Action::Settings => self.dialog = Some(Dialog::Settings),
            Action::ToggleLang => {
                self.cfg.lang_en = !self.cfg.lang_en;
                super::set_lang_en(self.cfg.lang_en);
                self.cfg.save();
            }
            Action::CreatePrimitive(k) => {
                if self.sketch.is_some() {
                    return;
                }
                self.tab = Tab::Solid;
                self.dialog = Some(Dialog::Primitive { prim: k.default_primitive() });
                self.invalidate_view();
            }
            Action::SetTool(tool) => {
                if tool.is_sketch() && self.sketch.is_none() {
                    return;
                }
                if let Some(sess) = &mut self.sketch {
                    sess.anchor = None;
                    sess.chain_start = None;
                }
                self.tool = tool;
                self.measure = Default::default();
                self.invalidate_view();
            }
            Action::Combine(op) => {
                let solids: Vec<u64> = self.scene.bodies.iter().filter(|b| !b.mesh.is_empty()).map(|b| b.id).collect();
                if solids.len() < 2 {
                    self.toast(t("Нужно минимум два тела", "At least two bodies are required"), true);
                    return;
                }
                let target = self.selected_solid().unwrap_or(solids[0]);
                let tool = solids.iter().rev().copied().find(|i| *i != target).unwrap_or(0);
                let op = match op {
                    BoolOp::Union => 0,
                    BoolOp::Subtract => 1,
                    BoolOp::Intersect => 2,
                };
                self.dialog = Some(Dialog::Combine { target, tool, op, keep: false });
            }
            Action::Mirror => {
                if self.selected_solid().is_none() {
                    self.toast(t("Выберите тело", "Select a body"), true);
                    return;
                }
                self.dialog = Some(Dialog::Mirror { plane: 0, copy: true });
                self.invalidate_view();
            }
            Action::Duplicate => {
                let Some(id) = self.selected_id() else { return };
                let Some(b) = self.scene.body(id) else { return };
                let mut nb = b.clone();
                let size = b.world_aabb().size();
                nb.name = self.unique_name(&base_name(&b.name));
                if !b.mesh.is_empty() {
                    nb.transform = Mat4::translation(Vec3::new(size.x * 1.2 + 2.0, 0.0, 0.0)) * nb.transform;
                }
                self.scene.add_body(nb);
                self.invalidate_view();
            }
            Action::Delete => {
                if let Some(id) = self.selected_id() {
                    self.scene.remove_body(id);
                    self.scene.selected = None;
                    self.invalidate_view();
                }
            }
            Action::NewSketch => {
                if self.sketch.is_none() {
                    self.tab = Tab::Sketch;
                    self.dialog = Some(Dialog::SketchPlane);
                }
            }
            Action::EditSketch => {
                let Some(id) = self.selected_id() else { return };
                let Some(plane) = self.scene.body(id).and_then(|b| b.sketch.as_ref()).map(|s| s.plane) else { return };
                self.begin_sketch(plane, Some(id));
            }
            Action::Extrude => {
                if self.sketch.is_some() {
                    self.finish_sketch();
                }
                let sel_sketch = self.selected_id().filter(|id| self.scene.body(*id).map(|b| b.sketch.is_some()).unwrap_or(false));
                let sketch = sel_sketch.or_else(|| self.scene.bodies.iter().rev().find(|b| b.sketch.is_some()).map(|b| b.id));
                let Some(sketch) = sketch else {
                    self.toast(t("Сначала создайте эскиз с замкнутым контуром", "Create a sketch with a closed profile first"), true);
                    return;
                };
                self.tab = Tab::Solid;
                self.dialog = Some(Dialog::Extrude { sketch, distance: 10.0, symmetric: false, op: 0, target: 0 });
                self.invalidate_view();
            }
            Action::SketchUndo => {
                if let Some(sess) = &mut self.sketch {
                    if sess.anchor.is_some() {
                        sess.anchor = None;
                        sess.chain_start = None;
                    } else {
                        sess.sketch.entities.pop();
                    }
                    self.invalidate_view();
                }
            }
            Action::FinishSketch => self.finish_sketch(),
            Action::CancelSketch => {
                if let Some(sess) = self.sketch.take() {
                    self.animate_camera(sess.prev_camera);
                }
                self.tool = Tool::Select;
                self.tab = Tab::Solid;
                self.invalidate_view();
            }
            Action::Weld => self.modify_selected_mesh("weld", |m| {
                let n = m.weld();
                m.compute_smooth_normals();
                format!("{}: {n}", t("Объединено вершин", "Vertices merged"))
            }),
            Action::FixOrientation => self.modify_selected_mesh("fix", |m| {
                m.weld();
                if m.is_closed() {
                    m.ensure_outward();
                    m.compute_smooth_normals();
                    t("Нормали направлены наружу", "Normals now point outward").to_string()
                } else {
                    m.compute_smooth_normals();
                    format!("{} ({})", t("Сетка не замкнута — ориентация не изменена", "Mesh is open — orientation unchanged"), m.boundary_edge_count())
                }
            }),
            Action::FlipNormals => self.modify_selected_mesh("flip", |m| {
                m.flip_winding();
                m.compute_smooth_normals();
                String::new()
            }),
            Action::Reduce => {
                if let Some(id) = self.selected_solid() {
                    self.dialog = Some(Dialog::Reduce { id, percent: 50.0 });
                } else {
                    self.toast(t("Выберите тело", "Select a body"), true);
                }
            }
            Action::SetShading(v) => {
                if let Some(id) = self.selected_solid() {
                    if let Some(b) = self.scene.body(id) {
                        let mut m = b.material.clone();
                        m.shading = v;
                        self.scene.set_material(id, m);
                        self.invalidate_view();
                    }
                }
            }
            Action::DropToFloor | Action::CenterOrigin => {
                let Some(id) = self.selected_solid() else { return };
                let Some(b) = self.scene.body(id) else { return };
                let bb = b.world_aabb();
                let c = bb.center();
                let d = if a == Action::DropToFloor { Vec3::new(0.0, 0.0, -bb.min.z) } else { Vec3::new(-c.x, -c.y, -bb.min.z) };
                let tf = Mat4::translation(d) * b.transform;
                self.scene.set_transform(id, tf);
                self.invalidate_view();
            }
            Action::BakeTransform => {
                let Some(id) = self.selected_solid() else { return };
                let Some(b) = self.scene.body(id) else { return };
                let mut nb = b.clone();
                let tf = nb.transform;
                nb.mesh.transform(tf);
                nb.mesh.compute_smooth_normals();
                nb.transform = Mat4::identity();
                nb.primitive = None;
                self.scene.replace_body(nb);
                self.invalidate_view();
            }
            Action::ShowInfo => {
                if let Some(id) = self.selected_solid() {
                    self.dialog = Some(Dialog::Info { id });
                } else {
                    self.toast(t("Выберите тело", "Select a body"), true);
                }
            }
            Action::SetNav(m) => {
                self.nav_mode = if self.nav_mode == m { NavMode::Default } else { m };
            }
            Action::Fit => {
                let b = self.scene_bounds();
                if b.is_empty() {
                    self.run_action(Action::ViewHome);
                } else {
                    self.fit_bounds(b, true);
                }
            }
            Action::ViewHome => {
                let d = Camera::default();
                let mut to = self.cam;
                to.yaw = d.yaw;
                to.pitch = d.pitch;
                let b = self.scene_bounds();
                if b.is_empty() {
                    to.target = d.target;
                    to.distance = d.distance;
                } else {
                    to.fit(&b, self.view_aspect());
                }
                self.animate_camera(to);
            }
            Action::DisplayMenu => {}
            Action::ToggleGrid => {
                self.cfg.show_grid = !self.cfg.show_grid;
                self.invalidate_view();
            }
            Action::ToggleOrtho => {
                self.cam.ortho = !self.cam.ortho;
                self.invalidate_view();
            }
            Action::SetMode(m) => {
                self.cfg.mode = m;
                self.invalidate_view();
            }
            Action::CycleMode => {
                self.cfg.mode = match self.cfg.mode {
                    DisplayMode::ShadedEdges => DisplayMode::Shaded,
                    DisplayMode::Shaded => DisplayMode::Wireframe,
                    DisplayMode::Wireframe => DisplayMode::ShadedEdges,
                };
                self.invalidate_view();
            }
            Action::ToggleBackground => {
                self.cfg.dark_bg = !self.cfg.dark_bg;
                self.invalidate_view();
            }
            Action::SetView(dir) => {
                let mut to = self.cam;
                to.set_view_dir(dir.vector());
                self.animate_camera(to);
            }
            Action::FocusSelected => {
                if let Some(b) = self.selected_id().and_then(|id| self.scene.body(id)) {
                    let bb = b.world_aabb();
                    self.fit_bounds(bb, true);
                }
            }
            Action::HideSelected => {
                if let Some(id) = self.selected_id() {
                    let vis = self.scene.body(id).map(|b| b.visible).unwrap_or(true);
                    self.scene.set_visible(id, !vis);
                    self.invalidate_view();
                }
            }
            Action::RenameSelected => {
                if let Some(b) = self.selected_id().and_then(|id| self.scene.body(id)) {
                    self.rename = Some((b.id, b.name.clone()));
                    self.browser_open = true;
                }
            }
            Action::Escape => self.escape(),
        }
    }

    fn open_filters(&self) -> Vec<crate::platform::Filter<'static>> {
        vec![
            (t("Все модели", "All models"), MODEL_EXTS),
            ("STL", &["stl"]),
            ("STEP", &["step", "stp"]),
            ("3MF", &["3mf"]),
            ("OBJ", &["obj"]),
            ("PLY", &["ply"]),
            (t("Проект AlphaCad", "AlphaCad project"), &["acad"]),
        ]
    }

    /// Открытие файлов. `open` — проект заменяет текущий документ и вид вписывается.
    pub fn open_files(&mut self, files: Vec<PathBuf>, open: bool) {
        let mut added = 0usize;
        let mut first_err: Option<String> = None;
        let started = std::time::Instant::now();
        let mut opened_project = false;
        for path in files {
            let fmt = formats::format_of_path(&path);
            match fmt {
                Some(ModelFormat::Project) => match formats::project::load_project(&path) {
                    Ok(scene) => {
                        if open || self.scene.bodies.is_empty() {
                            self.reset_document();
                            self.scene = scene;
                            self.project_path = Some(path.clone());
                            self.saved_revision = self.scene.revision;
                            opened_project = true;
                        } else {
                            let bodies = scene.bodies;
                            self.scene.group("import", |sc| {
                                for b in bodies {
                                    sc.add_body(b);
                                }
                            });
                        }
                        self.cfg.add_recent(&path);
                        added += 1;
                    }
                    Err(e) => {
                        first_err.get_or_insert(format!("{}: {e}", file_name(&path)));
                    }
                },
                Some(_) => match formats::load_mesh(&path) {
                    Ok(mut mesh) => {
                        if mesh.normals.len() != mesh.positions.len() {
                            mesh.compute_smooth_normals();
                        }
                        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Body".into());
                        let mut body = Body::new(stem, mesh);
                        body.source_path = Some(path.to_string_lossy().to_string());
                        self.scene.add_body(body);
                        self.cfg.add_recent(&path);
                        added += 1;
                    }
                    Err(e) => {
                        first_err.get_or_insert(format!("{}: {e}", file_name(&path)));
                    }
                },
                None => {
                    first_err.get_or_insert(format!("{}: {}", file_name(&path), t("неподдерживаемый формат", "unsupported format")));
                }
            }
        }
        if let Some(e) = first_err {
            self.toast(e, true);
        }
        if added > 0 {
            let ms = started.elapsed().as_millis();
            if opened_project {
                self.toast(format!("{} ({ms} ms)", t("Проект открыт", "Project opened")), false);
            } else {
                let tris: usize = self.scene.selected_body().map(|b| b.mesh.triangle_count()).unwrap_or(0);
                self.toast(format!("{}: {added} · {} {} ({ms} ms)", t("Загружено", "Loaded"), super::view::fmt_count(tris), t("треуг.", "tris")), false);
            }
            self.cfg.save();
            let b = self.scene_bounds();
            self.fit_bounds(b, false);
        }
        self.invalidate_view();
    }

    /// Сохранение проекта. Возвращает `true` при успехе.
    pub fn save_project(&mut self, save_as: bool) -> bool {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let def = self
                    .project_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("{}.acad", t("Проект", "Project")));
                let Some(mut p) = crate::platform::save_file_dialog(t("Сохранить проект", "Save project"), &def, &[(t("Проект AlphaCad", "AlphaCad project"), &["acad"])]) else {
                    return false;
                };
                if p.extension().is_none() {
                    p.set_extension("acad");
                }
                p
            }
        };
        match formats::project::save_project(&self.scene, &path) {
            Ok(()) => {
                self.project_path = Some(path.clone());
                self.saved_revision = self.scene.revision;
                self.cfg.add_recent(&path);
                self.cfg.save();
                self.toast(format!("{}: {}", t("Сохранено", "Saved"), file_name(&path)), false);
                true
            }
            Err(e) => {
                self.toast(format!("{}: {e}", t("Ошибка сохранения", "Save failed")), true);
                false
            }
        }
    }

    /// Меш для экспорта: выбранное тело или все видимые тела, в мировых координатах.
    pub fn export_mesh(&self) -> Option<(Mesh, String)> {
        let bodies: Vec<&Body> = match self.selected_solid().and_then(|id| self.scene.body(id)) {
            Some(b) => vec![b],
            None => self.scene.bodies.iter().filter(|b| b.visible && !b.mesh.is_empty()).collect(),
        };
        if bodies.is_empty() {
            return None;
        }
        let mut out = Mesh::new();
        for b in &bodies {
            let mut m = b.mesh.clone();
            m.transform(b.transform);
            out.merge(&m);
        }
        let name = if bodies.len() == 1 {
            bodies[0].name.clone()
        } else {
            self.project_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| t("Модель", "Model").to_string())
        };
        out.name = name.clone();
        Some((out, name))
    }

    fn export(&mut self, kind: ExportKind) {
        let Some((mesh, name)) = self.export_mesh() else {
            self.toast(t("Нет тел для экспорта", "Nothing to export"), true);
            return;
        };
        let (ext, label): (&'static str, &'static str) = match kind {
            ExportKind::StlBinary | ExportKind::StlAscii => ("stl", "STL"),
            ExportKind::Step => ("step", "STEP"),
            ExportKind::ThreeMf => ("3mf", "3MF"),
            ExportKind::Obj => ("obj", "OBJ"),
            ExportKind::Ply => ("ply", "PLY"),
        };
        let exts: &'static [&'static str] = match kind {
            ExportKind::Step => &["step", "stp"],
            ExportKind::ThreeMf => &["3mf"],
            ExportKind::StlBinary | ExportKind::StlAscii => &["stl"],
            ExportKind::Obj => &["obj"],
            ExportKind::Ply => &["ply"],
        };
        let def = format!("{}.{ext}", sanitize(&name));
        let Some(mut path) = crate::platform::save_file_dialog(t("Экспорт", "Export"), &def, &[(label, exts)]) else { return };
        if path.extension().is_none() {
            path.set_extension(ext);
        }
        match write_export(&mesh, &path, kind) {
            Ok(()) => self.toast(format!("{}: {}", t("Экспортировано", "Exported"), file_name(&path)), false),
            Err(e) => self.toast(format!("{}: {e}", t("Ошибка экспорта", "Export failed")), true),
        }
    }

    fn screenshot(&mut self) {
        let name = format!("{}.png", t("Снимок", "Screenshot"));
        let Some(mut path) = crate::platform::save_file_dialog(t("Сохранить снимок", "Save screenshot"), &name, &[("PNG", &["png"])]) else { return };
        if path.extension().is_none() {
            path.set_extension("png");
        }
        let fb = &self.view_fb;
        let png = crate::renderer::canvas::encode_png(fb.width, fb.height, &fb.to_rgb8());
        match std::fs::write(&path, png) {
            Ok(()) => self.toast(format!("{}: {}", t("Снимок сохранён", "Screenshot saved"), file_name(&path)), false),
            Err(e) => self.toast(e.to_string(), true),
        }
    }

    /// Завершение эскиза: создание или обновление тела-эскиза.
    pub fn finish_sketch(&mut self) {
        let Some(sess) = self.sketch.take() else { return };
        self.tool = Tool::Select;
        self.tab = Tab::Solid;
        let empty = sess.sketch.entities.is_empty();
        match sess.body_id.and_then(|id| self.scene.body(id).cloned()) {
            Some(mut b) => {
                b.sketch = Some(sess.sketch);
                self.scene.replace_body(b);
            }
            None if !empty => {
                let name = sess.sketch.name.clone();
                let mut b = Body::new(name, Mesh::new());
                b.sketch = Some(sess.sketch);
                self.scene.add_body(b);
            }
            None => {}
        }
        self.animate_camera(sess.prev_camera);
        if !empty {
            self.toast(t("Эскиз готов — нажмите «Выдавить» (E)", "Sketch finished — press Extrude (E)"), false);
        }
        self.invalidate_view();
    }

    /// Размещение нового тела: на плоскости XY, справа от уже существующих тел.
    pub fn placement_for(&self, mesh: &Mesh) -> Mat4 {
        let bb = mesh.aabb();
        let mut t = Vec3::new(-(bb.min.x + bb.max.x) * 0.5, -(bb.min.y + bb.max.y) * 0.5, -bb.min.z);
        let mut scene = crate::math::Aabb::empty();
        for b in &self.scene.bodies {
            if b.visible && !b.mesh.is_empty() {
                scene.union(&b.world_aabb());
            }
        }
        if !scene.is_empty() {
            let gap = (bb.size().x.max(bb.size().y) * 0.25).max(5.0);
            t.x = scene.max.x + gap - bb.min.x;
            t.y = scene.center().y - (bb.min.y + bb.max.y) * 0.5;
        }
        Mat4::translation(t)
    }

    pub fn create_primitive(&mut self, prim: Primitive) {
        let mesh = prim.build();
        let tf = self.placement_for(&mesh);
        let mut b = Body::new(self.unique_name(State::primitive_name(&prim)), mesh);
        b.primitive = Some(prim);
        b.transform = tf;
        self.scene.add_body(b);
        self.tool = Tool::Select;
        self.invalidate_view();
    }

    /// Выдавливание эскиза. `op`: 0 — новое тело, 1 — соединить, 2 — вырезать, 3 — пересечь.
    pub fn do_extrude(&mut self, sketch: u64, distance: f32, symmetric: bool, op: usize, target: u64) -> bool {
        let Some((mut m, tf)) = extrude_mesh(self, sketch, distance, symmetric) else {
            self.toast(t("Нет замкнутых профилей", "No closed profiles"), true);
            return false;
        };
        m.transform(tf);
        if op == 0 || self.scene.body(target).is_none() {
            m.compute_smooth_normals();
            let mut b = Body::new(self.unique_name(t("Тело", "Body")), m);
            b.material.shading = 0;
            self.scene.group(t("Выдавить", "Extrude"), |sc| {
                sc.set_visible(sketch, false);
                sc.add_body(b);
            });
            self.invalidate_view();
            return true;
        }
        let bop = [BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect][(op - 1).min(2)];
        let Some(tb) = self.scene.body(target).cloned() else { return false };
        let mut tm = tb.mesh.clone();
        tm.transform(tb.transform);
        match csg::boolean(&tm, &m, bop) {
            Ok(mut res) => {
                res.compute_smooth_normals();
                let mut nb = tb;
                nb.mesh = res;
                nb.transform = Mat4::identity();
                nb.primitive = None;
                nb.material.shading = 0;
                self.scene.group(t("Выдавить", "Extrude"), |sc| {
                    sc.set_visible(sketch, false);
                    sc.replace_body(nb);
                });
                self.invalidate_view();
                true
            }
            Err(e) => {
                self.toast(format!("{}: {e}", t("Ошибка операции", "Operation failed")), true);
                false
            }
        }
    }

    pub fn do_combine(&mut self, target: u64, tool: u64, op: BoolOp, keep: bool) -> bool {
        let (Some(a), Some(b)) = (self.scene.body(target).cloned(), self.scene.body(tool).cloned()) else { return false };
        let mut am = a.mesh.clone();
        am.transform(a.transform);
        let mut bm = b.mesh.clone();
        bm.transform(b.transform);
        let started = std::time::Instant::now();
        match csg::boolean(&am, &bm, op) {
            Ok(mut res) => {
                if res.is_empty() {
                    self.toast(t("Результат пуст — тела не пересекаются?", "Empty result — do the bodies overlap?"), true);
                    return false;
                }
                res.compute_smooth_normals();
                let mut nb = a;
                nb.mesh = res;
                nb.transform = Mat4::identity();
                nb.primitive = None;
                nb.material.shading = 0;
                self.scene.group(t("Объединить", "Combine"), |sc| {
                    sc.replace_body(nb);
                    if !keep {
                        sc.remove_body(tool);
                    }
                });
                self.scene.selected = Some(target);
                self.toast(format!("{} ({} ms)", t("Готово", "Done"), started.elapsed().as_millis()), false);
                self.invalidate_view();
                true
            }
            Err(e) => {
                self.toast(format!("{}: {e}", t("Ошибка булевой операции", "Boolean operation failed")), true);
                false
            }
        }
    }

    pub fn do_mirror(&mut self, plane: usize, copy: bool) {
        let Some(id) = self.selected_solid() else { return };
        let Some(b) = self.scene.body(id).cloned() else { return };
        let m = mirrored_world(&b.mesh, b.transform, plane);
        if copy {
            let mut nb = b.clone();
            nb.name = self.unique_name(&base_name(&b.name));
            nb.mesh = m;
            nb.transform = Mat4::identity();
            nb.primitive = None;
            self.scene.add_body(nb);
        } else {
            let mut nb = b;
            nb.mesh = m;
            nb.transform = Mat4::identity();
            nb.primitive = None;
            self.scene.replace_body(nb);
        }
        self.invalidate_view();
    }

    pub fn do_reduce(&mut self, id: u64, percent: f32) {
        let Some(b) = self.scene.body(id).cloned() else { return };
        let before = b.mesh.triangle_count();
        let target = ((before as f32) * percent.clamp(1.0, 100.0) / 100.0) as usize;
        if target >= before {
            return;
        }
        let mut nb = b;
        nb.mesh.decimate(target.max(4));
        nb.mesh.compute_smooth_normals();
        nb.primitive = None;
        let after = nb.mesh.triangle_count();
        self.scene.replace_body(nb);
        self.toast(format!("{} → {} {}", super::view::fmt_count(before), super::view::fmt_count(after), t("треуг.", "tris")), false);
        self.invalidate_view();
    }
}

/// Запуск ещё одного экземпляра программы — новое окно со своим проектом.
///
/// Отдельный процесс надёжнее нескольких окон в одном: проекты полностью
/// независимы, а сбой в одном окне не затрагивает другие.
pub fn spawn_new_window() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // macOS: внутри .app запускаем через `open -n`, чтобы окно получило фокус
    // и корректно отображалось в Dock.
    if cfg!(target_os = "macos") {
        let s = exe.to_string_lossy();
        if let Some(pos) = s.find(".app/Contents/MacOS/") {
            let bundle = &s[..pos + 4];
            return std::process::Command::new("/usr/bin/open").arg("-n").arg(bundle).spawn().map(|_| ()).map_err(|e| e.to_string());
        }
    }
    std::process::Command::new(exe).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Запись меша в файл экспорта.
pub fn write_export(mesh: &Mesh, path: &Path, kind: ExportKind) -> Result<(), String> {
    match kind {
        ExportKind::StlAscii => std::fs::write(path, formats::stl::to_ascii(mesh)).map_err(|e| e.to_string()),
        ExportKind::StlBinary => std::fs::write(path, formats::stl::to_binary(mesh)).map_err(|e| e.to_string()),
        ExportKind::Step => formats::save_mesh_as(mesh, path, ModelFormat::Step),
        ExportKind::ThreeMf => formats::save_mesh_as(mesh, path, ModelFormat::ThreeMf),
        ExportKind::Obj => formats::save_mesh_as(mesh, path, ModelFormat::Obj),
        ExportKind::Ply => formats::save_mesh_as(mesh, path, ModelFormat::Ply),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
}

/// Имя без числового суффикса («Цилиндр 2» → «Цилиндр»).
fn base_name(name: &str) -> String {
    match name.rsplit_once(' ') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() => head.to_string(),
        _ => name.to_string(),
    }
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
    if s.trim().is_empty() {
        "model".into()
    } else {
        s
    }
}
