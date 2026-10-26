// Owned by `code`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod razer;
mod settings;
mod tray;
mod ui;

use eframe::egui::{self, ViewportBuilder, WindowLevel};

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--probe") {
        attach_console();
        init_logging("warn");
        return razer::probe();
    }
    init_logging("warn");
    if !single_instance() {
        log::info!("lu-battery is already running");
        return Ok(());
    }

    let settings = settings::load();
    let (rgba, width, height) = tray::app_icon_rgba();
    let mut viewport = ViewportBuilder::default()
        .with_title("lu-battery")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_taskbar(false)
        .with_active(false)
        .with_inner_size([220.0, 60.0])
        .with_mouse_passthrough(settings.click_through)
        .with_window_level(if settings.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        })
        .with_icon(egui::IconData {
            rgba,
            width,
            height,
        });
    // Start close to the final place; the first frame moves it exactly there.
    if let Some((x, y)) =
        app::target_position(&app::monitors::all(), &settings, egui::vec2(220.0, 60.0))
    {
        viewport = viewport.with_position([x as f32, y as f32]);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        ..Default::default()
    };
    eframe::run_native(
        "lu-battery",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, settings)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

fn init_logging(default: &str) {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default)).init();
}

/// Only one overlay at a time: a named mutex that lives as long as the process.
#[cfg(windows)]
fn single_instance() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    // SAFETY: plain Win32 call; the handle is intentionally never closed.
    match unsafe { CreateMutexW(None, true, w!("Local\\lu-battery-single-instance")) } {
        Ok(_handle) => {
            // SAFETY: reads the thread's last error right after the call.
            let err = unsafe { GetLastError() };
            err != ERROR_ALREADY_EXISTS
        }
        Err(e) => {
            log::warn!("single instance mutex: {e}");
            true
        }
    }
}

#[cfg(not(windows))]
fn single_instance() -> bool {
    true
}

/// Release builds are GUI programs; give `--probe` the console it was
/// started from.
#[cfg(windows)]
fn attach_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call; fails harmlessly when there is no console.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}
