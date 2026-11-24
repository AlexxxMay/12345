//! Сцена: тела, материалы, история undo/redo.

use crate::math::{Color, Mat4};
use crate::mesh::{Mesh, Primitive};
use crate::sketch::Sketch;

/// Материал тела.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub color: Color,
    /// Режим затенения: 0=плоское, 1=гладкое (Гуро), 2=каркас.
    pub shading: u8,
}

impl Default for Material {
    fn default() -> Self {
        Self { color: Color::rgb(200, 205, 215), shading: 1 }
    }
}

/// Тело в сцене.
#[derive(Clone, Debug)]
pub struct Body {
    pub id: u64,
    pub name: String,
    pub mesh: Mesh,
    pub transform: Mat4,
    pub material: Material,
    pub visible: bool,
    /// Параметры примитива, если тело создано как примитив (для редактирования).
    pub primitive: Option<Primitive>,
    /// Эскиз, если тело — эскиз.
    pub sketch: Option<Sketch>,
    /// Путь исходного файла (для импортированных моделей).
    pub source_path: Option<String>,
}

impl Body {
    pub fn new(name: impl Into<String>, mesh: Mesh) -> Self {
        Self {
            id: 0,
            name: name.into(),
            mesh,
            transform: Mat4::identity(),
            material: Material::default(),
            visible: true,
            primitive: None,
            sketch: None,
            source_path: None,
        }
    }

    /// Мировая AABB (с учётом трансформации).
    pub fn world_aabb(&self) -> crate::math::Aabb {
        self.mesh.aabb().transformed(self.transform)
    }
}

/// Команда истории (undo/redo).
#[derive(Clone, Debug)]
pub enum Command {
    AddBody { body: Body, index: usize },
    RemoveBody { body: Body, index: usize },
    TransformBody { id: u64, before: Mat4, after: Mat4 },
    RenameBody { id: u64, before: String, after: String },
    SetMaterial { id: u64, before: Material, after: Material },
    SetVisible { id: u64, before: bool, after: bool },
    ReplaceMesh { id: u64, before: Mesh, after: Mesh },
    ReplacePrimitive { id: u64, before: Primitive, after: Primitive },
    /// Полная замена тела (меш, трансформация, примитив, эскиз).
    ReplaceBody { before: Box<Body>, after: Box<Body> },
    /// Группа команд, отменяемая одним действием.
    Group { label: String, cmds: Vec<Command> },
}

impl Command {
    /// Краткое машинное имя вида команды (для временной шкалы).
    pub fn kind(&self) -> &'static str {
        match self {
            Command::AddBody { .. } => "add",
            Command::RemoveBody { .. } => "remove",
            Command::TransformBody { .. } => "transform",
            Command::RenameBody { .. } => "rename",
            Command::SetMaterial { .. } => "material",
            Command::SetVisible { .. } => "visible",
            Command::ReplaceMesh { .. } => "mesh",
            Command::ReplacePrimitive { .. } => "primitive",
            Command::ReplaceBody { .. } => "body",
            Command::Group { .. } => "group",
        }
    }
}

/// Сцена с историей.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub bodies: Vec<Body>,
    pub next_id: u64,
    pub undo_stack: Vec<Command>,
    pub redo_stack: Vec<Command>,
    pub selected: Option<u64>,
    /// Счётчик изменений (для отслеживания несохранённых правок).
    pub revision: u64,
    in_group: bool,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            next_id: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            selected: None,
            revision: 0,
            in_group: false,
        }
    }

    pub fn add_body(&mut self, mut body: Body) -> u64 {
        body.id = self.next_id;
        self.next_id += 1;
        let id = body.id;
        let index = self.bodies.len();
        self.bodies.push(body.clone());
        self.push_command(Command::AddBody { body, index });
        self.selected = Some(id);
        id
    }

    pub fn add_body_no_undo(&mut self, mut body: Body) -> u64 {
        body.id = self.next_id;
        self.next_id += 1;
        let id = body.id;
        self.bodies.push(body);
        id
    }

    pub fn remove_body(&mut self, id: u64) -> bool {
        if let Some(pos) = self.bodies.iter().position(|b| b.id == id) {
            let body = self.bodies.remove(pos);
            self.push_command(Command::RemoveBody { body, index: pos });
            if self.selected == Some(id) {
                self.selected = self.bodies.last().map(|b| b.id);
            }
            true
        } else {
            false
        }
    }

    pub fn body(&self, id: u64) -> Option<&Body> {
        self.bodies.iter().find(|b| b.id == id)
    }

    pub fn body_mut(&mut self, id: u64) -> Option<&mut Body> {
        self.bodies.iter_mut().find(|b| b.id == id)
    }

    pub fn selected_body(&self) -> Option<&Body> {
        self.selected.and_then(|id| self.body(id))
    }

    pub fn selected_body_mut(&mut self) -> Option<&mut Body> {
        let id = self.selected?;
        self.body_mut(id)
    }

    pub fn set_transform(&mut self, id: u64, after: Mat4) {
        if let Some(b) = self.body(id) {
            let before = b.transform;
            if before != after {
                if let Some(bm) = self.body_mut(id) {
                    bm.transform = after;
                }
                self.push_command(Command::TransformBody { id, before, after });
            }
        }
    }

    /// Фиксация трансформации, изменённой напрямую (перетаскивание манипулятора).
    pub fn commit_transform(&mut self, id: u64, before: Mat4) {
        if let Some(b) = self.body(id) {
            let after = b.transform;
            if before != after {
                self.push_command(Command::TransformBody { id, before, after });
            }
        }
    }

    pub fn rename(&mut self, id: u64, after: String) {
        if let Some(b) = self.body(id) {
            let before = b.name.clone();
            if before != after {
                if let Some(bm) = self.body_mut(id) {
                    bm.name = after.clone();
                }
                self.push_command(Command::RenameBody { id, before, after });
            }
        }
    }

    pub fn set_material(&mut self, id: u64, after: Material) {
        if let Some(b) = self.body(id) {
            let before = b.material;
            if before != after {
                if let Some(bm) = self.body_mut(id) {
                    bm.material = after;
                }
                self.push_command(Command::SetMaterial { id, before, after });
            }
        }
    }

    pub fn set_visible(&mut self, id: u64, after: bool) {
        if let Some(b) = self.body(id) {
            let before = b.visible;
            if before != after {
                if let Some(bm) = self.body_mut(id) {
                    bm.visible = after;
                }
                self.push_command(Command::SetVisible { id, before, after });
            }
        }
    }

    /// Замена меша (например, при редактировании параметров примитива).
    pub fn replace_mesh(&mut self, id: u64, after: Mesh) {
        if let Some(b) = self.body(id) {
            let before = b.mesh.clone();
            if let Some(bm) = self.body_mut(id) {
                bm.mesh = after.clone();
            }
            self.push_command(Command::ReplaceMesh { id, before, after });
        }
    }

    pub fn set_primitive(&mut self, id: u64, after: Primitive) {
        if let Some(b) = self.body(id) {
            if let Some(before) = b.primitive {
                if before != after {
                    let mesh = after.build();
                    if let Some(bm) = self.body_mut(id) {
                        bm.primitive = Some(after);
                        bm.mesh = mesh;
                    }
                    self.push_command(Command::ReplacePrimitive { id, before, after });
                }
            }
        }
    }

    fn push_command(&mut self, cmd: Command) {
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
        self.revision += 1;
        // Ограничиваем историю, чтобы не раздувать память.
        if !self.in_group && self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
    }

    /// Выполнение нескольких изменений как одного действия истории.
    pub fn group<R>(&mut self, label: &str, f: impl FnOnce(&mut Scene) -> R) -> R {
        let was = self.in_group;
        self.in_group = true;
        let start = self.undo_stack.len();
        let r = f(self);
        self.in_group = was;
        if self.undo_stack.len() > start {
            let cmds = self.undo_stack.split_off(start);
            if cmds.len() == 1 {
                self.undo_stack.extend(cmds);
            } else {
                self.undo_stack.push(Command::Group { label: label.to_string(), cmds });
            }
            if !was && self.undo_stack.len() > 200 {
                self.undo_stack.remove(0);
            }
        }
        r
    }

    /// Полная замена тела с сохранением в истории.
    pub fn replace_body(&mut self, after: Body) {
        if let Some(pos) = self.bodies.iter().position(|b| b.id == after.id) {
            let before = std::mem::replace(&mut self.bodies[pos], after.clone());
            self.push_command(Command::ReplaceBody { before: Box::new(before), after: Box::new(after) });
        }
    }

    /// Переупорядочивание без истории не требуется; поиск индекса тела.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.bodies.iter().position(|b| b.id == id)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(cmd) = self.undo_stack.pop() {
            self.revision += 1;
            self.apply(&cmd, false);
            self.redo_stack.push(cmd);
        }
    }

    pub fn redo(&mut self) {
        if let Some(cmd) = self.redo_stack.pop() {
            self.revision += 1;
            self.apply(&cmd, true);
            self.undo_stack.push(cmd);
        }
    }

    fn apply(&mut self, cmd: &Command, forward: bool) {
        match cmd {
            Command::AddBody { body, index } => {
                if forward {
                    let idx = (*index).min(self.bodies.len());
                    self.bodies.insert(idx, body.clone());
                    self.selected = Some(body.id);
                } else {
                    self.bodies.retain(|b| b.id != body.id);
                    if self.selected == Some(body.id) {
                        self.selected = None;
                    }
                }
            }
            Command::RemoveBody { body, index } => {
                if forward {
                    self.bodies.retain(|b| b.id != body.id);
                    if self.selected == Some(body.id) {
                        self.selected = None;
                    }
                } else {
                    let idx = (*index).min(self.bodies.len());
                    self.bodies.insert(idx, body.clone());
                }
            }
            Command::TransformBody { id, before, after } => {
                if let Some(b) = self.body_mut(*id) {
                    b.transform = if forward { *after } else { *before };
                }
            }
            Command::RenameBody { id, before, after } => {
                if let Some(b) = self.body_mut(*id) {
                    b.name = if forward { after.clone() } else { before.clone() };
                }
            }
            Command::SetMaterial { id, before, after } => {
                if let Some(b) = self.body_mut(*id) {
                    b.material = if forward { *after } else { *before };
                }
            }
            Command::SetVisible { id, before, after } => {
                if let Some(b) = self.body_mut(*id) {
                    b.visible = if forward { *after } else { *before };
                }
            }
            Command::ReplaceMesh { id, before, after } => {
                if let Some(b) = self.body_mut(*id) {
                    b.mesh = if forward { after.clone() } else { before.clone() };
                }
            }
            Command::ReplacePrimitive { id, before, after } => {
                if let Some(b) = self.body_mut(*id) {
                    let p = if forward { *after } else { *before };
                    b.primitive = Some(p);
                    b.mesh = p.build();
                }
            }
            Command::ReplaceBody { before, after } => {
                let src = if forward { after } else { before };
                if let Some(b) = self.body_mut(src.id) {
                    *b = (**src).clone();
                }
            }
            Command::Group { cmds, .. } => {
                if forward {
                    for c in cmds {
                        self.apply(c, true);
                    }
                } else {
                    for c in cmds.iter().rev() {
                        self.apply(c, false);
                    }
                }
            }
        }
    }

    /// Общая мировая AABB всех видимых тел.
    pub fn world_bounds(&self) -> crate::math::Aabb {
        let mut b = crate::math::Aabb::empty();
        for body in &self.bodies {
            if body.visible {
                b.union(&body.world_aabb());
            }
        }
        b
    }

    pub fn clear(&mut self) {
        self.bodies.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.selected = None;
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn undo_redo_add_remove() {
        let mut s = Scene::new();
        let id = s.add_body(Body::new("A", primitive_box(1.0, 1.0, 1.0)));
        assert_eq!(s.bodies.len(), 1);
        s.undo();
        assert_eq!(s.bodies.len(), 0);
        s.redo();
        assert_eq!(s.bodies.len(), 1);
        assert!(s.remove_body(id));
        assert_eq!(s.bodies.len(), 0);
        s.undo();
        assert_eq!(s.bodies.len(), 1);
    }

    #[test]
    fn undo_transform() {
        let mut s = Scene::new();
        let id = s.add_body(Body::new("A", primitive_box(1.0, 1.0, 1.0)));
        let t = Mat4::translation(crate::math::Vec3::new(5.0, 0.0, 0.0));
        s.set_transform(id, t);
        assert_eq!(s.body(id).unwrap().transform, t);
        s.undo();
        assert_eq!(s.body(id).unwrap().transform, Mat4::identity());
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use crate::mesh::Primitive;

    #[test]
    fn group_undo_is_single_step() {
        let mut sc = Scene::new();
        let p = Primitive::Box { w: 1.0, h: 1.0, d: 1.0 };
        sc.group("two", |s| {
            s.add_body(Body::new("a", p.build()));
            s.add_body(Body::new("b", p.build()));
        });
        assert_eq!(sc.bodies.len(), 2);
        assert_eq!(sc.undo_stack.len(), 1);
        sc.undo();
        assert_eq!(sc.bodies.len(), 0);
        sc.redo();
        assert_eq!(sc.bodies.len(), 2);
    }
}
