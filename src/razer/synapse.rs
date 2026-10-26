//! Watches Razer Synapse 4's log files for battery news.
//!
//! Synapse talks to the devices anyway and logs every battery notification it
//! gets. lu-battery uses that only as a *trigger*: when Synapse reports a new
//! level or charging state for a device, that device is read over HID right
//! away instead of waiting for the next poll. Only when the HID read fails is
//! the Synapse value shown (`ReadSource::Synapse`, whole percent, raw_max 100).
//!
//! Files (in `%LOCALAPPDATA%\Razer\RazerAppEngine\User Data\Logs`):
//! - `products_<product id>_mw {<container id>}<n>.log`: the device's
//!   middleware log. `<n>` grows when Synapse rotates the file.
//! - `systray_systrayv<n>.log`: device dumps of the tray process.
//!
//! What the lines look like (checked on the user's machine, 2026-10-26):
//!
//! ```text
//! … "eventIdDesc":"BATTERY_CHANGE_EVENT","eventValue":{"batteryLevel":77,"chargingStatus":1,
//!   "chargingStatusDesc":"CHARGING","powerSourceStatus":1,"powerSourceStatusDesc":"EXTERNAL_POWER_CONNECTED",…},
//!   "productId":171,"productName":"Razer Basilisk V3 Pro" …
//! … "eventIdDesc":"DEVICE_EXTERNAL_POWER_SOURCE_CHANGE_EVENT","eventValue":{"value":0,
//!   "valueDesc":"EXTERNAL_POWER_REMOVED"}},"productId":171 …
//! … "eventIdDesc":"WIRELESS_CHANGE_EVENT","eventValue":{"state":3,"stateDesc":"CONNECT_EVENT",…},"productId":171 …
//! … "realProductId":170, … "powerStatus":{"chargingStatus":"Charging","level":77} …
//! ```
//!
//! Product ids are the USB PIDs in decimal: 171 = 0x00AB (Basilisk V3 Pro via
//! dongle, used in events), 170 = 0x00AA (its wired PID, Synapse's
//! `realProductId` for the mouse in dumps and file names), 688 = 0x02B0
//! (Huntsman V3 Pro Mini, no battery). Both 170 and 171 map to our id
//! "1532:00ab" through the device table. The Barracuda X (0x0550 = 1360) does
//! not show up in Synapse 4 at all.
//!
//! `chargingStatusDesc` `NO_CHARGE_BATTERY_FULL` / `chargingStatus`
//! `NoCharge_BatteryFull` is what Synapse logs for "not charging" at any level
//! (seen at 73 % with the cable removed), so it maps to `Discharging`.
//! `"chargingStatus":"off","level":0` means no value and is skipped.

use super::devices;
use crate::model::Charging;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const TICK: Duration = Duration::from_millis(500);
/// Read at most this much new data per file and tick.
const MAX_CHUNK: u64 = 4 * 1024 * 1024;

/// Something Synapse logged about a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Level and charging state.
    Battery {
        product_id: u16,
        percent: u8,
        charging: Charging,
    },
    /// Cable plugged/pulled or wireless (dis)connect: worth a fresh read.
    Changed { product_id: u16 },
}

/// The latest Synapse value of one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynapseValue {
    pub percent: u8,
    pub charging: Charging,
    pub at: Instant,
}

/// Latest values by our device id ("1532:00ab").
pub type Values = Arc<Mutex<HashMap<String, SynapseValue>>>;

/// `%LOCALAPPDATA%\Razer\RazerAppEngine\User Data\Logs`.
pub fn log_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| {
        d.data_local_dir()
            .join("Razer")
            .join("RazerAppEngine")
            .join("User Data")
            .join("Logs")
    })
}

fn is_watched(name: &str) -> bool {
    name.ends_with(".log")
        && (name.starts_with("systray_") || (name.starts_with("products_") && name.contains("_mw")))
}

/// Starts the watcher thread. `on_change(device_id)` is called when a device's
/// level or charging state changed (or it was plugged / connected).
/// Missing log folder or no Synapse: the thread just keeps looking quietly.
pub fn start(on_change: impl Fn(&str) + Send + 'static) -> Values {
    let values: Values = Arc::default();
    let shared = values.clone();
    let spawned = thread::Builder::new()
        .name("synapse-log".into())
        .spawn(move || {
            let mut watcher = Watcher::default();
            loop {
                if let Some(dir) = log_dir() {
                    for event in watcher.tick(&dir) {
                        if let Some(id) = apply(&shared, event) {
                            on_change(&id);
                        }
                    }
                }
                thread::sleep(TICK);
            }
        });
    if let Err(e) = spawned {
        log::warn!("synapse watcher: {e}");
    }
    values
}

/// Stores a battery value; returns the device id if something changed that
/// deserves a fresh HID read.
fn apply(values: &Values, event: Event) -> Option<String> {
    match event {
        Event::Battery {
            product_id,
            percent,
            charging,
        } => {
            let id = devices::lookup(product_id)?.id();
            let mut map = values.lock().unwrap_or_else(|e| e.into_inner());
            let new = SynapseValue {
                percent,
                charging,
                at: Instant::now(),
            };
            let changed = map
                .get(&id)
                .is_none_or(|old| old.percent != percent || old.charging != charging);
            map.insert(id.clone(), new);
            changed.then_some(id)
        }
        Event::Changed { product_id } => Some(devices::lookup(product_id)?.id()),
    }
}

/// Synapse rotates `name.log` → `name1.log` → `name2.log` …; it writes to the
/// one with the highest number. Returns that file of every group.
fn newest_per_group(paths: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut groups: HashMap<String, (u32, PathBuf)> = HashMap::new();
    for path in paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_watched(name) {
            continue;
        }
        let stem = name.trim_end_matches(".log");
        let key = stem.trim_end_matches(|c: char| c.is_ascii_digit());
        let n = stem[key.len()..].parse().unwrap_or(0);
        let key = key.to_string();
        match groups.get(&key) {
            Some((old, _)) if *old >= n => {}
            _ => {
                groups.insert(key, (n, path));
            }
        }
    }
    groups.into_values().map(|(_, p)| p).collect()
}

/// Follows the appended part of the newest file of every log group.
#[derive(Default)]
struct Watcher {
    files: HashMap<PathBuf, Tail>,
    started: bool,
}

struct Tail {
    offset: u64,
    /// Bytes after the last newline, kept until the line is complete.
    partial: Vec<u8>,
}

impl Watcher {
    fn tick(&mut self, dir: &Path) -> Vec<Event> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let first = !self.started;
        self.started = true;
        let mut events = Vec::new();
        let mut seen = Vec::new();
        for path in newest_per_group(entries.flatten().map(|e| e.path())) {
            // Not `DirEntry::metadata()`: for files another process keeps
            // open (Synapse does), the directory listing has a stale size.
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            seen.push(path.clone());
            let len = meta.len();
            let tail = self.files.entry(path.clone()).or_insert_with(|| Tail {
                // Files that exist at start: only what comes after now.
                // Files that appear later (rotation): everything.
                offset: if first { len } else { 0 },
                partial: Vec::new(),
            });
            if len < tail.offset {
                // Truncated or replaced.
                tail.offset = 0;
                tail.partial.clear();
            }
            if len == tail.offset {
                continue;
            }
            if let Err(e) = read_new(&path, tail, len, &mut events) {
                log::debug!("{}: {e}", path.display());
            }
        }
        self.files.retain(|p, _| seen.contains(p));
        events
    }
}

fn read_new(
    path: &Path,
    tail: &mut Tail,
    len: u64,
    events: &mut Vec<Event>,
) -> std::io::Result<()> {
    let mut file = fs::File::open(path)?;
    let start = tail.offset.max(len.saturating_sub(MAX_CHUNK));
    if start > tail.offset {
        tail.partial.clear();
    }
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.take(len - start).read_to_end(&mut buf)?;
    tail.offset = start + buf.len() as u64;
    tail.partial.extend_from_slice(&buf);
    let Some(last_nl) = tail.partial.iter().rposition(|&b| b == b'\n') else {
        return Ok(());
    };
    let rest = tail.partial.split_off(last_nl + 1);
    for line in tail.partial.split(|&b| b == b'\n') {
        parse_line(&String::from_utf8_lossy(line), events);
    }
    tail.partial = rest;
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing

/// The number right after `key` at `pos`, e.g. `"level":77`.
fn number_at(s: &str, pos: usize) -> Option<u32> {
    let digits: String = s[pos..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The string right after `key` at `pos`, e.g. `"Charging"`.
fn string_at(s: &str, pos: usize) -> Option<&str> {
    let rest = s[pos..].strip_prefix('"')?;
    rest.find('"').map(|end| &rest[..end])
}

/// Value of the first `key` (e.g. `"productId":`) in `s`.
fn first_number(s: &str, key: &str) -> Option<u32> {
    s.find(key).and_then(|i| number_at(s, i + key.len()))
}

fn charging_from(desc: &str) -> Option<Charging> {
    let d = desc.to_ascii_lowercase();
    if d == "off" {
        None
    } else if d.starts_with("charging") {
        Some(Charging::Charging)
    } else if d.starts_with("nocharge") || d.starts_with("no_charge") {
        Some(Charging::Discharging)
    } else {
        Some(Charging::Unknown)
    }
}

fn product_id(n: u32) -> Option<u16> {
    u16::try_from(n).ok()
}

pub fn parse_line(line: &str, out: &mut Vec<Event>) {
    // Notification events: the product id follows the event value.
    for (i, _) in line.match_indices("\"eventIdDesc\":\"") {
        let rest = &line[i..];
        let Some(pid) = first_number(rest, "\"productId\":").and_then(product_id) else {
            continue;
        };
        if rest.starts_with("\"eventIdDesc\":\"BATTERY_CHANGE_EVENT\"") {
            let level = first_number(rest, "\"batteryLevel\":");
            let charging = rest
                .find("\"chargingStatusDesc\":")
                .and_then(|j| string_at(rest, j + "\"chargingStatusDesc\":".len()))
                .and_then(charging_from);
            if let (Some(level @ 0..=100), Some(charging)) = (level, charging) {
                out.push(Event::Battery {
                    product_id: pid,
                    percent: level as u8,
                    charging,
                });
            }
        } else if rest.starts_with("\"eventIdDesc\":\"DEVICE_EXTERNAL_POWER_SOURCE_CHANGE_EVENT\"")
            || rest.starts_with("\"eventIdDesc\":\"WIRELESS_CHANGE_EVENT\"")
        {
            out.push(Event::Changed { product_id: pid });
        }
    }

    // Device dumps: the product id comes before `powerStatus`.
    const POWER: &str = "\"powerStatus\":{\"chargingStatus\":";
    for (i, _) in line.match_indices(POWER) {
        let before = &line[..i];
        let pid = ["\"realProductId\":", "\"productId\":"]
            .iter()
            .filter_map(|k| before.rfind(k).map(|j| (j, j + k.len())))
            .max_by_key(|(j, _)| *j)
            .and_then(|(_, at)| number_at(line, at))
            .and_then(product_id);
        let Some(pid) = pid else { continue };
        let after = i + POWER.len();
        let Some(charging) = string_at(line, after).and_then(charging_from) else {
            continue;
        };
        let level = first_number(&line[after..], "\"level\":");
        if let Some(level @ 0..=100) = level {
            out.push(Event::Battery {
                product_id: pid,
                percent: level as u8,
                charging,
            });
        }
    }
}

/// Modification time helper for `--probe`.
fn modified(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// `--probe`: the latest battery value per device found in the newest logs.
pub fn probe() {
    let Some(dir) = log_dir() else { return };
    let Ok(entries) = fs::read_dir(&dir) else {
        println!("Synapse: kein Log-Ordner ({})", dir.display());
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_watched))
        .map(|e| e.path())
        .collect();
    files.sort_by_key(|p| std::cmp::Reverse(modified(p)));
    let mut latest: HashMap<u16, (Event, String)> = HashMap::new();
    // Oldest first, so later lines overwrite earlier ones.
    for path in files.iter().take(4).rev() {
        let Ok(bytes) = fs::read(path) else { continue };
        let start = bytes.len().saturating_sub(2 * 1024 * 1024);
        let mut events = Vec::new();
        for line in bytes[start..].split(|&b| b == b'\n') {
            parse_line(&String::from_utf8_lossy(line), &mut events);
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        for e in events {
            if let Event::Battery { product_id, .. } = e {
                latest.insert(product_id, (e, name.to_string()));
            }
        }
    }
    println!("Synapse-Logs ({}):", dir.display());
    if latest.is_empty() {
        println!("  keine Akkuwerte gefunden");
    }
    let mut rows: Vec<_> = latest.into_iter().collect();
    rows.sort_by_key(|(pid, _)| *pid);
    for (pid, (e, file)) in rows {
        let ours = devices::lookup(pid)
            .map(|d| format!("→ {} ({})", d.name, d.id()))
            .unwrap_or_else(|| "→ nicht in unserer Tabelle".into());
        println!("  productId {pid} = 0x{pid:04x} {ours}: {e:?}  [{file}]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &str = r#"[2026/10/26 11:51:05.109] info: [mw-transport] receive {"rawBuffer":[5,49,197,1,1,1,0,0,0,0,0,0,0,0,0,0],"containerId":"{6537BA35-0F00-5DF2-A437-F64B0BFB4008}","typeId":1,"recordId":5,"recordIdDesc":"NOTIFICATION_EVENTS","data":{"eventId":49,"eventIdDesc":"BATTERY_CHANGE_EVENT","eventValue":{"batteryLevel":77,"chargingStatus":1,"chargingStatusDesc":"CHARGING","powerSourceStatus":1,"powerSourceStatusDesc":"EXTERNAL_POWER_CONNECTED","deviceType":1,"deviceTypeDesc":"MOUSE"}},"productId":171,"productName":"Razer Basilisk V3 Pro","serialNumber":""} --> {"recordId":5}"#;

    #[test]
    fn battery_event() {
        let mut ev = Vec::new();
        parse_line(EVENT, &mut ev);
        assert_eq!(
            ev,
            vec![Event::Battery {
                product_id: 171,
                percent: 77,
                charging: Charging::Charging
            }]
        );
    }

    #[test]
    fn not_charging_event() {
        let line = r#"{"eventIdDesc":"BATTERY_CHANGE_EVENT","eventValue":{"batteryLevel":73,"chargingStatus":0,"chargingStatusDesc":"NO_CHARGE_BATTERY_FULL","powerSourceStatus":0,"powerSourceStatusDesc":"EXTERNAL_POWER_REMOVED"}},"productId":171,"productName":"Razer Basilisk V3 Pro"}"#;
        let mut ev = Vec::new();
        parse_line(line, &mut ev);
        assert_eq!(
            ev,
            vec![Event::Battery {
                product_id: 171,
                percent: 73,
                charging: Charging::Discharging
            }]
        );
    }

    #[test]
    fn power_and_wireless_events() {
        let line = r#"{"eventIdDesc":"DEVICE_EXTERNAL_POWER_SOURCE_CHANGE_EVENT","eventValue":{"value":0,"valueDesc":"EXTERNAL_POWER_REMOVED"}},"productId":171,"productName":"x"} {"eventIdDesc":"WIRELESS_CHANGE_EVENT","eventValue":{"state":3,"stateDesc":"CONNECT_EVENT"}},"productId":171}"#;
        let mut ev = Vec::new();
        parse_line(line, &mut ev);
        assert_eq!(
            ev,
            vec![
                Event::Changed { product_id: 171 },
                Event::Changed { product_id: 171 }
            ]
        );
    }

    #[test]
    fn device_dump() {
        let line = r#"[{"serialNumber":"PM2405F97200794","productId":688,"name":"kbd"},{"category":"MOUSE","realProductId":170,"subCategory":"","supportXYDPI":true,"powerStatus":{"chargingStatus":"NoCharge_BatteryFull","level":73},"firmwareInfo":{}},{"powerStatus":{"chargingStatus":"off","level":0},"productId":1}]"#;
        let mut ev = Vec::new();
        parse_line(line, &mut ev);
        assert_eq!(
            ev,
            vec![Event::Battery {
                product_id: 170,
                percent: 73,
                charging: Charging::Discharging
            }]
        );
    }

    #[test]
    fn ids_map_to_our_device() {
        let values: Values = Arc::default();
        let ev = |pid, pct| Event::Battery {
            product_id: pid,
            percent: pct,
            charging: Charging::Charging,
        };
        assert_eq!(apply(&values, ev(171, 50)), Some("1532:00ab".to_string()));
        // Same value via the wired id: no change.
        assert_eq!(apply(&values, ev(170, 50)), None);
        assert_eq!(apply(&values, ev(170, 51)), Some("1532:00ab".to_string()));
        // Huntsman: not ours.
        assert_eq!(apply(&values, ev(688, 51)), None);
        assert_eq!(values.lock().unwrap()["1532:00ab"].percent, 51);
    }

    #[test]
    fn newest_file_per_group() {
        let names = [
            "systray_systrayv24.log",
            "systray_systrayv25.log",
            "products_688_mw {B}4.log",
            "products_688_mw {B}5.log",
            "products_170_mw {A}.log",
            "products_170_ui {A}.log",
            "background-manager5.log",
        ];
        let mut got: Vec<String> = newest_per_group(names.iter().map(PathBuf::from))
            .into_iter()
            .map(|p| p.display().to_string())
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                "products_170_mw {A}.log",
                "products_688_mw {B}5.log",
                "systray_systrayv25.log"
            ]
        );
    }

    #[test]
    fn tails_only_new_complete_lines() {
        let dir = std::env::temp_dir().join(format!("lu-battery-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("products_170_mw {X}.log");
        fs::write(&file, format!("{EVENT}\n")).unwrap();
        let mut w = Watcher::default();
        assert!(w.tick(&dir).is_empty(), "existing content is skipped");
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        use std::io::Write;
        let (a, b) = EVENT.split_at(100);
        write!(f, "{a}").unwrap();
        assert!(w.tick(&dir).is_empty(), "half a line waits");
        writeln!(f, "{b}").unwrap();
        assert_eq!(w.tick(&dir).len(), 1);
        // Rotation: a new file is read from the start.
        fs::write(dir.join("products_170_mw {X}1.log"), format!("{EVENT}\n")).unwrap();
        assert_eq!(w.tick(&dir).len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
