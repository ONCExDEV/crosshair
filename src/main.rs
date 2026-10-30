//! One exe, two modes: the tiny overlay (default) and the settings window
//! (`--settings`). The settings window runs as its own process so its memory
//! is released when it closes.

#![windows_subsystem = "windows"]

mod daemon;
mod settings;

fn main() {
    if std::env::args().any(|a| a == "--settings") {
        settings::run();
    } else {
        daemon::run();
    }
}
