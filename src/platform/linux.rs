//! Linux: оконный режим пока не реализован (целевые платформы первой версии —
//! Windows и macOS). Доступны консольные команды: `info`, `convert`, `snapshot`.

use super::{Filter, Handler};
use std::path::PathBuf;

pub fn run(_handler: Box<dyn Handler>, _title: &str, _w: u32, _h: u32) -> Result<(), String> {
    Err("оконный режим для Linux появится в следующих версиях; используйте `alphacad --help` для консольных команд".into())
}

pub fn open_file_dialog(_title: &str, _filters: &[Filter], _multiple: bool) -> Vec<PathBuf> {
    Vec::new()
}

pub fn save_file_dialog(_title: &str, _default_name: &str, _filters: &[Filter]) -> Option<PathBuf> {
    None
}
