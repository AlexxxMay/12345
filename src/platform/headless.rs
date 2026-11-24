//! Безоконный режим (CLI, тесты): системные диалоги отключены.

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Включить безоконный режим.
pub fn activate() {
    ACTIVE.store(true, Ordering::Relaxed);
}

/// Безоконный режим активен?
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed) || cfg!(test)
}
