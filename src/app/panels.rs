//! Панели интерфейса: панель приложения, лента инструментов, браузер,
//! свойства, временная шкала, панель навигации, меню, уведомления.

use super::actions::{Action, ExportKind, PrimKind};
use super::view::{fmt_count, ViewDir};
use super::{color_u32, compose, decompose, t, u32_color, BodyInfo, Menu, MenuItem, NavMode, State, Tab, Tool, PALETTE};
use crate::csg::BoolOp;
use crate::math::Vec3;
use crate::mesh::Primitive;
use crate::renderer::canvas::{fade, Rect};
use crate::renderer::software::DisplayMode;
use crate::scene::Command;
use crate::ui::icons::Icon;
use crate::ui::{fmt_num, id_idx, id_of, theme, Ui};

/// Описание кнопки инструмента на ленте.
struct ToolDef {
    icon: Icon,
    name: &'static str,
    tip: &'static str,
    key: &'static str,
    action: Action,
    active: bool,
    enabled: bool,
}

fn td(icon: Icon, name: &'static str, tip: &'static str, key: &'static str, action: Action) -> ToolDef {
    ToolDef { icon, name, tip, key, action, active: false, enabled: true }
}

impl State {
    // -----------------------------------------------------------------------
    // Панель приложения
    // -----------------------------------------------------------------------

    pub(super) fn draw_appbar(&mut self, ui: &mut Ui) {
        let r = self.layout.appbar;
        ui.region(r);
        ui.fb.fill_rect(r, theme::APPBAR());
        let s = ui.s;
        // Логотип.
        let lx = r.x + 10.0 * s;
        let ly = r.y + (r.h - 24.0 * s) * 0.5;
        let lr = Rect::new(lx, ly, 24.0 * s, 24.0 * s);
        crate::ui::icons::draw_logo(ui.fb, lr);
        ui.text(lr.right() + 8.0 * s, r.y + (r.h - 17.0 * s) * 0.5, "AlphaCad", 13.5, theme::APPBAR_TEXT());
        let mut x = lr.right() + 8.0 * s + ui.text_width("AlphaCad", 13.5) + 18.0 * s;

        // Кнопка «Файл».
        let fl = t("Файл", "File");
        let fw = ui.text_width(fl, 12.5) + 34.0 * s;
        let fr = Rect::new(x, r.y + 5.0 * s, fw, r.h - 10.0 * s);
        let fid = id_of("appbar_file");
        let (hov, _, clicked) = ui.interact(fid, fr);
        let open = self.menu.as_ref().map(|m| m.opener == fr).unwrap_or(false);
        if hov || open {
            ui.fb.fill_rounded(fr, 4.0 * s, theme::APPBAR_HOVER());
        }
        ui.text_in(Rect::new(fr.x + 10.0 * s, fr.y, fr.w, fr.h), fl, 12.5, theme::APPBAR_TEXT(), 0);
        ui.icon(Icon::ChevronDown, Rect::new(fr.right() - 18.0 * s, fr.y + (fr.h - 10.0 * s) * 0.5, 10.0 * s, 10.0 * s), theme::APPBAR_TEXT());
        if clicked {
            self.toggle_menu(fr, self.file_menu_items());
        }
        x = fr.right() + 6.0 * s;

        // Быстрые кнопки.
        let bs = r.h - 10.0 * s;
        let quick = [
            (Icon::Open, t("Открыть (Ctrl+O)", "Open (Ctrl+O)"), Action::Open, true),
            (Icon::Save, t("Сохранить проект (Ctrl+S)", "Save project (Ctrl+S)"), Action::Save, true),
            (Icon::Undo, t("Отменить (Ctrl+Z)", "Undo (Ctrl+Z)"), Action::Undo, self.scene.can_undo()),
            (Icon::Redo, t("Повторить (Ctrl+Y)", "Redo (Ctrl+Y)"), Action::Redo, self.scene.can_redo()),
        ];
        for (i, (ic, tip, act, en)) in quick.into_iter().enumerate() {
            let br = Rect::new(x, r.y + 5.0 * s, bs, bs);
            let id = id_idx(id_of("appbar_q"), i);
            let (hov, _, clicked) = if en { ui.interact(id, br) } else { (false, false, false) };
            if hov {
                ui.fb.fill_rounded(br, 4.0 * s, theme::APPBAR_HOVER());
            }
            let col = if en { theme::APPBAR_TEXT() } else { theme::APPBAR_DIM() };
            let ic_r = br.shrink(5.0 * s);
            match ic {
                Icon::Undo | Icon::Redo => ui.icon(ic, ic_r, col),
                _ => {
                    ui.icon(ic, ic_r, col);
                }
            }
            ui.set_tooltip(id, tip);
            if clicked {
                self.run_action(act);
            }
            x += bs + 2.0 * s;
        }

        // Вкладка документа по центру.
        let title = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| t("Без имени", "Untitled").to_string());
        let title = if self.is_dirty() { format!("{title} •") } else { title };
        let tw = ui.text_width(&title, 12.5) + 40.0 * s;
        let tr = Rect::new(((r.w - tw) * 0.5).max(x + 10.0 * s), r.y + 6.0 * s, tw, r.h - 6.0 * s);
        ui.fb.fill_rounded(Rect::new(tr.x, tr.y, tr.w, tr.h + 6.0 * s), 6.0 * s, theme::TOOLBAR());
        ui.icon(Icon::Body, Rect::new(tr.x + 9.0 * s, tr.y + (tr.h - 15.0 * s) * 0.5, 15.0 * s, 15.0 * s), theme::TEXT());
        ui.text_in(Rect::new(tr.x + 28.0 * s, tr.y, tr.w - 32.0 * s, tr.h), &title, 12.5, theme::TEXT(), 0);

        // Справа: язык, настройки, справка.
        let mut rx = r.right() - 8.0 * s;
        let right_btns = [
            (Icon::Help, t("Справка и горячие клавиши (F1)", "Help & shortcuts (F1)"), Action::Help),
            (Icon::Settings, t("Настройки", "Settings"), Action::Settings),
        ];
        for (i, (ic, tip, act)) in right_btns.into_iter().enumerate() {
            rx -= bs;
            let br = Rect::new(rx, r.y + 5.0 * s, bs, bs);
            let id = id_idx(id_of("appbar_r"), i);
            let (hov, _, clicked) = ui.interact(id, br);
            if hov {
                ui.fb.fill_rounded(br, 4.0 * s, theme::APPBAR_HOVER());
            }
            if ic == Icon::Settings {
                // Светлая шестерёнка на тёмном фоне.
                let ir = br.shrink(6.0 * s);
                ui.icon(ic, ir, theme::APPBAR_TEXT());
                let (cx, cy) = ir.center();
                ui.fb.fill_circle(cx, cy, ir.w * 0.12, theme::APPBAR());
            } else {
                ui.icon(ic, br.shrink(6.0 * s), theme::APPBAR_TEXT());
            }
            ui.set_tooltip(id, tip);
            if clicked {
                self.run_action(act);
            }
            rx -= 2.0 * s;
        }
        let lang = if super::lang_en() { "EN" } else { "RU" };
        let lw = 40.0 * s;
        rx -= lw;
        let lrr = Rect::new(rx, r.y + 7.0 * s, lw, r.h - 14.0 * s);
        let lid = id_of("appbar_lang");
        let (hov, _, clicked) = ui.interact(lid, lrr);
        ui.fb.stroke_rounded(lrr, 4.0 * s, 1.0, if hov { theme::APPBAR_TEXT() } else { theme::APPBAR_BORDER() });
        ui.text_in(lrr, lang, 11.5, theme::APPBAR_TEXT(), 1);
        ui.set_tooltip(lid, "Язык интерфейса / Interface language");
        if clicked {
            self.run_action(Action::ToggleLang);
        }
    }

    fn file_menu_items(&self) -> Vec<MenuItem> {
        let has = !self.scene.bodies.is_empty();
        let mut v = vec![
            MenuItem::new(Some(Icon::New), t("Новый проект", "New project"), "Ctrl+N", Action::New),
            MenuItem::new(None, t("Новое окно", "New window"), "Ctrl+Shift+N", Action::NewWindow),
            MenuItem::new(Some(Icon::Open), t("Открыть…", "Open…"), "Ctrl+O", Action::Open),
            MenuItem::new(Some(Icon::Import), t("Импорт в проект…", "Import into project…"), "Ctrl+I", Action::Import),
            MenuItem::sep(),
            MenuItem::new(Some(Icon::Save), t("Сохранить проект", "Save project"), "Ctrl+S", Action::Save),
            MenuItem::new(None, t("Сохранить проект как…", "Save project as…"), "Ctrl+Shift+S", Action::SaveAs),
            MenuItem::sep(),
            MenuItem::new(Some(Icon::Export), t("Экспорт STL (двоичный)…", "Export STL (binary)…"), "Ctrl+E", Action::Export(ExportKind::StlBinary)).enabled(has),
            MenuItem::new(None, t("Экспорт STL (ASCII)…", "Export STL (ASCII)…"), "", Action::Export(ExportKind::StlAscii)).enabled(has),
            MenuItem::new(None, t("Экспорт STEP…", "Export STEP…"), "", Action::Export(ExportKind::Step)).enabled(has),
            MenuItem::new(None, t("Экспорт 3MF…", "Export 3MF…"), "", Action::Export(ExportKind::ThreeMf)).enabled(has),
            MenuItem::new(None, t("Экспорт OBJ…", "Export OBJ…"), "", Action::Export(ExportKind::Obj)).enabled(has),
            MenuItem::new(None, t("Экспорт PLY…", "Export PLY…"), "", Action::Export(ExportKind::Ply)).enabled(has),
            MenuItem::new(Some(Icon::Camera), t("Снимок вида (PNG)…", "Capture image (PNG)…"), "", Action::Screenshot),
        ];
        if !self.cfg.recent.is_empty() {
            v.push(MenuItem::sep());
            for (i, r) in self.cfg.recent.iter().take(6).enumerate() {
                let name = std::path::Path::new(r).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| r.clone());
                v.push(MenuItem { icon: if i == 0 { Some(Icon::Folder) } else { None }, label: name, shortcut: String::new(), action: Some(Action::OpenRecent(i)), enabled: true });
            }
        }
        v.push(MenuItem::sep());
        v.push(MenuItem::new(Some(Icon::Close), t("Выход", "Exit"), "Ctrl+Q", Action::Quit));
        v
    }

    pub fn toggle_menu(&mut self, opener: Rect, items: Vec<MenuItem>) {
        if self.menu.as_ref().map(|m| m.opener == opener).unwrap_or(false) {
            self.menu = None;
        } else {
            self.menu = Some(Menu { x: opener.x, y: opener.bottom() + 2.0, items, opener });
        }
    }

    // -----------------------------------------------------------------------
    // Лента инструментов
    // -----------------------------------------------------------------------

    fn tool_groups(&self) -> Vec<(&'static str, Vec<ToolDef>, Vec<ToolDef>)> {
        let sel = self.scene.selected_body();
        let has_sel = sel.is_some();
        let sel_mesh = sel.map(|b| !b.mesh.is_empty()).unwrap_or(false);
        let solids = self.scene.bodies.iter().filter(|b| !b.mesh.is_empty()).count();
        let has_sketch = self.scene.bodies.iter().any(|b| b.sketch.is_some());
        let sketches = self.scene.bodies.iter().filter(|b| b.sketch.is_some()).count();
        let tool = self.tool;
        let mk_active = |mut d: ToolDef, a: bool| {
            d.active = a;
            d
        };
        let en = |mut d: ToolDef, e: bool| {
            d.enabled = e;
            d
        };
        match self.tab {
            Tab::Solid => vec![
                (
                    t("СОЗДАТЬ", "CREATE"),
                    vec![
                        td(Icon::Sketch, t("Создать эскиз", "Create sketch"), t("Начать 2D-эскиз на плоскости", "Start a 2D sketch on a plane"), "", Action::NewSketch),
                        en(td(Icon::Extrude, t("Выдавить", "Extrude"), t("Выдавить профиль эскиза в тело", "Extrude a sketch profile into a body"), "E", Action::Extrude), has_sketch),
                        en(td(Icon::Revolve, t("Вращение", "Revolve"), t("Тело вращения профиля вокруг оси эскиза или отрезка", "Revolve a profile around a sketch axis or line"), "", Action::Revolve), has_sketch),
                        en(td(Icon::Sweep, t("Сдвиг", "Sweep"), t("Протянуть профиль вдоль траектории из другого эскиза", "Sweep a profile along a path from another sketch"), "", Action::Sweep), sketches >= 2),
                        en(td(Icon::Loft, t("Лофт", "Loft"), t("Плавный переход между сечениями из разных эскизов", "Smooth transition between sections from different sketches"), "", Action::Loft), sketches >= 2),
                        td(Icon::Box, t("Параллелепипед", "Box"), t("Создать параллелепипед", "Create a box"), "", Action::CreatePrimitive(PrimKind::Box)),
                        td(Icon::Cylinder, t("Цилиндр", "Cylinder"), t("Создать цилиндр", "Create a cylinder"), "", Action::CreatePrimitive(PrimKind::Cylinder)),
                        td(Icon::Sphere, t("Сфера", "Sphere"), t("Создать сферу", "Create a sphere"), "", Action::CreatePrimitive(PrimKind::Sphere)),
                    ],
                    vec![
                        td(Icon::Torus, t("Тор", "Torus"), t("Создать тор", "Create a torus"), "", Action::CreatePrimitive(PrimKind::Torus)),
                        td(Icon::Cone, t("Конус", "Cone"), "", "", Action::CreatePrimitive(PrimKind::Cone)),
                        td(Icon::Tube, t("Труба", "Pipe"), "", "", Action::CreatePrimitive(PrimKind::Tube)),
                        td(Icon::Wedge, t("Клин", "Wedge"), "", "", Action::CreatePrimitive(PrimKind::Wedge)),
                        td(Icon::Pyramid, t("Пирамида", "Pyramid"), "", "", Action::CreatePrimitive(PrimKind::Pyramid)),
                        td(Icon::Capsule, t("Капсула", "Capsule"), "", "", Action::CreatePrimitive(PrimKind::Capsule)),
                        td(Icon::Coil, t("Пружина", "Coil"), "", "", Action::Coil),
                    ],
                ),
                (
                    t("ИЗМЕНИТЬ", "MODIFY"),
                    vec![
                        en(mk_active(td(Icon::Move, t("Переместить", "Move"), t("Перемещение манипулятором", "Move with the manipulator"), "M", Action::SetTool(Tool::Move)), tool == Tool::Move), true),
                        mk_active(td(Icon::Rotate, t("Повернуть", "Rotate"), t("Поворот манипулятором (шаг 5°, Shift — свободно)", "Rotate with the manipulator (5° snap, Shift — free)"), "R", Action::SetTool(Tool::Rotate)), tool == Tool::Rotate),
                        mk_active(td(Icon::Scale, t("Масштаб", "Scale"), t("Масштабирование по осям или равномерно", "Scale per axis or uniformly"), "S", Action::SetTool(Tool::Scale)), tool == Tool::Scale),
                        en(td(Icon::Shell, t("Оболочка", "Shell"), t("Сделать тело полым с заданной толщиной стенки", "Hollow the body with a given wall thickness"), "", Action::Shell), sel_mesh),
                        en(td(Icon::Union, t("Объединить", "Combine"), t("Булевы операции: объединение, вычитание, пересечение", "Boolean: join, cut, intersect"), "", Action::Combine(BoolOp::Union)), solids >= 2),
                        en(td(Icon::Mirror, t("Зеркало", "Mirror"), t("Зеркальное отражение тела", "Mirror a body"), "", Action::Mirror), sel_mesh),
                        en(td(Icon::Pattern, t("Массив", "Pattern"), t("Линейный или круговой массив копий тела", "Linear or circular pattern of a body"), "", Action::Pattern), sel_mesh),
                    ],
                    vec![
                        en(td(Icon::Subtract, t("Вычесть", "Cut"), "", "", Action::Combine(BoolOp::Subtract)), solids >= 2),
                        en(td(Icon::Intersect, t("Пересечь", "Intersect"), "", "", Action::Combine(BoolOp::Intersect)), solids >= 2),
                        en(td(Icon::Duplicate, t("Дублировать", "Duplicate"), "", "Ctrl+D", Action::Duplicate), has_sel),
                        en(td(Icon::Delete, t("Удалить", "Delete"), "", "Del", Action::Delete), has_sel),
                    ],
                ),
                (
                    t("СЛОЖНЫЕ ФОРМЫ", "ADVANCED"),
                    vec![
                        en(td(Icon::Twist, t("Скручивание", "Twist"), t("Скрутить тело вокруг оси", "Twist the body around an axis"), "", Action::Deform(0)), sel_mesh),
                        en(td(Icon::Taper, t("Сужение", "Taper"), t("Плавно сузить или расширить тело вдоль оси", "Taper the body along an axis"), "", Action::Deform(1)), sel_mesh),
                        en(td(Icon::Bend, t("Изгиб", "Bend"), t("Изогнуть тело по дуге", "Bend the body along an arc"), "", Action::Deform(2)), sel_mesh),
                        td(Icon::Coil, t("Пружина", "Coil"), t("Пружина: радиус, шаг, витки, проволока", "Coil: radius, pitch, turns, wire"), "", Action::Coil),
                    ],
                    vec![
                        en(td(Icon::Subdivide, t("Подразделить", "Subdivide"), "", "", Action::Subdivide), sel_mesh),
                        en(td(Icon::Smooth, t("Сгладить", "Smooth"), "", "", Action::Smooth), sel_mesh),
                    ],
                ),
                (
                    t("ВСТАВИТЬ", "INSERT"),
                    vec![td(Icon::Import, t("Вставить модель", "Insert mesh"), t("Импорт STL/STEP/3MF/OBJ/PLY в текущий проект", "Import STL/STEP/3MF/OBJ/PLY into project"), "Ctrl+I", Action::Import)],
                    vec![],
                ),
                (
                    t("ВЫБОР", "SELECT"),
                    vec![mk_active(td(Icon::Select, t("Выбор", "Select"), t("Выбор тел щелчком", "Select bodies by clicking"), "V", Action::SetTool(Tool::Select)), tool == Tool::Select)],
                    vec![],
                ),
            ],
            Tab::Sketch => {
                let in_sk = self.sketch.is_some();
                if in_sk {
                    vec![
                        (
                            t("СОЗДАТЬ", "CREATE"),
                            vec![
                                mk_active(td(Icon::Line, t("Линия", "Line"), t("Ломаная: щелчки по точкам, замыкание — щелчок в начало", "Polyline: click points, click start to close"), "L", Action::SetTool(Tool::Line)), tool == Tool::Line),
                                mk_active(td(Icon::Rect, t("Прямоугольник", "Rectangle"), t("Прямоугольник по двум углам", "Two-point rectangle"), "R", Action::SetTool(Tool::Rectangle)), tool == Tool::Rectangle),
                                mk_active(td(Icon::Circle, t("Окружность", "Circle"), t("Окружность: центр и радиус", "Circle: center and radius"), "C", Action::SetTool(Tool::Circle)), tool == Tool::Circle),
                                mk_active(td(Icon::Arc, t("Дуга", "Arc"), t("Дуга по трём точкам: начало, конец, точка на дуге", "Three-point arc: start, end, point on arc"), "A", Action::SetTool(Tool::Arc)), tool == Tool::Arc),
                                mk_active(td(Icon::Polygon, t("Многоугольник", "Polygon"), t("Правильный многоугольник: центр и вершина (число сторон — на панели)", "Regular polygon: center and vertex (sides in the panel)"), "O", Action::SetTool(Tool::Polygon)), tool == Tool::Polygon),
                                mk_active(td(Icon::Ellipse, t("Эллипс", "Ellipse"), t("Эллипс: центр, конец большой оси, ширина", "Ellipse: center, major axis end, width"), "", Action::SetTool(Tool::Ellipse)), tool == Tool::Ellipse),
                                mk_active(td(Icon::Slot, t("Паз", "Slot"), t("Паз: центры концов и ширина", "Slot: end centers and width"), "", Action::SetTool(Tool::Slot)), tool == Tool::Slot),
                                mk_active(td(Icon::Spline, t("Сплайн", "Spline"), t("Гладкая кривая через точки; Enter / ПКМ — завершить, щелчок в начало — замкнуть", "Smooth curve through points; Enter / RMB to finish, click start to close"), "S", Action::SetTool(Tool::Spline)), tool == Tool::Spline),
                            ],
                            vec![],
                        ),
                        (
                            t("ИЗМЕНИТЬ", "MODIFY"),
                            vec![
                                mk_active(td(Icon::SketchFillet, t("Скругление", "Fillet"), t("Скруглить угол между двумя отрезками (радиус — на панели)", "Fillet a corner between two lines (radius in the panel)"), "", Action::SetTool(Tool::SketchFillet)), tool == Tool::SketchFillet),
                                mk_active(td(Icon::SketchOffset, t("Смещение", "Offset"), t("Эквидистанта: щелчок по линии (расстояние — на панели, минус — внутрь)", "Offset: click a curve (distance in the panel, negative — inward)"), "", Action::SetTool(Tool::SketchOffset)), tool == Tool::SketchOffset),
                                td(Icon::Undo, t("Удалить последний", "Remove last"), t("Удалить последний элемент эскиза", "Remove the last sketch entity"), "", Action::SketchUndo),
                                mk_active(td(Icon::Select, t("Выбор", "Select"), t("Навигация без рисования", "Navigate without drawing"), "V", Action::SetTool(Tool::Select)), tool == Tool::Select),
                            ],
                            vec![],
                        ),
                        (
                            t("ЗАВЕРШИТЬ", "FINISH"),
                            vec![
                                td(Icon::Finish, t("Завершить эскиз", "Finish sketch"), t("Сохранить эскиз и выйти", "Save the sketch and exit"), "Enter", Action::FinishSketch),
                                td(Icon::Close, t("Отменить эскиз", "Cancel sketch"), t("Выйти без сохранения", "Exit without saving"), "", Action::CancelSketch),
                            ],
                            vec![],
                        ),
                    ]
                } else {
                    vec![(
                        t("ЭСКИЗ", "SKETCH"),
                        vec![
                            td(Icon::Sketch, t("Создать эскиз", "Create sketch"), t("Выбрать плоскость и начать эскиз", "Pick a plane and start sketching"), "", Action::NewSketch),
                            en(td(Icon::Extrude, t("Выдавить", "Extrude"), t("Выдавить профиль эскиза", "Extrude a sketch profile"), "E", Action::Extrude), has_sketch),
                            en(td(Icon::Revolve, t("Вращение", "Revolve"), t("Тело вращения профиля", "Revolve a profile"), "", Action::Revolve), has_sketch),
                            en(td(Icon::Sweep, t("Сдвиг", "Sweep"), t("Протянуть профиль вдоль траектории", "Sweep a profile along a path"), "", Action::Sweep), sketches >= 2),
                            en(td(Icon::Loft, t("Лофт", "Loft"), t("Переход между сечениями", "Transition between sections"), "", Action::Loft), sketches >= 2),
                        ],
                        vec![],
                    )]
                }
            }
            Tab::Mesh => vec![
                (
                    t("ПОДГОТОВКА", "PREPARE"),
                    vec![
                        en(td(Icon::Weld, t("Сварить вершины", "Merge vertices"), t("Объединить совпадающие вершины", "Merge coincident vertices"), "", Action::Weld), sel_mesh),
                        en(td(Icon::Repair, t("Исправить ориентацию", "Fix orientation"), t("Развернуть нормали наружу", "Orient normals outward"), "", Action::FixOrientation), sel_mesh),
                        en(td(Icon::FlipNormals, t("Развернуть нормали", "Flip normals"), "", "", Action::FlipNormals), sel_mesh),
                        en(td(Icon::Reduce, t("Упростить", "Reduce"), t("Уменьшить число треугольников", "Reduce the triangle count"), "", Action::Reduce), sel_mesh),
                    ],
                    vec![],
                ),
                (
                    t("МОДИФИКАТОРЫ", "MODIFIERS"),
                    vec![
                        en(td(Icon::Subdivide, t("Подразделить", "Subdivide"), t("Подразделение поверхности: гладкое (Loop) или простое", "Subdivision surface: smooth (Loop) or simple"), "", Action::Subdivide), sel_mesh),
                        en(td(Icon::Smooth, t("Сгладить", "Smooth"), t("Сглаживание вершин: Лаплас или Таубин (без усадки)", "Vertex smoothing: Laplacian or Taubin (no shrinking)"), "", Action::Smooth), sel_mesh),
                        en(td(Icon::Solidify, t("Толщина", "Solidify"), t("Придать поверхности толщину", "Give a surface thickness"), "", Action::Solidify), sel_mesh),
                    ],
                    vec![
                        en(td(Icon::Twist, t("Скручивание", "Twist"), "", "", Action::Deform(0)), sel_mesh),
                        en(td(Icon::Taper, t("Сужение", "Taper"), "", "", Action::Deform(1)), sel_mesh),
                        en(td(Icon::Bend, t("Изгиб", "Bend"), "", "", Action::Deform(2)), sel_mesh),
                    ],
                ),
                (
                    t("ЗАТЕНЕНИЕ", "SHADING"),
                    vec![
                        en(td(Icon::Shaded, t("Гладкое", "Smooth"), t("Гладкое затенение тела", "Smooth body shading"), "", Action::SetShading(1)), sel_mesh),
                        en(td(Icon::ShadedEdges, t("Плоское", "Flat"), t("Плоское (фасетное) затенение", "Flat (faceted) shading"), "", Action::SetShading(0)), sel_mesh),
                        en(td(Icon::Wireframe, t("Каркас", "Wireframe"), t("Каркасное отображение тела", "Wireframe body display"), "", Action::SetShading(2)), sel_mesh),
                    ],
                    vec![],
                ),
                (
                    t("ПОЛОЖЕНИЕ", "PLACEMENT"),
                    vec![
                        en(td(Icon::DropToFloor, t("На плоскость", "Drop to floor"), t("Опустить тело на плоскость XY", "Drop the body onto the XY plane"), "", Action::DropToFloor), sel_mesh),
                        en(td(Icon::Center, t("В начало координат", "Center at origin"), t("Центрировать по X/Y и поставить на плоскость", "Center in X/Y and drop to floor"), "", Action::CenterOrigin), sel_mesh),
                        en(td(Icon::Normals, t("Применить трансформацию", "Bake transform"), t("Записать трансформацию в вершины", "Write the transform into vertices"), "", Action::BakeTransform), sel_mesh),
                    ],
                    vec![],
                ),
            ],
            Tab::Inspect => vec![(
                t("ИНСПЕКЦИЯ", "INSPECT"),
                vec![
                    mk_active(td(Icon::Measure, t("Измерить", "Measure"), t("Расстояние между двумя точками (привязка к вершинам)", "Distance between two points (vertex snap)"), "I", Action::SetTool(Tool::Measure)), tool == Tool::Measure),
                    en(td(Icon::Info, t("Анализ сетки", "Mesh analysis"), t("Объём, площадь, замкнутость, габариты", "Volume, area, watertightness, size"), "", Action::ShowInfo), sel_mesh),
                    td(Icon::Camera, t("Снимок", "Capture"), t("Сохранить вид в PNG", "Save the view as PNG"), "", Action::Screenshot),
                ],
                vec![],
            )],
        }
    }

    pub(super) fn draw_toolbar(&mut self, ui: &mut Ui) {
        let r = self.layout.toolbar;
        ui.region(r);
        let s = ui.s;
        ui.fb.fill_rect(r, theme::TOOLBAR());
        ui.fb.fill_rect(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), theme::TOOLBAR_BORDER());

        // Переключатель рабочего пространства.
        let ws = Rect::new(r.x + 10.0 * s, r.y + 10.0 * s, 104.0 * s, r.h - 20.0 * s);
        ui.fb.fill_rounded(ws, 5.0 * s, theme::PANEL_SOLID());
        ui.fb.stroke_rounded(ws, 5.0 * s, 1.0, theme::TOOLBAR_BORDER());
        ui.text_in(Rect::new(ws.x, ws.y + 6.0 * s, ws.w, 16.0 * s), t("РАБОЧЕЕ", "WORKSPACE"), 9.5, theme::TEXT_DIM(), 1);
        ui.text_in(Rect::new(ws.x, ws.y + 22.0 * s, ws.w, 22.0 * s), t("ДИЗАЙН", "DESIGN"), 14.0, theme::ACCENT(), 1);
        ui.text_in(Rect::new(ws.x, ws.bottom() - 22.0 * s, ws.w, 16.0 * s), t("мм · Z вверх", "mm · Z up"), 10.0, theme::TEXT_DIM(), 1);

        // Вкладки.
        let tabs = [
            (Tab::Solid, t("ТВЁРДОЕ ТЕЛО", "SOLID")),
            (Tab::Sketch, t("ЭСКИЗ", "SKETCH")),
            (Tab::Mesh, t("СЕТКА", "MESH")),
            (Tab::Inspect, t("ИНСПЕКЦИЯ", "INSPECT")),
        ];
        let mut x = ws.right() + 14.0 * s;
        let tab_h = 26.0 * s;
        for (i, (tab, name)) in tabs.iter().enumerate() {
            let tw = ui.text_width(name, 11.5) + 22.0 * s;
            let tr = Rect::new(x, r.y + 2.0 * s, tw, tab_h);
            let id = id_idx(id_of("tab"), i);
            let (hov, _, clicked) = ui.interact(id, tr);
            let active = self.tab == *tab;
            let col = if active {
                if *tab == Tab::Sketch && self.sketch.is_some() {
                    theme::SKETCH()
                } else {
                    theme::ACCENT()
                }
            } else if hov {
                theme::TEXT()
            } else {
                theme::TEXT_DIM()
            };
            ui.text_in(tr, name, 11.5, col, 1);
            if active {
                ui.fb.fill_rect(Rect::new(tr.x + 6.0 * s, tr.bottom() - 2.5 * s, tr.w - 12.0 * s, 2.5 * s), col);
            }
            if clicked {
                self.tab = *tab;
            }
            x += tw + 2.0 * s;
        }
        ui.fb.fill_rect(Rect::new(ws.right() + 14.0 * s, r.y + tab_h + 3.0 * s, r.w, 1.0), fade(theme::TOOLBAR_BORDER(), 0.6));

        // Группы инструментов.
        let groups = self.tool_groups();
        let bs = 40.0 * s;
        let mut gx = ws.right() + 14.0 * s;
        let by = r.y + tab_h + 7.0 * s;
        let mut fire: Option<Action> = None;
        for (gi, (gname, tools, extra)) in groups.iter().enumerate() {
            let gstart = gx;
            for (ti, d) in tools.iter().enumerate() {
                let br = Rect::new(gx, by, bs, bs);
                let id = id_idx(id_idx(id_of("tool"), gi), ti);
                let tip = if d.tip.is_empty() {
                    d.name.to_string()
                } else if d.key.is_empty() {
                    format!("{}\n{}", d.name, d.tip)
                } else {
                    format!("{}  ({})\n{}", d.name, d.key, d.tip)
                };
                if ui.icon_button(id, br, d.icon, &tip, d.active, d.enabled) {
                    fire = Some(d.action);
                }
                gx += bs + 2.0 * s;
            }
            let gw = (gx - gstart).max(ui.text_width(gname, 10.0) + 20.0 * s);
            gx = gstart + gw;
            // Подпись группы с выпадающим списком.
            let lr = Rect::new(gstart, by + bs + 2.0 * s, gw - 2.0 * s, 16.0 * s);
            let lid = id_idx(id_of("group_label"), gi);
            let (hov, _, clicked) = ui.interact(lid, lr);
            if hov {
                ui.fb.fill_rounded(lr, 3.0 * s, theme::HOVER());
            }
            let label = format!("{gname} ▾");
            ui.text_in(lr, &label, 10.0, if hov { theme::ACCENT() } else { theme::TEXT_DIM() }, 1);
            if clicked {
                let mut items: Vec<MenuItem> = tools
                    .iter()
                    .chain(extra.iter())
                    .map(|d| MenuItem { icon: Some(d.icon), label: d.name.to_string(), shortcut: d.key.to_string(), action: Some(d.action), enabled: d.enabled })
                    .collect();
                if items.is_empty() {
                    items.push(MenuItem::sep());
                }
                self.toggle_menu(lr, items);
            }
            // Разделитель.
            gx += 8.0 * s;
            ui.fb.fill_rect(Rect::new(gx - 5.0 * s, by + 2.0 * s, 1.0, bs + 14.0 * s), theme::TOOLBAR_BORDER());
        }
        if let Some(a) = fire {
            self.run_action(a);
        }
    }

    // -----------------------------------------------------------------------
    // Браузер
    // -----------------------------------------------------------------------

    pub(super) fn draw_browser_collapsed(&mut self, ui: &mut Ui) {
        let s = ui.s;
        let b = self.layout.browser;
        let r = Rect::new(b.x, b.y, 120.0 * s, 30.0 * s);
        ui.panel(r, 6.0);
        let id = id_of("browser_expand");
        let (hov, _, clicked) = ui.interact(id, r);
        if hov {
            ui.fb.fill_rounded(r.shrink(2.0), 5.0 * s, theme::HOVER());
        }
        ui.icon(Icon::ChevronRight, Rect::new(r.x + 8.0 * s, r.y + 8.0 * s, 14.0 * s, 14.0 * s), theme::TEXT_DIM());
        ui.text_in(Rect::new(r.x + 28.0 * s, r.y, r.w - 30.0 * s, r.h), t("БРАУЗЕР", "BROWSER"), 11.0, theme::TEXT(), 0);
        if clicked {
            self.browser_open = true;
        }
    }

    pub(super) fn draw_browser(&mut self, ui: &mut Ui) {
        let s = ui.s;
        let row_h = 24.0 * s;
        let solids: Vec<usize> = (0..self.scene.bodies.len()).filter(|&i| self.scene.bodies[i].sketch.is_none()).collect();
        let sketches: Vec<usize> = (0..self.scene.bodies.len()).filter(|&i| self.scene.bodies[i].sketch.is_some()).collect();
        let rows = 3 + solids.len() + if sketches.is_empty() { 0 } else { 1 + sketches.len() };
        let content_h = 32.0 * s + rows as f32 * row_h + 10.0 * s;
        let b = self.layout.browser;
        let r = Rect::new(b.x, b.y, b.w, content_h.min(b.h));
        ui.panel(r, 6.0);
        // Заголовок.
        let hr = Rect::new(r.x, r.y, r.w, 30.0 * s);
        let id = id_of("browser_collapse");
        let (hov, _, clicked) = ui.interact(id, Rect::new(hr.x, hr.y, 30.0 * s, hr.h));
        if hov {
            ui.fb.fill_rounded(Rect::new(hr.x + 3.0 * s, hr.y + 3.0 * s, 24.0 * s, 24.0 * s), 4.0 * s, theme::HOVER());
        }
        ui.icon(Icon::ChevronDown, Rect::new(hr.x + 8.0 * s, hr.y + 8.0 * s, 14.0 * s, 14.0 * s), theme::TEXT_DIM());
        ui.text_in(Rect::new(hr.x + 30.0 * s, hr.y, hr.w - 40.0 * s, hr.h), t("БРАУЗЕР", "BROWSER"), 11.0, theme::TEXT(), 0);
        if clicked {
            self.browser_open = false;
        }
        ui.fb.fill_rect(Rect::new(r.x + 1.0, hr.bottom(), r.w - 2.0, 1.0), theme::TOOLBAR_BORDER());

        let list = Rect::new(r.x + 4.0 * s, hr.bottom() + 4.0 * s, r.w - 8.0 * s, r.bottom() - hr.bottom() - 8.0 * s);
        let sid = id_of("browser_scroll");
        let max_scroll = (content_h - r.h).max(0.0);
        let mut scroll = *ui.st.scroll.get(&sid).unwrap_or(&0.0);
        if ui.hovered(r) && ui.input.wheel != 0.0 {
            scroll = (scroll - ui.input.wheel * row_h * 2.0).clamp(0.0, max_scroll);
        }
        scroll = scroll.clamp(0.0, max_scroll);
        ui.st.scroll.insert(sid, scroll);
        let old_clip = ui.fb.clip;
        ui.fb.set_clip(list);
        let mut y = list.y - scroll;

        // Документ.
        let doc_name = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| t("Без имени", "Untitled").to_string());
        self.tree_header(ui, Rect::new(list.x, y, list.w, row_h), 0, Icon::Body, &doc_name);
        y += row_h;
        self.tree_header(ui, Rect::new(list.x, y, list.w, row_h), 1, Icon::Folder, &format!("{} ({})", t("Начало координат", "Origin"), "XYZ"));
        y += row_h;
        self.tree_header(ui, Rect::new(list.x, y, list.w, row_h), 1, Icon::Folder, &format!("{} ({})", t("Тела", "Bodies"), solids.len()));
        y += row_h;
        let mut actions: Vec<(usize, u8)> = Vec::new();
        for &i in &solids {
            if let Some(a) = self.tree_body_row(ui, Rect::new(list.x, y, list.w, row_h), i, list) {
                actions.push((i, a));
            }
            y += row_h;
        }
        if !sketches.is_empty() {
            self.tree_header(ui, Rect::new(list.x, y, list.w, row_h), 1, Icon::Folder, &format!("{} ({})", t("Эскизы", "Sketches"), sketches.len()));
            y += row_h;
            for &i in &sketches {
                if let Some(a) = self.tree_body_row(ui, Rect::new(list.x, y, list.w, row_h), i, list) {
                    actions.push((i, a));
                }
                y += row_h;
            }
        }
        ui.fb.clip = old_clip;
        for (i, a) in actions {
            let Some(b) = self.scene.bodies.get(i) else { continue };
            let id = b.id;
            match a {
                0 => {
                    self.scene.selected = Some(id);
                    self.invalidate_view();
                }
                1 => {
                    let v = !b.visible;
                    self.scene.set_visible(id, v);
                }
                2 => {
                    self.rename = Some((id, b.name.clone()));
                    self.scene.selected = Some(id);
                }
                3 => {
                    self.scene.selected = Some(id);
                    let (mx, my) = ui.input.mouse;
                    self.open_context_menu_for(id, mx, my);
                }
                4 => {
                    // Двойной щелчок по эскизу — редактировать.
                    if let Some(sk) = &b.sketch {
                        let plane = sk.plane;
                        self.begin_sketch(plane, Some(id));
                    } else {
                        self.rename = Some((id, b.name.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    fn tree_header(&mut self, ui: &mut Ui, r: Rect, depth: usize, icon: Icon, label: &str) {
        let s = ui.s;
        let x = r.x + depth as f32 * 14.0 * s;
        ui.icon(Icon::ChevronDown, Rect::new(x + 2.0 * s, r.y + 7.0 * s, 10.0 * s, 10.0 * s), theme::TEXT_DIM());
        ui.icon(icon, Rect::new(x + 15.0 * s, r.y + 3.0 * s, 18.0 * s, 18.0 * s), theme::TEXT());
        ui.text_in(Rect::new(x + 38.0 * s, r.y, r.right() - x - 40.0 * s, r.h), label, 12.0, theme::TEXT(), 0);
    }

    /// Строка тела: 0 — выбор, 1 — видимость, 2 — переименование, 3 — контекстное меню, 4 — двойной щелчок.
    fn tree_body_row(&mut self, ui: &mut Ui, r: Rect, idx: usize, clip: Rect) -> Option<u8> {
        let s = ui.s;
        let b = &self.scene.bodies[idx];
        let (bid, visible, name, is_sketch) = (b.id, b.visible, b.name.clone(), b.sketch.is_some());
        let x = r.x + 28.0 * s;
        let id = id_idx(id_of("tree_row"), bid as usize);
        let vis_r = r.intersect(&clip);
        if vis_r.is_empty() {
            return None;
        }
        let selected = self.scene.selected == Some(bid);
        let hov = ui.hovered(vis_r);
        if selected {
            ui.fb.fill_rounded(r, 4.0 * s, theme::ACCENT_SOFT());
        } else if hov {
            ui.fb.fill_rounded(r, 4.0 * s, theme::HOVER());
        }
        // Глаз.
        let er = Rect::new(x, r.y + 3.0 * s, 18.0 * s, 18.0 * s);
        let eid = id_idx(id_of("tree_eye"), bid as usize);
        let (eh, _, eclick) = ui.interact(eid, er.intersect(&clip));
        ui.icon(if visible { Icon::Eye } else { Icon::EyeOff }, er.shrink(1.0 * s), if eh { theme::ACCENT() } else if visible { theme::TEXT_DIM() } else { theme::TEXT_DISABLED() });
        ui.set_tooltip(eid, t("Показать / скрыть", "Show / hide"));
        if eclick {
            return Some(1);
        }
        let ir = Rect::new(x + 22.0 * s, r.y + 3.0 * s, 18.0 * s, 18.0 * s);
        ui.icon(if is_sketch { Icon::SketchItem } else { Icon::Body }, ir, theme::TEXT());
        let tr = Rect::new(ir.right() + 6.0 * s, r.y, r.right() - ir.right() - 8.0 * s, r.h);
        // Переименование на месте.
        if let Some((rid, buf)) = &self.rename {
            if *rid == bid {
                let mut b2 = buf.clone();
                let fid = id_of("rename_field");
                if ui.st.focus != fid {
                    ui.st.focus = fid;
                    ui.st.edit_buf = b2.clone();
                    ui.st.edit_cursor = b2.chars().count();
                    ui.st.edit_select_all = true;
                }
                let done = ui.text_field(fid, Rect::new(tr.x, r.y + 2.0 * s, tr.w, r.h - 4.0 * s), &mut b2);
                if done {
                    let nm = b2.trim().to_string();
                    if !nm.is_empty() {
                        self.scene.rename(bid, nm);
                    }
                    self.rename = None;
                } else if ui.st.focus != fid {
                    self.rename = None;
                }
                return None;
            }
        }
        ui.text_in(tr, &name, 12.0, if visible { theme::TEXT() } else { theme::TEXT_DISABLED() }, 0);
        let (_, _, clicked) = ui.interact(id, vis_r);
        if hov && ui.input.pressed[1] {
            return Some(3);
        }
        if hov && ui.input.double_click {
            return Some(4);
        }
        if clicked {
            return Some(0);
        }
        None
    }

    // -----------------------------------------------------------------------
    // Свойства выбранного тела
    // -----------------------------------------------------------------------

    fn body_info(&mut self, idx: usize) -> BodyInfo {
        let b = &self.scene.bodies[idx];
        let fp = crate::renderer::software::mesh_fingerprint(&b.mesh) ^ (b.transform.m.iter().fold(0u64, |a, v| a.rotate_left(5) ^ v.to_bits() as u64));
        if let Some((id, f, info)) = &self.info_cache {
            if *id == b.id && *f == fp {
                return info.clone();
            }
        }
        // Объём и площадь с учётом масштаба трансформации.
        let mut m = b.mesh.clone();
        m.transform(b.transform);
        let info = BodyInfo {
            tris: b.mesh.triangle_count(),
            verts: b.mesh.vertex_count(),
            size: m.aabb().size(),
            volume: m.volume().abs(),
            area: m.surface_area(),
            closed: b.mesh.is_closed(),
        };
        self.info_cache = Some((b.id, fp, info.clone()));
        info
    }

    pub(super) fn draw_properties(&mut self, ui: &mut Ui) {
        let Some(sel) = self.scene.selected else { return };
        let Some(idx) = self.scene.index_of(sel) else { return };
        let s = ui.s;
        let area = self.layout.right;
        let body = self.scene.bodies[idx].clone();
        let is_sketch = body.sketch.is_some();
        let row = 28.0 * s;
        let mut h = 44.0 * s;
        if is_sketch {
            h += row * 5.0;
        } else {
            h += row * 4.0 + 10.0 * s; // трансформация
            if let Some(p) = &body.primitive {
                h += row * (1 + prim_param_count(p)) as f32;
            }
            h += row * 3.0; // внешний вид
            h += row * 6.0; // сведения
        }
        let r = Rect::new(area.x, area.y, area.w, h.min(area.h));
        ui.panel(r, 6.0);
        let old_clip = ui.fb.clip;
        ui.fb.set_clip(r.shrink(1.0));

        // Заголовок.
        let hr = Rect::new(r.x, r.y, r.w, 36.0 * s);
        ui.fb.fill_rounded(Rect::new(hr.x + 1.0, hr.y + 1.0, hr.w - 2.0, hr.h), 6.0 * s, theme::PANEL_HEADER());
        ui.icon(if is_sketch { Icon::SketchItem } else { Icon::Body }, Rect::new(hr.x + 10.0 * s, hr.y + 9.0 * s, 18.0 * s, 18.0 * s), theme::TEXT());
        let mut name = body.name.clone();
        if ui.text_field(id_of("prop_name"), Rect::new(hr.x + 34.0 * s, hr.y + 6.0 * s, hr.w - 72.0 * s, 24.0 * s), &mut name) {
            let nm = name.trim().to_string();
            if !nm.is_empty() && nm != body.name {
                self.scene.rename(sel, nm);
            }
        }
        if ui.icon_button(id_of("prop_close"), Rect::new(hr.right() - 32.0 * s, hr.y + 6.0 * s, 24.0 * s, 24.0 * s), Icon::Close, t("Снять выбор (Esc)", "Deselect (Esc)"), false, true) {
            self.scene.selected = None;
            self.invalidate_view();
        }
        let x0 = r.x + 12.0 * s;
        let w0 = r.w - 24.0 * s;
        let mut y = hr.bottom() + 6.0 * s;

        if let Some(sk) = &body.sketch {
            ui.section(Rect::new(x0, y, w0, 20.0 * s), t("Эскиз", "Sketch"));
            y += row;
            let fr = ui.labeled(Rect::new(x0, y, w0, 24.0 * s), t("Плоскость", "Plane"), 110.0);
            ui.text_in(fr, &sk.plane.label(), 12.5, theme::TEXT(), 0);
            y += row;
            let fr = ui.labeled(Rect::new(x0, y, w0, 24.0 * s), t("Элементов", "Entities"), 110.0);
            ui.text_in(fr, &sk.entities.len().to_string(), 12.5, theme::TEXT(), 0);
            y += row;
            let fr = ui.labeled(Rect::new(x0, y, w0, 24.0 * s), t("Профилей", "Profiles"), 110.0);
            ui.text_in(fr, &sk.profiles().len().to_string(), 12.5, theme::TEXT(), 0);
            y += row + 4.0 * s;
            let bw = (w0 - 8.0 * s) * 0.5;
            if ui.button(id_of("prop_edit_sketch"), Rect::new(x0, y, bw, 26.0 * s), t("Редактировать", "Edit"), false) {
                let plane = sk.plane;
                self.begin_sketch(plane, Some(sel));
            }
            if ui.button(id_of("prop_extrude"), Rect::new(x0 + bw + 8.0 * s, y, bw, 26.0 * s), t("Выдавить", "Extrude"), true) {
                self.run_action(Action::Extrude);
            }
            ui.fb.clip = old_clip;
            return;
        }

        // Трансформация.
        ui.section(Rect::new(x0, y, w0, 20.0 * s), t("Трансформация", "Transform"));
        y += row;
        let (tr, rot, sc) = decompose(&body.transform);
        let lw = 78.0;
        let mut changed = false;
        let mut vals = [[tr.x, tr.y, tr.z], [rot.x, rot.y, rot.z], [sc.x, sc.y, sc.z]];
        let labels = [t("Положение", "Position"), t("Поворот", "Rotation"), t("Масштаб", "Scale")];
        for (k, lab) in labels.iter().enumerate() {
            let fr = ui.labeled(Rect::new(x0, y, w0, 24.0 * s), lab, lw);
            let fw = (fr.w - 8.0 * s) / 3.0;
            for c in 0..3 {
                let cr = Rect::new(fr.x + c as f32 * (fw + 4.0 * s), y, fw, 24.0 * s);
                // Цветная метка оси.
                ui.fb.fill_rect(Rect::new(cr.x, cr.y + 3.0 * s, 2.0 * s, cr.h - 6.0 * s), [0xFFE0_3E3E, 0xFF35_A853, 0xFF2F_6FE0][c]);
                let id = id_idx(id_idx(id_of("prop_tf"), k), c);
                let (step, min, max) = match k {
                    0 => (1.0, -1e6, 1e6),
                    1 => (5.0, -360.0, 360.0),
                    _ => (0.1, -1000.0, 1000.0),
                };
                if ui.number_field(id, cr, &mut vals[k][c], "", step, min, max) {
                    changed = true;
                }
            }
            y += row;
        }
        if changed {
            let fix = |v: f32| if v.abs() < 1e-6 { 1e-6 } else { v };
            let m = compose(
                Vec3::new(vals[0][0], vals[0][1], vals[0][2]),
                Vec3::new(vals[1][0], vals[1][1], vals[1][2]),
                Vec3::new(fix(vals[2][0]), fix(vals[2][1]), fix(vals[2][2])),
            );
            self.scene.set_transform(sel, m);
        }
        y += 10.0 * s;

        // Параметры примитива.
        if let Some(p) = body.primitive {
            ui.section(Rect::new(x0, y, w0, 20.0 * s), &format!("{} — {}", t("Параметры", "Parameters"), State::primitive_name(&p)));
            y += row;
            let mut np = p;
            if edit_primitive(ui, &mut np, x0, &mut y, w0, row, "prop_prim") && np != p {
                self.scene.set_primitive(sel, np);
            }
        }

        // Внешний вид.
        ui.section(Rect::new(x0, y, w0, 20.0 * s), t("Внешний вид", "Appearance"));
        y += row;
        let cur = color_u32(body.material.color);
        if let Some(c) = ui.color_palette(id_of("prop_color"), Rect::new(x0, y, w0, 24.0 * s), &PALETTE, cur) {
            let mut m = body.material;
            m.color = u32_color(c);
            self.scene.set_material(sel, m);
        }
        y += row;
        let mut sh = match body.material.shading {
            0 => 1,
            2 => 2,
            _ => 0,
        } as usize;
        if ui.segmented(id_of("prop_shading"), Rect::new(x0, y, w0, 24.0 * s), &[t("Гладкое", "Smooth"), t("Плоское", "Flat"), t("Каркас", "Wire")], &mut sh) {
            let mut m = body.material;
            m.shading = [1, 0, 2][sh];
            self.scene.set_material(sel, m);
        }
        y += row + 6.0 * s;

        // Сведения.
        let info = self.body_info(idx);
        ui.section(Rect::new(x0, y, w0, 20.0 * s), t("Сведения", "Info"));
        y += row;
        let mm = t("мм", "mm");
        let rows: [(&str, String); 5] = [
            (t("Треугольников", "Triangles"), fmt_count(info.tris)),
            (t("Габариты", "Size"), format!("{} × {} × {} {mm}", fmt_num(info.size.x), fmt_num(info.size.y), fmt_num(info.size.z))),
            (t("Объём", "Volume"), format!("{} {mm}³", fmt_num(info.volume))),
            (t("Площадь", "Area"), format!("{} {mm}²", fmt_num(info.area))),
            (t("Замкнутость", "Watertight"), if info.closed { t("да", "yes").to_string() } else { t("нет (есть открытые рёбра)", "no (open edges)").to_string() }),
        ];
        for (k, v) in rows.iter() {
            let fr = ui.labeled(Rect::new(x0, y, w0, 22.0 * s), k, 110.0);
            let col = if *k == t("Замкнутость", "Watertight") && !info.closed { theme::DANGER() } else { theme::TEXT() };
            ui.text_in(fr, v, 12.0, col, 0);
            y += 22.0 * s;
        }
        ui.fb.clip = old_clip;
    }

    /// Панель эскиза (подсказки и инструменты в режиме эскиза).
    pub(super) fn draw_sketch_panel(&mut self, ui: &mut Ui) {
        if self.sketch.is_none() || self.dialog.is_some() {
            return;
        }
        let s = ui.s;
        let area = self.layout.right;
        let r = Rect::new(area.x, area.y, area.w, 316.0 * s);
        ui.panel(r, 6.0);
        let hr = Rect::new(r.x, r.y, r.w, 34.0 * s);
        ui.fb.fill_rounded(Rect::new(hr.x + 1.0, hr.y + 1.0, hr.w - 2.0, hr.h), 6.0 * s, theme::SKETCH_SOFT());
        ui.icon(Icon::Sketch, Rect::new(hr.x + 10.0 * s, hr.y + 8.0 * s, 18.0 * s, 18.0 * s), theme::TEXT());
        let name = self.sketch.as_ref().map(|s| s.sketch.name.clone()).unwrap_or_default();
        ui.text_in(Rect::new(hr.x + 34.0 * s, hr.y, hr.w - 40.0 * s, hr.h), &name, 13.0, theme::TEXT(), 0);
        let x0 = r.x + 12.0 * s;
        let w0 = r.w - 24.0 * s;
        let mut y = hr.bottom() + 8.0 * s;
        let tools = [
            (Tool::Line, Icon::Line, t("Линия (L)", "Line (L)")),
            (Tool::Rectangle, Icon::Rect, t("Прямоугольник (R)", "Rectangle (R)")),
            (Tool::Circle, Icon::Circle, t("Окружность (C)", "Circle (C)")),
            (Tool::Arc, Icon::Arc, t("Дуга (A)", "Arc (A)")),
            (Tool::Polygon, Icon::Polygon, t("Многоугольник (O)", "Polygon (O)")),
            (Tool::Ellipse, Icon::Ellipse, t("Эллипс", "Ellipse")),
            (Tool::Slot, Icon::Slot, t("Паз", "Slot")),
            (Tool::Spline, Icon::Spline, t("Сплайн (S)", "Spline (S)")),
            (Tool::SketchFillet, Icon::SketchFillet, t("Скругление", "Fillet")),
            (Tool::SketchOffset, Icon::SketchOffset, t("Смещение", "Offset")),
            (Tool::Select, Icon::Select, t("Навигация (V)", "Navigate (V)")),
        ];
        let per_row = 6usize;
        let bw = (w0 - 4.0 * s * (per_row as f32 - 1.0)) / per_row as f32;
        for (i, (tl, ic, tip)) in tools.iter().enumerate() {
            let (cx, cy) = ((i % per_row) as f32, (i / per_row) as f32);
            let br = Rect::new(x0 + cx * (bw + 4.0 * s), y + cy * 34.0 * s, bw, 30.0 * s);
            if ui.icon_button(id_idx(id_of("sk_tool"), i), br, *ic, tip, self.tool == *tl, true) {
                self.run_action(Action::SetTool(*tl));
            }
        }
        y += 72.0 * s;
        // Подсказка и параметр текущего инструмента.
        let hint = match self.tool {
            Tool::Line => t("Щелчки по точкам; щелчок в начало — замкнуть, ПКМ — прервать", "Click points; click the start to close, RMB to stop"),
            Tool::Rectangle => t("Два противоположных угла", "Two opposite corners"),
            Tool::Circle => t("Центр, затем точка на окружности", "Center, then a point on the circle"),
            Tool::Arc => t("Начало, конец, затем точка на дуге", "Start, end, then a point on the arc"),
            Tool::Polygon => t("Центр, затем вершина", "Center, then a vertex"),
            Tool::Ellipse => t("Центр, конец большой оси, затем ширина", "Center, major axis end, then width"),
            Tool::Slot => t("Центры двух концов, затем ширина", "Two end centers, then width"),
            Tool::Spline => t("Точки кривой; Enter / ПКМ — готово, щелчок в начало — замкнуть", "Curve points; Enter / RMB to finish, click start to close"),
            Tool::SketchFillet => t("Щёлкните по углу между двумя отрезками", "Click a corner between two lines"),
            Tool::SketchOffset => t("Щёлкните по линии; минус — внутрь", "Click a curve; negative — inward"),
            _ => t("Навигация без рисования", "Navigate without drawing"),
        };
        ui.text_in(Rect::new(x0, y, w0, 18.0 * s), hint, 11.0, theme::TEXT_DIM(), 0);
        y += 22.0 * s;
        let param: Option<(&str, &mut f32, &str, f32, f32, f32)> = match self.tool {
            Tool::Polygon => Some((t("Сторон", "Sides"), &mut self.sketch_params.polygon_sides, "", 1.0, 3.0, 64.0)),
            Tool::SketchFillet => Some((t("Радиус", "Radius"), &mut self.sketch_params.fillet_radius, t("мм", "mm"), 0.5, 0.01, 1e4)),
            Tool::SketchOffset => Some((t("Расстояние", "Distance"), &mut self.sketch_params.offset, t("мм", "mm"), 0.5, -1e4, 1e4)),
            _ => None,
        };
        if let Some((label, v, unit, step, lo, hi)) = param {
            let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), label, 100.0);
            if ui.number_field(id_of("sk_param"), fr, v, unit, step, lo, hi) {
                self.invalidate_view();
            }
            if self.tool == Tool::Polygon {
                self.sketch_params.polygon_sides = self.sketch_params.polygon_sides.round();
            }
        }
        y += 32.0 * s;
        let (n_ent, n_prof) = self.sketch.as_ref().map(|s| (s.sketch.entities.len(), s.sketch.profiles().len())).unwrap_or((0, 0));
        ui.text_in(Rect::new(x0, y, w0, 20.0 * s), &format!("{}: {}   {}: {}", t("Элементов", "Entities"), n_ent, t("Замкнутых профилей", "Closed profiles"), n_prof), 12.0, theme::TEXT_DIM(), 0);
        y += 24.0 * s;
        let mut snap = self.cfg.snap;
        if ui.checkbox(id_of("sk_snap"), Rect::new(x0, y, w0, 22.0 * s), t("Привязка к сетке (Shift — отключить)", "Grid snap (hold Shift to disable)"), &mut snap) {
            self.cfg.snap = snap;
        }
        y += 30.0 * s;
        let bw2 = (w0 - 8.0 * s) * 0.5;
        if ui.button_ex(id_of("sk_undo"), Rect::new(x0, y, bw2, 26.0 * s), t("Удалить последний", "Remove last"), false, n_ent > 0) {
            self.run_action(Action::SketchUndo);
        }
        if ui.button(id_of("sk_cancel"), Rect::new(x0 + bw2 + 8.0 * s, y, bw2, 26.0 * s), t("Отменить эскиз", "Cancel sketch"), false) {
            self.run_action(Action::CancelSketch);
        }
        y += 34.0 * s;
        if ui.button(id_of("sk_finish"), Rect::new(x0, y, w0, 30.0 * s), t("✓ Завершить эскиз", "✓ Finish sketch"), true) {
            self.run_action(Action::FinishSketch);
        }
    }

    // -----------------------------------------------------------------------
    // Временная шкала
    // -----------------------------------------------------------------------

    pub(super) fn draw_timeline(&mut self, ui: &mut Ui) {
        let r = self.layout.timeline;
        ui.region(r);
        let s = ui.s;
        ui.fb.fill_rect(r, theme::TIMELINE());
        ui.fb.fill_rect(Rect::new(r.x, r.y, r.w, 1.0), theme::TOOLBAR_BORDER());
        let bs = 26.0 * s;
        let by = r.y + (r.h - bs) * 0.5;
        let mut x = r.x + 8.0 * s;
        let nu = self.scene.undo_stack.len();
        let nr = self.scene.redo_stack.len();
        let ctrl = [
            (Icon::First, t("К началу истории", "Go to start"), nu > 0, 0i32),
            (Icon::Back, t("Шаг назад (Ctrl+Z)", "Step back (Ctrl+Z)"), nu > 0, 1),
            (Icon::Play, t("Шаг вперёд (Ctrl+Y)", "Step forward (Ctrl+Y)"), nr > 0, 2),
            (Icon::Last, t("К концу истории", "Go to end"), nr > 0, 3),
        ];
        for (i, (ic, tip, en, code)) in ctrl.into_iter().enumerate() {
            if ui.icon_button(id_idx(id_of("tl_ctrl"), i), Rect::new(x, by, bs, bs), ic, tip, false, en) {
                match code {
                    0 => self.history_goto(0),
                    1 => self.run_action(Action::Undo),
                    2 => self.run_action(Action::Redo),
                    _ => self.history_goto(nu + nr),
                }
            }
            x += bs + 2.0 * s;
        }
        x += 8.0 * s;
        ui.fb.fill_rect(Rect::new(x - 5.0 * s, by + 3.0 * s, 1.0, bs - 6.0 * s), theme::TOOLBAR_BORDER());

        // Элементы истории.
        let status_w = 340.0 * s;
        let avail = (r.right() - status_w - x).max(0.0);
        let ew = 24.0 * s;
        let total = nu + nr;
        let max_items = (avail / (ew + 2.0 * s)) as usize;
        let start = total.saturating_sub(max_items);
        let mut goto: Option<usize> = None;
        let entries: Vec<(&Command, bool)> = self.scene.undo_stack.iter().map(|c| (c, true)).chain(self.scene.redo_stack.iter().rev().map(|c| (c, false))).collect();
        for (i, (cmd, done)) in entries.iter().enumerate().skip(start) {
            let er = Rect::new(x, by + 1.0 * s, ew, ew);
            let (icon, label) = command_icon(cmd);
            let id = id_idx(id_of("tl_item"), i);
            let (hov, _, clicked) = ui.interact(id, er);
            let bg = if *done { if hov { theme::TIMELINE_ITEM_HOVER() } else { theme::TIMELINE_ITEM() } } else if hov { theme::TIMELINE_UNDONE_HOVER() } else { theme::TIMELINE_UNDONE() };
            ui.fb.fill_rounded(er, 3.0 * s, bg);
            ui.fb.stroke_rounded(er, 3.0 * s, 1.0, if *done { theme::BORDER() } else { theme::TOOLBAR_BORDER() });
            ui.icon(icon, er.shrink(3.0 * s), theme::TEXT());
            if !*done {
                ui.fb.fill_rounded(er, 3.0 * s, (theme::TIMELINE_UNDONE() & 0x00FF_FFFF) | 0x8000_0000);
            }
            ui.set_tooltip(id, &format!("{}. {}", i + 1, label));
            if clicked {
                goto = Some(i + 1);
            }
            x += ew + 2.0 * s;
            if i + 1 == nu {
                // Маркер текущего положения.
                ui.fb.fill_rect(Rect::new(x - 1.0 * s, r.y + 4.0 * s, 2.5 * s, r.h - 8.0 * s), theme::ACCENT());
                ui.fb.fill_polygon(&[(x - 5.0 * s, r.y + 3.0 * s), (x + 5.5 * s, r.y + 3.0 * s), (x + 0.25 * s, r.y + 9.0 * s)], theme::ACCENT());
                x += 4.0 * s;
            }
        }
        if let Some(g) = goto {
            self.history_goto(g);
        }

        // Строка состояния.
        let status = self.status_text();
        ui.text_in(Rect::new(r.right() - status_w, r.y, status_w - 12.0 * s, r.h), &status, 11.5, theme::TEXT_DIM(), 2);
    }

    fn status_text(&self) -> String {
        if let Some(sess) = &self.sketch {
            return match (self.tool, sess.anchor.is_some()) {
                (Tool::Line, false) => t("Линия: щёлкните начальную точку", "Line: click the start point").into(),
                (Tool::Line, true) => t("Линия: следующая точка · ПКМ — завершить цепочку", "Line: next point · RMB ends the chain").into(),
                (Tool::Rectangle, false) => t("Прямоугольник: первый угол", "Rectangle: first corner").into(),
                (Tool::Rectangle, true) => t("Прямоугольник: противоположный угол", "Rectangle: opposite corner").into(),
                (Tool::Circle, false) => t("Окружность: центр", "Circle: center").into(),
                (Tool::Circle, true) => t("Окружность: точка на окружности", "Circle: point on circle").into(),
                _ => t("Режим эскиза", "Sketch mode").into(),
            };
        }
        match self.tool {
            Tool::Measure => t("Измерение: щёлкните две точки на поверхности", "Measure: click two points on a surface").into(),
            Tool::Move | Tool::Rotate | Tool::Scale if self.scene.selected.is_none() => t("Выберите тело для трансформации", "Select a body to transform").into(),
            _ => match self.scene.selected_body() {
                Some(b) => format!("{}: {}", t("Выбрано", "Selected"), b.name),
                None => format!("{}: {} · {}", t("Тел", "Bodies"), self.scene.bodies.len(), t("ЛКМ — вращение, ПКМ — сдвиг, колесо — масштаб", "LMB orbit, RMB pan, wheel zoom")),
            },
        }
    }

    /// Переход к позиции истории (количеству выполненных команд).
    pub fn history_goto(&mut self, pos: usize) {
        while self.scene.undo_stack.len() > pos && self.scene.can_undo() {
            self.scene.undo();
        }
        while self.scene.undo_stack.len() < pos && self.scene.can_redo() {
            self.scene.redo();
        }
        self.invalidate_view();
    }

    // -----------------------------------------------------------------------
    // Панель навигации
    // -----------------------------------------------------------------------

    pub(super) fn draw_navbar(&mut self, ui: &mut Ui) {
        let r = self.layout.navbar;
        let s = ui.s;
        ui.panel(r, 8.0);
        let bs = r.h - 8.0 * s;
        let mut x = r.x + 6.0 * s;
        let y = r.y + 4.0 * s;
        let items: [(Icon, &str, Option<Action>, bool); 10] = [
            (Icon::Orbit, t("Вращение левой кнопкой (по умолчанию)", "Orbit with left button (default)"), Some(Action::SetNav(NavMode::Orbit)), matches!(self.nav_mode, NavMode::Default | NavMode::Orbit)),
            (Icon::Pan, t("Сдвиг левой кнопкой", "Pan with left button"), Some(Action::SetNav(NavMode::Pan)), self.nav_mode == NavMode::Pan),
            (Icon::Zoom, t("Масштаб левой кнопкой", "Zoom with left button"), Some(Action::SetNav(NavMode::Zoom)), self.nav_mode == NavMode::Zoom),
            (Icon::Fit, t("Вписать всё (F)", "Fit all (F)"), Some(Action::Fit), false),
            (Icon::Home, t("Начальный вид", "Home view"), Some(Action::ViewHome), false),
            (Icon::Shaded, "", None, false),
            (match self.cfg.mode {
                DisplayMode::Shaded => Icon::Shaded,
                DisplayMode::ShadedEdges => Icon::ShadedEdges,
                DisplayMode::Wireframe => Icon::Wireframe,
            }, t("Стиль отображения (W)", "Visual style (W)"), Some(Action::DisplayMenu), false),
            (Icon::Grid, t("Сетка (G)", "Grid (G)"), Some(Action::ToggleGrid), self.cfg.show_grid),
            (Icon::Perspective, t("Перспектива / ортография (P)", "Perspective / orthographic (P)"), Some(Action::ToggleOrtho), !self.cam.ortho),
            (Icon::Camera, t("Снимок вида (PNG)", "Capture image (PNG)"), Some(Action::Screenshot), false),
        ];
        for (i, (ic, tip, act, active)) in items.into_iter().enumerate() {
            match act {
                None => {
                    ui.fb.fill_rect(Rect::new(x + 2.0 * s, y + 6.0 * s, 1.0, bs - 12.0 * s), theme::TOOLBAR_BORDER());
                    x += 6.0 * s;
                }
                Some(a) => {
                    let br = Rect::new(x, y, bs, bs);
                    if ui.icon_button(id_idx(id_of("nav"), i), br, ic, tip, active, true) {
                        if a == Action::DisplayMenu {
                            let items = vec![
                                MenuItem::new(Some(Icon::Shaded), t("Тонированный", "Shaded"), "", Action::SetMode(DisplayMode::Shaded)),
                                MenuItem::new(Some(Icon::ShadedEdges), t("Тонированный с рёбрами", "Shaded with edges"), "", Action::SetMode(DisplayMode::ShadedEdges)),
                                MenuItem::new(Some(Icon::Wireframe), t("Каркас", "Wireframe"), "", Action::SetMode(DisplayMode::Wireframe)),
                                MenuItem::sep(),
                                MenuItem::new(None, if self.cfg.dark_bg { t("Светлый фон", "Light background") } else { t("Тёмный фон", "Dark background") }, "", Action::ToggleBackground),
                            ];
                            let opener = br;
                            self.toggle_menu(opener, items);
                            if let Some(m) = &mut self.menu {
                                m.y = br.y - 6.0 * s - 5.0 * 28.0 * s;
                            }
                        } else {
                            self.run_action(a);
                        }
                    }
                    x += bs + 2.0 * s;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Меню и уведомления
    // -----------------------------------------------------------------------

    pub(super) fn draw_menu(&mut self, ui: &mut Ui) {
        let Some(menu) = self.menu.clone() else { return };
        let s = ui.s;
        let old_layer = ui.st.layer;
        ui.st.layer = 2;
        let ih = 28.0 * s;
        let sh = 9.0 * s;
        let mut h = 8.0 * s;
        let mut w: f32 = 200.0 * s;
        for it in &menu.items {
            if it.action.is_none() {
                h += sh;
            } else {
                h += ih;
                w = w.max(ui.text_width(&it.label, 12.5) + ui.text_width(&it.shortcut, 11.5) + 70.0 * s);
            }
        }
        let fw = ui.fb.width as f32;
        let fh = ui.fb.height as f32;
        let x = menu.x.min(fw - w - 4.0).max(0.0);
        let y = menu.y.min(fh - h - 4.0).max(0.0);
        let r = Rect::new(x, y, w, h);
        ui.fb.reset_clip();
        ui.panel(r, 6.0);
        let mut yy = y + 4.0 * s;
        let mut fire = None;
        for (i, it) in menu.items.iter().enumerate() {
            if it.action.is_none() {
                ui.menu_separator(Rect::new(x, yy, w, sh));
                yy += sh;
                continue;
            }
            let ir = Rect::new(x + 4.0 * s, yy, w - 8.0 * s, ih);
            if ui.menu_item(id_idx(id_of("menu_item"), i), ir, it.icon, &it.label, &it.shortcut, it.enabled) {
                fire = it.action;
            }
            yy += ih;
        }
        ui.st.layer = old_layer;
        let (mx, my) = ui.input.mouse;
        let outside = !r.contains(mx, my) && !menu.opener.contains(mx, my);
        if fire.is_some() || ((ui.input.pressed[0] || ui.input.pressed[1]) && outside) || ui.input.key_pressed(crate::ui::Key::Escape) {
            self.menu = None;
        }
        if let Some(a) = fire {
            self.run_action(a);
        }
    }

    pub(super) fn draw_toasts(&mut self, ui: &mut Ui) {
        let s = ui.s;
        self.toasts.retain(|t| t.at.elapsed().as_secs_f32() < if t.error { 7.0 } else { 3.5 });
        if self.toasts.is_empty() {
            return;
        }
        let mut y = self.layout.view.bottom() - 60.0 * s;
        let toasts = self.toasts.clone();
        for t in toasts.iter().rev() {
            let age = t.at.elapsed().as_secs_f32();
            let life = if t.error { 7.0 } else { 3.5 };
            let alpha = ((life - age) / 0.4).clamp(0.0, 1.0);
            let w = (ui.text_width(&t.text, 12.5) + 44.0 * s).min(self.layout.view.w * 0.6);
            let h = 34.0 * s;
            let r = Rect::new(self.layout.view.right() - w - 14.0 * s, y - h, w, h);
            ui.fb.shadow(r, 6.0 * s, 5.0 * s, (60.0 * alpha) as u8);
            ui.fb.fill_rounded(r, 6.0 * s, fade(theme::TOOLTIP(), alpha));
            ui.fb.fill_circle(r.x + 16.0 * s, r.y + h * 0.5, 4.5 * s, fade(if t.error { theme::DANGER() } else { theme::OK() }, alpha));
            ui.text_in(Rect::new(r.x + 28.0 * s, r.y, r.w - 34.0 * s, r.h), &t.text, 12.5, fade(0xFFFF_FFFF, alpha), 0);
            y -= h + 8.0 * s;
        }
        self.wake_ms = Some(self.wake_ms.map_or(100, |w| w.min(100)));
    }

    /// Контекстное меню по правому клику в 3D-виде.
    pub(super) fn open_context_menu(&mut self, mx: f32, my: f32) {
        let target = self.hover_body.or_else(|| self.pick_sketch_body(mx, my));
        match target {
            Some(id) => {
                self.scene.selected = Some(id);
                self.invalidate_view();
                self.open_context_menu_for(id, mx, my);
            }
            None => {
                let items = vec![
                    MenuItem::new(Some(Icon::Fit), t("Вписать всё", "Fit all"), "F", Action::Fit),
                    MenuItem::new(Some(Icon::Home), t("Начальный вид", "Home view"), "Home", Action::ViewHome),
                    MenuItem::sep(),
                    MenuItem::new(None, t("Вид спереди", "Front view"), "1", Action::SetView(ViewDir::Front)),
                    MenuItem::new(None, t("Вид справа", "Right view"), "3", Action::SetView(ViewDir::Right)),
                    MenuItem::new(None, t("Вид сверху", "Top view"), "7", Action::SetView(ViewDir::Top)),
                    MenuItem::sep(),
                    MenuItem::new(Some(Icon::Sketch), t("Создать эскиз", "Create sketch"), "", Action::NewSketch),
                    MenuItem::new(Some(Icon::Box), t("Параллелепипед", "Box"), "", Action::CreatePrimitive(PrimKind::Box)),
                    MenuItem::new(Some(Icon::Import), t("Вставить модель…", "Insert mesh…"), "Ctrl+I", Action::Import),
                ];
                self.menu = Some(Menu { x: mx, y: my, items, opener: Rect::new(mx, my, 1.0, 1.0) });
            }
        }
    }

    pub(super) fn open_context_menu_for(&mut self, id: u64, mx: f32, my: f32) {
        let Some(b) = self.scene.body(id) else { return };
        let is_sketch = b.sketch.is_some();
        let mut items = Vec::new();
        if is_sketch {
            items.push(MenuItem::new(Some(Icon::Sketch), t("Редактировать эскиз", "Edit sketch"), "", Action::EditSketch));
            items.push(MenuItem::new(Some(Icon::Extrude), t("Выдавить", "Extrude"), "E", Action::Extrude));
            items.push(MenuItem::sep());
        } else {
            items.push(MenuItem::new(Some(Icon::Move), t("Переместить", "Move"), "M", Action::SetTool(Tool::Move)));
            items.push(MenuItem::new(Some(Icon::Rotate), t("Повернуть", "Rotate"), "R", Action::SetTool(Tool::Rotate)));
            items.push(MenuItem::new(Some(Icon::DropToFloor), t("Опустить на плоскость", "Drop to floor"), "", Action::DropToFloor));
            items.push(MenuItem::new(Some(Icon::Info), t("Анализ сетки", "Mesh analysis"), "", Action::ShowInfo));
            items.push(MenuItem::new(Some(Icon::Export), t("Экспорт тела в STL…", "Export body as STL…"), "", Action::Export(ExportKind::StlBinary)));
            items.push(MenuItem::sep());
        }
        items.push(MenuItem::new(Some(Icon::Fit), t("Приблизить к телу", "Zoom to body"), "", Action::FocusSelected));
        items.push(MenuItem::new(Some(Icon::EyeOff), t("Скрыть", "Hide"), "", Action::HideSelected));
        items.push(MenuItem::new(None, t("Переименовать", "Rename"), "", Action::RenameSelected));
        items.push(MenuItem::new(Some(Icon::Duplicate), t("Дублировать", "Duplicate"), "Ctrl+D", Action::Duplicate));
        items.push(MenuItem::new(Some(Icon::Delete), t("Удалить", "Delete"), "Del", Action::Delete));
        self.menu = Some(Menu { x: mx, y: my, items, opener: Rect::new(mx, my, 1.0, 1.0) });
    }
}

/// Иконка и название команды истории.
fn command_icon(c: &Command) -> (Icon, String) {
    match c {
        Command::AddBody { body, .. } => {
            let icon = match body.primitive {
                Some(Primitive::Box { .. }) => Icon::Box,
                Some(Primitive::Cylinder { .. }) => Icon::Cylinder,
                Some(Primitive::Sphere { .. }) => Icon::Sphere,
                Some(Primitive::Torus { .. }) => Icon::Torus,
                Some(Primitive::Cone { .. }) => Icon::Cone,
                Some(Primitive::Tube { .. }) => Icon::Tube,
                Some(Primitive::Wedge { .. }) => Icon::Wedge,
                Some(Primitive::Pyramid { .. }) => Icon::Pyramid,
                Some(Primitive::Capsule { .. }) => Icon::Capsule,
                None => {
                    if body.sketch.is_some() {
                        Icon::Sketch
                    } else if body.source_path.is_some() {
                        Icon::Import
                    } else {
                        Icon::Body
                    }
                }
            };
            (icon, format!("{}: {}", t("Создание", "Create"), body.name))
        }
        Command::RemoveBody { body, .. } => (Icon::Delete, format!("{}: {}", t("Удаление", "Delete"), body.name)),
        Command::TransformBody { .. } => (Icon::Move, t("Трансформация", "Transform").into()),
        Command::RenameBody { after, .. } => (Icon::SketchItem, format!("{}: {}", t("Переименование", "Rename"), after)),
        Command::SetMaterial { .. } => (Icon::Shaded, t("Внешний вид", "Appearance").into()),
        Command::SetVisible { after, .. } => (if *after { Icon::Eye } else { Icon::EyeOff }, t("Видимость", "Visibility").into()),
        Command::ReplaceMesh { .. } => (Icon::Weld, t("Изменение сетки", "Mesh edit").into()),
        Command::ReplacePrimitive { .. } => (Icon::Settings, t("Параметры примитива", "Primitive parameters").into()),
        Command::ReplaceBody { after, .. } => (if after.sketch.is_some() { Icon::Sketch } else { Icon::Repair }, format!("{}: {}", t("Изменение", "Edit"), after.name)),
        Command::Group { label, cmds } => {
            let icon = if label.contains("Выдав") || label.contains("Extrude") {
                Icon::Extrude
            } else if label.contains("Объед") || label.contains("Combine") {
                Icon::Union
            } else if label.contains("Зерк") || label.contains("Mirror") {
                Icon::Mirror
            } else if label.contains("Вращ") || label.contains("Revolve") {
                Icon::Revolve
            } else if label.contains("Сдвиг") || label.contains("Sweep") {
                Icon::Sweep
            } else if label.contains("Лофт") || label.contains("Loft") {
                Icon::Loft
            } else if label.contains("Массив") || label.contains("Pattern") {
                Icon::Pattern
            } else {
                cmds.first().map(|c| command_icon(c).0).unwrap_or(Icon::Body)
            };
            (icon, label.clone())
        }
    }
}

/// Число редактируемых параметров примитива.
pub fn prim_param_count(p: &Primitive) -> usize {
    match p {
        Primitive::Box { .. } | Primitive::Wedge { .. } => 3,
        Primitive::Cylinder { .. } | Primitive::Capsule { .. } | Primitive::Pyramid { .. } => 3,
        Primitive::Sphere { .. } => 2,
        Primitive::Cone { .. } | Primitive::Tube { .. } | Primitive::Torus { .. } => 4,
    }
}

/// Поля параметров примитива. Возвращает `true`, если что-то изменено.
pub fn edit_primitive(ui: &mut Ui, p: &mut Primitive, x: f32, y: &mut f32, w: f32, row: f32, key: &str) -> bool {
    let s = ui.s;
    let mm = t("мм", "mm");
    let mut changed = false;
    let base = id_of(key);
    let mut k = 0usize;
    let mut fld = |ui: &mut Ui, label: &str, v: &mut f32, unit: &str, step: f32, min: f32, max: f32, y: &mut f32| {
        let fr = ui.labeled(Rect::new(x, *y, w, 24.0 * s), label, 110.0);
        if ui.number_field(id_idx(base, k), fr, v, unit, step, min, max) {
            changed = true;
        }
        k += 1;
        *y += row;
    };
    let seg = |ui: &mut Ui, v: &mut u32, min: u32, y: &mut f32, fld: &mut dyn FnMut(&mut Ui, &str, &mut f32, &str, f32, f32, f32, &mut f32)| {
        let mut f = *v as f32;
        fld(ui, t("Сегментов", "Segments"), &mut f, "", 4.0, min as f32, 512.0, y);
        *v = (f.round() as u32).clamp(min, 512);
    };
    match p {
        Primitive::Box { w: bw, h, d } => {
            fld(ui, t("Длина (X)", "Length (X)"), bw, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Ширина (Y)", "Width (Y)"), h, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Высота (Z)", "Height (Z)"), d, mm, 1.0, 0.001, 1e5, y);
        }
        Primitive::Wedge { w: bw, h, d } => {
            fld(ui, t("Длина (X)", "Length (X)"), bw, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Ширина (Y)", "Width (Y)"), h, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Высота (Z)", "Height (Z)"), d, mm, 1.0, 0.001, 1e5, y);
        }
        Primitive::Cylinder { radius, height, segments } => {
            fld(ui, t("Радиус", "Radius"), radius, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Высота", "Height"), height, mm, 1.0, 0.001, 1e5, y);
            seg(ui, segments, 3, y, &mut fld);
        }
        Primitive::Capsule { radius, height, segments } => {
            fld(ui, t("Радиус", "Radius"), radius, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Высота цилиндра", "Cylinder height"), height, mm, 1.0, 0.0, 1e5, y);
            seg(ui, segments, 6, y, &mut fld);
        }
        Primitive::Pyramid { base: b, height, segments } => {
            fld(ui, t("Основание", "Base"), b, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Высота", "Height"), height, mm, 1.0, 0.001, 1e5, y);
            seg(ui, segments, 3, y, &mut fld);
        }
        Primitive::Sphere { radius, segments, rings } => {
            fld(ui, t("Радиус", "Radius"), radius, mm, 1.0, 0.001, 1e5, y);
            seg(ui, segments, 6, y, &mut fld);
            *rings = (*segments / 2).max(3);
        }
        Primitive::Cone { radius_bottom, radius_top, height, segments } => {
            fld(ui, t("Радиус основания", "Bottom radius"), radius_bottom, mm, 1.0, 0.0, 1e5, y);
            fld(ui, t("Радиус вершины", "Top radius"), radius_top, mm, 1.0, 0.0, 1e5, y);
            fld(ui, t("Высота", "Height"), height, mm, 1.0, 0.001, 1e5, y);
            seg(ui, segments, 3, y, &mut fld);
        }
        Primitive::Tube { radius_outer, radius_inner, height, segments } => {
            fld(ui, t("Внешний радиус", "Outer radius"), radius_outer, mm, 1.0, 0.002, 1e5, y);
            fld(ui, t("Внутренний радиус", "Inner radius"), radius_inner, mm, 1.0, 0.001, 1e5, y);
            fld(ui, t("Высота", "Height"), height, mm, 1.0, 0.001, 1e5, y);
            seg(ui, segments, 3, y, &mut fld);
            if *radius_inner >= *radius_outer {
                *radius_inner = *radius_outer * 0.9;
            }
        }
        Primitive::Torus { major_radius, minor_radius, major_segments, minor_segments } => {
            fld(ui, t("Радиус тора", "Major radius"), major_radius, mm, 1.0, 0.002, 1e5, y);
            fld(ui, t("Радиус сечения", "Minor radius"), minor_radius, mm, 1.0, 0.001, 1e5, y);
            seg(ui, major_segments, 6, y, &mut fld);
            let mut ms = *minor_segments as f32;
            fld(ui, t("Сегментов сечения", "Section segments"), &mut ms, "", 4.0, 4.0, 256.0, y);
            *minor_segments = (ms.round() as u32).clamp(4, 256);
        }
    }
    changed
}
