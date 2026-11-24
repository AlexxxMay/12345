//! Рендеринг: 2D-холст интерфейса, программный 3D-растеризатор (CPU)
//! и аппаратный рендер через OpenGL (GPU).
pub mod canvas;
pub mod gpu;
pub mod software;

pub use canvas::{argb, rgb, Framebuffer, Rect};
pub use software::{Camera, DisplayMesh, DisplayMode, Highlight, Line3, RenderItem, RenderSettings};
