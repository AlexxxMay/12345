//! Рендеринг: 2D-холст интерфейса и программный 3D-растеризатор.
pub mod canvas;
pub mod software;

pub use canvas::{argb, rgb, Framebuffer, Rect};
pub use software::{Camera, DisplayMesh, DisplayMode, Highlight, Line3, RenderItem, RenderSettings};
