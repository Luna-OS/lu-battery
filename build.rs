//! Embeds the app icon and version info into the Windows exe. Owned by `code`.
//!
//! The icon comes from `assets/icon.ico` / `assets/icon.png` (owned by
//! `design`). Both are optional: without them the tray falls back to a drawn
//! icon. Sets the cfgs `lu_icon_ico` and `lu_icon_png` for the code.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lu_icon_ico)");
    println!("cargo::rustc-check-cfg=cfg(lu_icon_png)");
    println!("cargo::rerun-if-changed=assets/icon.ico");
    println!("cargo::rerun-if-changed=assets/icon.png");
    println!("cargo::rerun-if-changed=build.rs");

    let has_ico = Path::new("assets/icon.ico").is_file();
    if Path::new("assets/icon.png").is_file() {
        println!("cargo::rustc-cfg=lu_icon_png");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("FileDescription", "lu-battery")
        .set("ProductName", "lu-battery")
        .set("OriginalFilename", "lu-battery.exe");
    if has_ico {
        // Resource id 1, loaded by the tray via `Icon::from_resource(1, …)`.
        res.set_icon_with_id("assets/icon.ico", "1");
    }
    match res.compile() {
        Ok(()) => {
            if has_ico {
                println!("cargo::rustc-cfg=lu_icon_ico");
            }
        }
        // No resource compiler: the exe works, just without icon/version info.
        Err(e) => println!("cargo::warning=no Windows resources embedded: {e}"),
    }
}
