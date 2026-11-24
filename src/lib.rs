//! ForgeCAD — лёгкий 3D CAD редактор.
//!
//! Просмотр, создание и редактирование 3D-моделей форматов STL, STEP, OBJ, PLY.
//! Реализован на чистом Rust (только `std`), без внешних зависимостей:
//! компактный бинарь, быстрая сборка, работа на слабых компьютерах.
//!
//! Модули:
//! - [`math`] — векторная/матричная математика, лучи, AABB.
//! - [`mesh`] — треугольные меши, примитивы, измерения, операции.
//! - [`formats`] — чтение/запись STL, STEP, OBJ, PLY и формата проекта.
//! - [`sketch`] — 2D-эскизы и выдавливание (extrude).
//! - [`scene`] — сцена, тела, история undo/redo.
//! - [`renderer`] — программный 3D-рендерер (z-буфер, многопоточность).
//! - [`platform`] — оконная система Win32 / Cocoa / X11 через FFI.
//! - [`ui`] — immediate-mode интерфейс в стиле Fusion 360.
//! - [`app`] — состояние приложения и раскладка интерфейса.

pub mod app;
pub mod cli;
pub mod csg;
pub mod formats;
pub mod math;
pub mod mesh;
pub mod platform;
pub mod renderer;
pub mod scene;
pub mod sketch;
pub mod ui;

/// Версия приложения.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Название приложения.
pub const APP_NAME: &str = "ForgeCAD";
