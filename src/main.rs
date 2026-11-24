//! AlphaCad — точка входа.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use alphacad::app::App;
use alphacad::cli::{self, Command};
use alphacad::platform;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            #[cfg(target_os = "windows")]
            platform::windows::attach_console();
            eprintln!("ошибка: {e}\n\n{}", cli::HELP);
            std::process::exit(2);
        }
    };
    match cmd {
        Command::Gui(files) => {
            let app = App::new(files);
            let title = format!("{} {}", alphacad::APP_NAME, alphacad::VERSION);
            if let Err(e) = platform::run(Box::new(app), &title, 1440, 900) {
                platform::error_box("AlphaCad", &format!("Не удалось открыть окно / Failed to open window:\n{e}"));
                std::process::exit(1);
            }
        }
        other => {
            #[cfg(target_os = "windows")]
            platform::windows::attach_console();
            std::process::exit(cli::run_console(other));
        }
    }
}
