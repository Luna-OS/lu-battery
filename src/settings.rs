//! Loading/saving [`Settings`] as JSON and the autostart entry. Owned by `code`.

use crate::model::Settings;
use std::fs;
use std::path::PathBuf;

const APP_DIR: &str = "lu-battery";
const FILE: &str = "settings.json";

/// `%APPDATA%\lu-battery\settings.json`.
pub fn path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.config_dir().join(APP_DIR).join(FILE))
}

/// Loads the settings. A missing file gives the defaults; a broken file is
/// kept as `settings.json.broken` and the defaults are used.
pub fn load() -> Settings {
    let Some(path) = path() else {
        return Settings::default();
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("{}: {e}", path.display());
            }
            return Settings::default();
        }
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(s) => sanitize(s),
        Err(e) => {
            log::warn!("{}: {e}; using defaults", path.display());
            let _ = fs::rename(&path, path.with_extension("json.broken"));
            Settings::default()
        }
    }
}

/// Writes the settings (via a temp file, so a crash never leaves half a file).
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let path = path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(settings)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Clamps values from a hand-edited file into their valid ranges.
pub fn sanitize(mut s: Settings) -> Settings {
    let d = Settings::default();
    let finite = |v: f32, def: f32| if v.is_finite() { v } else { def };
    s.margin = finite(s.margin, d.margin).clamp(0.0, 500.0);
    s.free_pos = [
        finite(s.free_pos[0], d.free_pos[0]),
        finite(s.free_pos[1], d.free_pos[1]),
    ];
    s.scale = finite(s.scale, d.scale).clamp(0.75, 2.0);
    s.opacity = finite(s.opacity, d.opacity).clamp(0.0, 1.0);
    s.poll_secs = s.poll_secs.clamp(2, 300);
    s.low_threshold = s.low_threshold.min(100);
    s.critical_threshold = s.critical_threshold.min(s.low_threshold);
    s
}

// ---------------------------------------------------------------------------
// Autostart: HKCU\Software\Microsoft\Windows\CurrentVersion\Run

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "lu-battery";

/// Adds or removes the autostart entry for the running exe.
#[cfg(windows)]
pub fn set_autostart(on: bool) -> anyhow::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};

    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
    if on {
        let exe = std::env::current_exe()?;
        key.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()))?;
    } else {
        match key.delete_value(RUN_VALUE) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Whether the autostart entry exists and points at this exe.
#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY) else {
        return false;
    };
    let Ok(value) = key.get_value::<String, _>(RUN_VALUE) else {
        return false;
    };
    std::env::current_exe()
        .map(|exe| {
            value
                .trim_matches('"')
                .eq_ignore_ascii_case(&exe.display().to_string())
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn set_autostart(_on: bool) -> anyhow::Result<()> {
    anyhow::bail!("autostart is only supported on Windows")
}

#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_fills_defaults() {
        let s: Settings = serde_json::from_str(r#"{"scale": 1.5, "poll_secs": 60}"#).unwrap();
        assert_eq!(s.scale, 1.5);
        assert_eq!(s.poll_secs, 60);
        assert_eq!(s.margin, Settings::default().margin);
    }

    #[test]
    fn roundtrip() {
        let s = Settings {
            hidden_devices: vec!["1532:0550".into()],
            ..Settings::default()
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn sanitize_clamps() {
        let s = sanitize(Settings {
            scale: 10.0,
            opacity: -1.0,
            poll_secs: 1,
            low_threshold: 15,
            critical_threshold: 40,
            margin: f32::NAN,
            ..Settings::default()
        });
        assert_eq!(s.scale, 2.0);
        assert_eq!(s.opacity, 0.0);
        assert_eq!(s.poll_secs, 2);
        assert_eq!(s.critical_threshold, 15);
        assert_eq!(s.margin, Settings::default().margin);
    }
}
