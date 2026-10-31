//! One exe, two modes: the tiny overlay (default) and the settings window
//! (`--settings`). The settings window runs as its own process so its memory
//! is released when it closes.

#![windows_subsystem = "windows"]

mod daemon;
mod settings;

fn main() {
    // There is no console in a windowed app, so keep crashes in a log file.
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::create_dir_all(xh::config::dir());
        let _ = std::fs::write(xh::config::dir().join("crash.log"), format!("{info}\n"));
    }));
    if std::env::args().any(|a| a == "--settings") {
        settings::run();
    } else {
        daemon::run();
    }
}
