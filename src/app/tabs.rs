//! Вкладки документов: несколько проектов в одном окне.
//!
//! Данные активного документа лежат прямо в [`State`] (весь остальной код
//! работает с ними как раньше), а неактивные документы — в `State::tabs`.
//! Переключение меняет их местами через `mem::swap`, без копирования.

use super::dialogs::Dialog;
use super::meshedit::MeshEditState;
use super::{t, CacheEntry, MeasureState, SketchSession, State, Tab, Tool};
use crate::renderer::software::Camera;
use crate::scene::Scene;
use std::collections::HashMap;
use std::mem::swap;
use std::path::PathBuf;

/// Документ неактивной вкладки.
pub struct DocTab {
    scene: Scene,
    cam: Camera,
    project_path: Option<PathBuf>,
    saved_revision: u64,
    sketch: Option<SketchSession>,
    measure: MeasureState,
    dialog: Option<Dialog>,
    tool: Tool,
    tab: Tab,
    cache: HashMap<u64, CacheEntry>,
    medit: MeshEditState,
    doc_no: u32,
}

impl DocTab {
    pub fn blank() -> DocTab {
        let scene = Scene::new();
        let saved_revision = scene.revision;
        DocTab {
            scene,
            cam: Camera::default(),
            project_path: None,
            saved_revision,
            sketch: None,
            measure: MeasureState::default(),
            dialog: None,
            tool: Tool::Select,
            tab: Tab::Solid,
            cache: HashMap::new(),
            medit: MeshEditState::default(),
            doc_no: 1,
        }
    }

    fn title(&self) -> String {
        doc_title(&self.project_path, self.doc_no)
    }

    fn dirty(&self) -> bool {
        self.scene.revision != self.saved_revision && !self.scene.bodies.is_empty()
    }
}

/// Заголовок: имя файла проекта или «Без имени N».
fn doc_title(p: &Option<PathBuf>, no: u32) -> String {
    p.as_ref().and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| {
        if no <= 1 {
            t("Без имени", "Untitled").to_string()
        } else {
            format!("{} {no}", t("Без имени", "Untitled"))
        }
    })
}

impl State {
    /// Обмен данными активного документа с `d`.
    fn swap_doc(&mut self, d: &mut DocTab) {
        swap(&mut self.scene, &mut d.scene);
        swap(&mut self.cam, &mut d.cam);
        swap(&mut self.project_path, &mut d.project_path);
        swap(&mut self.saved_revision, &mut d.saved_revision);
        swap(&mut self.sketch, &mut d.sketch);
        swap(&mut self.measure, &mut d.measure);
        // Модальные окна к документу не относятся.
        if self.dialog.as_ref().map(|x| !x.is_side_panel()).unwrap_or(false) {
            self.dialog = None;
        }
        swap(&mut self.dialog, &mut d.dialog);
        swap(&mut self.tool, &mut d.tool);
        swap(&mut self.tab, &mut d.tab);
        swap(&mut self.cache, &mut d.cache);
        swap(&mut self.medit, &mut d.medit);
        swap(&mut self.doc_no, &mut d.doc_no);
        // Состояние, привязанное к телам прежнего документа.
        self.cam_anim = None;
        self.drag = None;
        self.hover_body = None;
        self.gizmo_hover = None;
        self.snap_hover = None;
        self.preview_cache = None;
        self.info_cache = None;
        self.rename = None;
        self.menu = None;
        self.invalidate_view();
    }

    /// Число вкладок.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Заголовок вкладки и признак несохранённых изменений.
    pub fn tab_title(&self, i: usize) -> (String, bool) {
        if i == self.active_tab {
            (doc_title(&self.project_path, self.doc_no), self.is_dirty())
        } else {
            self.tabs.get(i).map(|d| (d.title(), d.dirty())).unwrap_or_default()
        }
    }

    /// Активный документ пуст (новый, без тел и файла).
    pub fn doc_is_blank(&self) -> bool {
        self.scene.bodies.is_empty() && self.project_path.is_none() && self.sketch.is_none()
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i == self.active_tab || i >= self.tabs.len() {
            return;
        }
        let mut cur = std::mem::replace(&mut self.tabs[self.active_tab], DocTab::blank());
        // Сейчас в `cur` — заглушка; меняем: активный документ уходит в слот,
        // документ вкладки `i` становится активным.
        self.swap_doc(&mut cur);
        self.tabs[self.active_tab] = cur;
        let mut next = std::mem::replace(&mut self.tabs[i], DocTab::blank());
        self.swap_doc(&mut next);
        self.active_tab = i;
        self.wake_ms = Some(0);
    }

    /// Новая пустая вкладка (становится активной).
    pub fn new_tab(&mut self) {
        self.tabs.push(DocTab::blank());
        let i = self.tabs.len() - 1;
        self.switch_tab(i);
        self.next_doc_no += 1;
        self.doc_no = self.next_doc_no;
        self.cam = Camera::default();
        self.invalidate_view();
    }

    /// Закрытие вкладки. Несохранённые изменения — с подтверждением.
    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.switch_tab(i);
        if self.is_dirty() {
            self.dialog = Some(Dialog::ConfirmDiscard { then: super::actions::Action::CloseTab(i) });
            return;
        }
        if self.tabs.len() == 1 {
            // Последняя вкладка: просто новый документ.
            self.reset_document();
            return;
        }
        // Активной становится соседняя вкладка, затем слот удаляется.
        let next = if i + 1 < self.tabs.len() { i + 1 } else { i - 1 };
        self.switch_tab(next);
        self.tabs.remove(i);
        if self.active_tab > i {
            self.active_tab -= 1;
        }
        self.invalidate_view();
    }

    /// Первая (кроме активной) вкладка с несохранёнными изменениями.
    pub fn other_dirty_tab(&self) -> Option<usize> {
        (0..self.tabs.len()).find(|&i| i != self.active_tab && self.tabs[i].dirty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;
    use crate::scene::Body;

    fn state() -> State {
        super::super::App::new(Vec::new()).st
    }

    #[test]
    fn tabs_keep_documents_apart() {
        let mut st = state();
        st.scene.add_body(Body::new("A", primitive_box(1.0, 1.0, 1.0)));
        st.new_tab();
        assert_eq!(st.tab_count(), 2);
        assert_eq!(st.active_tab, 1);
        assert!(st.scene.bodies.is_empty(), "новая вкладка пуста");
        st.scene.add_body(Body::new("B", primitive_box(1.0, 1.0, 1.0)));
        st.scene.add_body(Body::new("C", primitive_box(1.0, 1.0, 1.0)));
        st.switch_tab(0);
        assert_eq!(st.scene.bodies.len(), 1);
        assert_eq!(st.scene.bodies[0].name, "A");
        assert!(st.tab_title(1).1, "вкладка 2 изменена");
        st.switch_tab(1);
        assert_eq!(st.scene.bodies.len(), 2);
    }

    #[test]
    fn closing_dirty_tab_asks() {
        let mut st = state();
        st.new_tab();
        st.scene.add_body(Body::new("B", primitive_box(1.0, 1.0, 1.0)));
        st.switch_tab(0);
        st.close_tab(1);
        assert_eq!(st.tab_count(), 2, "вкладка с изменениями не закрыта без вопроса");
        assert!(matches!(st.dialog, Some(Dialog::ConfirmDiscard { .. })));
        assert_eq!(st.active_tab, 1);
        // «Не сохранять».
        st.dialog = None;
        st.saved_revision = st.scene.revision;
        st.close_tab(1);
        assert_eq!(st.tab_count(), 1);
        assert_eq!(st.active_tab, 0);
        assert!(st.scene.bodies.is_empty());
    }

    #[test]
    fn close_middle_tab_keeps_order() {
        let mut st = state();
        st.scene.add_body(Body::new("A", primitive_box(1.0, 1.0, 1.0)));
        st.saved_revision = st.scene.revision;
        st.new_tab();
        st.new_tab();
        st.scene.add_body(Body::new("C", primitive_box(1.0, 1.0, 1.0)));
        st.saved_revision = st.scene.revision;
        st.close_tab(1);
        assert_eq!(st.tab_count(), 2);
        assert_eq!(st.scene.bodies[0].name, "C", "активна следующая вкладка");
        assert_eq!(st.active_tab, 1);
        st.switch_tab(0);
        assert_eq!(st.scene.bodies[0].name, "A");
    }
}
