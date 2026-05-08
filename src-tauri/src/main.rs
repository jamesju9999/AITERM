// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|a| a == "--headless") {
        aiterm_lib::run_headless();
    } else {
        aiterm_lib::run();
    }
}
