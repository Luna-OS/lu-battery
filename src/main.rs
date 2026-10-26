// Owned by `code`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod model;
mod razer;
#[allow(dead_code)]
mod ui;

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--probe") {
        return razer::probe();
    }
    Ok(())
}
