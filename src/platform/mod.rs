//! Платформенный слой: окно, события, вывод кадра, системные диалоги.
//!
//! Реализации написаны напрямую через системные API без внешних библиотек:
//! - Windows — Win32 (user32/gdi32/comdlg32/shell32);
//! - macOS — Cocoa через Objective-C runtime + CoreGraphics;
//! - Linux — X11 (libX11 загружается динамически), диалоги через zenity/kdialog.
//!
//! Для типовой проверки Win32/Cocoa кода на Linux есть cargo-feature
//! `check-windows` / `check-macos` (только `cargo check`, без линковки).

use crate::renderer::canvas::Framebuffer;
use crate::ui::{Cursor, Key, Mods, MouseButton};
use std::path::PathBuf;

#[cfg(any(target_os = "windows", feature = "check-windows"))]
pub mod windows;

#[cfg(any(target_os = "macos", feature = "check-macos"))]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

pub mod headless;

/// Событие окна.
#[derive(Clone, Debug)]
pub enum Event {
    /// Размер области рисования в физических пикселях и масштаб (DPI).
    Resize { width: usize, height: usize, scale: f32 },
    MouseMove { x: f32, y: f32 },
    MouseButton { button: MouseButton, down: bool },
    /// Прокрутка: положительное значение — от себя (приблизить).
    Wheel { delta: f32 },
    Key { key: Key, mods: Mods, down: bool },
    Text(String),
    Modifiers(Mods),
    DropFiles(Vec<PathBuf>),
    MouseLeave,
    CloseRequested,
}

/// Результат кадра для платформы.
#[derive(Clone, Debug, Default)]
pub struct FrameOutput {
    pub title: String,
    pub cursor: Cursor,
    pub quit: bool,
    /// Перерисовать через N мс (анимации, подсказки). `Some(0)` — сразу.
    pub wake_ms: Option<u64>,
}

/// Обработчик окна (реализуется приложением).
pub trait Handler {
    fn event(&mut self, e: Event);
    /// Построение кадра. Вызывается после пачки событий или по таймеру.
    fn frame(&mut self) -> FrameOutput;
    fn framebuffer(&self) -> &Framebuffer;
}

/// Фильтр файлового диалога: (название, расширения без точки).
pub type Filter<'a> = (&'a str, &'a [&'a str]);

/// Запуск главного цикла окна.
pub fn run(handler: Box<dyn Handler>, title: &str, width: u32, height: u32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return windows::run(handler, title, width, height);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::run(handler, title, width, height);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::run(handler, title, width, height);
    }
    #[allow(unreachable_code)]
    {
        let _ = (handler, title, width, height);
        Err("платформа не поддерживается".into())
    }
}

/// Диалог открытия файлов.
pub fn open_file_dialog(title: &str, filters: &[Filter], multiple: bool) -> Vec<PathBuf> {
    if headless::is_active() {
        return Vec::new();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::open_file_dialog(title, filters, multiple);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::open_file_dialog(title, filters, multiple);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::open_file_dialog(title, filters, multiple);
    }
    #[allow(unreachable_code)]
    {
        let _ = (title, filters, multiple);
        Vec::new()
    }
}

/// Диалог сохранения файла.
pub fn save_file_dialog(title: &str, default_name: &str, filters: &[Filter]) -> Option<PathBuf> {
    if headless::is_active() {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        return windows::save_file_dialog(title, default_name, filters);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::save_file_dialog(title, default_name, filters);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::save_file_dialog(title, default_name, filters);
    }
    #[allow(unreachable_code)]
    {
        let _ = (title, default_name, filters);
        None
    }
}

/// Системное окно сообщения об ошибке (для фатальных ошибок до запуска UI).
pub fn error_box(title: &str, text: &str) {
    eprintln!("{title}: {text}");
    #[cfg(target_os = "windows")]
    windows::message_box(title, text);
    #[cfg(target_os = "macos")]
    macos::message_box(title, text);
}

/// Каталог настроек приложения (создаётся при необходимости).
pub fn config_dir() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    let dir = base.unwrap_or_else(std::env::temp_dir).join("ForgeCAD");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Язык системы — русский?
pub fn system_prefers_russian() -> bool {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return v.to_lowercase().starts_with("ru");
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        return windows::user_lang_is_russian();
    }
    #[allow(unreachable_code)]
    true
}
