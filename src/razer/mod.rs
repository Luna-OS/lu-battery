//! Razer device discovery and battery polling. Owned by `code`.
//!
//! A background thread enumerates Razer HID devices, reads every known device
//! with a battery every `poll_secs` (or right away on [`Command::RefreshNow`]
//! and when a device is plugged in or out) and sends the full device list to
//! the app over a channel.

pub mod barracuda;
pub mod devices;
pub mod protocol;

use crate::model::{BatteryReading, Charging, DeviceStatus, Link};
use devices::{DeviceInfo, Driver, RAZER_VID};
use eframe::egui;
use hidapi::{HidApi, HidDevice};
use std::collections::{BTreeSet, HashMap};
use std::ffi::CString;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// How often the HID device list is re-enumerated to notice hot-plug.
const HOTPLUG_INTERVAL: Duration = Duration::from_secs(3);
/// Pause between two requests to the same device, so Razer Synapse (which
/// may talk to the device at the same time) gets a turn.
const REQUEST_GAP: Duration = Duration::from_millis(25);

/// Messages from the app to the polling thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    RefreshNow,
    SetPollSecs(u32),
    Quit,
}

/// One battery value as read from a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawBattery {
    pub raw: u16,
    pub raw_max: u16,
    pub charging: Charging,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The receiver is there but the device is off / asleep / out of range.
    Asleep,
    Failed(String),
    /// Nothing new: the device may not be asked yet (rate limit), keep the
    /// last status.
    NotDue,
}

/// One HID interface (Windows: top-level collection) of a device.
#[derive(Debug, Clone)]
pub struct Interface {
    pub path: CString,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
}

/// A supported device that is currently plugged in.
#[derive(Debug, Clone)]
pub struct Present {
    pub info: &'static DeviceInfo,
    pub interfaces: Vec<Interface>,
}

/// Lists the supported devices that are plugged in, in table order.
pub fn enumerate(api: &HidApi) -> Vec<Present> {
    let mut found: Vec<Present> = Vec::new();
    for d in api.device_list() {
        if d.vendor_id() != RAZER_VID {
            continue;
        }
        let Some(info) = devices::lookup(d.product_id()) else {
            continue;
        };
        let iface = Interface {
            path: d.path().to_owned(),
            interface_number: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
        };
        match found.iter_mut().find(|p| p.info.pid == info.pid) {
            Some(p) => p.interfaces.push(iface),
            None => found.push(Present {
                info,
                interfaces: vec![iface],
            }),
        }
    }
    found.sort_by_key(|p| devices::DEVICES.iter().position(|d| d.pid == p.info.pid));
    found
}

// ---------------------------------------------------------------------------
// Standard protocol over HID feature reports

/// Sends one request and waits for its answer.
///
/// Busy answers are re-read after a short pause; answers that belong to
/// another request (Synapse) make us send the request again.
pub fn query(
    dev: &HidDevice,
    req: &[u8; protocol::REPORT_LEN],
) -> Result<protocol::Response, ReadError> {
    use protocol::ParseError;

    let mut out = [0u8; protocol::REPORT_LEN + 1];
    out[1..].copy_from_slice(req);
    let mut last = ParseError::Busy;
    for attempt in 0..3 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(100));
        }
        dev.send_feature_report(&out)
            .map_err(|e| ReadError::Failed(format!("Senden fehlgeschlagen: {e}")))?;
        // Wireless receivers need a moment to forward the request.
        thread::sleep(Duration::from_millis(40));
        for _ in 0..10 {
            let mut buf = [0u8; protocol::REPORT_LEN + 1];
            let n = dev
                .get_feature_report(&mut buf)
                .map_err(|e| ReadError::Failed(format!("Lesen fehlgeschlagen: {e}")))?;
            // `n` counts the leading report id byte.
            if n < protocol::REPORT_LEN + 1 {
                return Err(ReadError::Failed(format!("kurze Antwort ({n} Bytes)")));
            }
            let mut resp = [0u8; protocol::REPORT_LEN];
            resp.copy_from_slice(&buf[1..]);
            match protocol::parse_response(req, &resp) {
                Ok(r) => return Ok(r),
                Err(ParseError::Busy) => {
                    last = ParseError::Busy;
                    thread::sleep(Duration::from_millis(30));
                }
                Err(ParseError::Timeout) => return Err(ReadError::Asleep),
                Err(e @ (ParseError::Mismatch(_) | ParseError::BadCrc)) => {
                    last = e;
                    break;
                }
                Err(e) => return Err(ReadError::Failed(e.to_string())),
            }
        }
    }
    Err(ReadError::Failed(last.to_string()))
}

/// Reads level and charging state with the standard protocol.
pub fn read_standard(dev: &HidDevice, transaction_id: u8) -> Result<RawBattery, ReadError> {
    let level = query(dev, &protocol::battery_request(transaction_id))?;
    thread::sleep(REQUEST_GAP);
    let raw = u16::from(level.args[1]);
    let charging = match query(dev, &protocol::charging_request(transaction_id)) {
        Ok(r) if r.args[1] == 0 => Charging::Discharging,
        Ok(_) if raw == 255 => Charging::Full,
        Ok(_) => Charging::Charging,
        Err(ReadError::Asleep) => return Err(ReadError::Asleep),
        Err(e) => {
            log::warn!("charging state: {e:?}");
            Charging::Unknown
        }
    };
    Ok(RawBattery {
        raw,
        raw_max: 255,
        charging,
    })
}

/// The interfaces of a device in the order worth trying: the remembered one,
/// then interface 0 / the mouse collection, then everything else.
fn candidate_order<'a>(p: &'a Present, preferred: Option<&CString>) -> Vec<&'a Interface> {
    let mut v: Vec<&Interface> = p.interfaces.iter().collect();
    v.sort_by_key(|i| {
        if Some(&i.path) == preferred {
            0
        } else if i.interface_number == 0 && i.usage_page == 0x01 && i.usage == 0x02 {
            1
        } else if i.interface_number == 0 {
            2
        } else {
            3
        }
    });
    v
}

/// Reads one plugged-in device. `preferred` remembers the interface that
/// worked last time.
fn read_device(
    api: &HidApi,
    p: &Present,
    preferred: &mut Option<CString>,
    listener: Option<&barracuda::Listener>,
    allow_active: bool,
) -> Result<RawBattery, ReadError> {
    match p.info.driver {
        Driver::Standard { transaction_id } => {
            let mut last = ReadError::Failed("kein passendes Interface".into());
            for iface in candidate_order(p, preferred.as_ref()) {
                let dev = match api.open_path(&iface.path) {
                    Ok(d) => d,
                    Err(e) => {
                        last = ReadError::Failed(format!("öffnen: {e}"));
                        continue;
                    }
                };
                match read_standard(&dev, transaction_id) {
                    Ok(b) => {
                        *preferred = Some(iface.path.clone());
                        return Ok(b);
                    }
                    Err(ReadError::Asleep) => {
                        *preferred = Some(iface.path.clone());
                        return Err(ReadError::Asleep);
                    }
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
        Driver::Barracuda => barracuda::read(api, &p.interfaces, listener, allow_active),
    }
}

// ---------------------------------------------------------------------------
// Polling thread

pub struct Handle {
    pub commands: Sender<Command>,
    pub updates: Receiver<Vec<DeviceStatus>>,
}

pub fn spawn(ctx: egui::Context, poll_secs: u32) -> Handle {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (upd_tx, upd_rx) = mpsc::channel();
    let self_tx = cmd_tx.clone();
    thread::Builder::new()
        .name("razer-poll".into())
        .spawn(move || poll_loop(ctx, poll_secs, cmd_rx, self_tx, upd_tx))
        .expect("spawn polling thread");
    Handle {
        commands: cmd_tx,
        updates: upd_rx,
    }
}

fn poll_interval(secs: u32) -> Duration {
    Duration::from_secs(u64::from(secs.clamp(5, 300)))
}

fn poll_loop(
    ctx: egui::Context,
    poll_secs: u32,
    commands: Receiver<Command>,
    self_tx: Sender<Command>,
    updates: Sender<Vec<DeviceStatus>>,
) {
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            log::error!("hidapi: {e}");
            return;
        }
    };
    let mut interval = poll_interval(poll_secs);
    let mut poller = Poller {
        refresh: self_tx,
        ..Poller::default()
    };
    let mut next_poll = Instant::now();
    let mut next_scan = Instant::now() + HOTPLUG_INTERVAL;
    let mut present = enumerate(&api);
    let mut force = true;

    loop {
        let now = Instant::now();
        if now >= next_scan {
            next_scan = now + HOTPLUG_INTERVAL;
            if let Err(e) = api.refresh_devices() {
                log::warn!("refresh_devices: {e}");
            }
            let fresh = enumerate(&api);
            if pids(&fresh) != pids(&present) {
                log::info!("device set changed");
                force = true;
            }
            present = fresh;
        }
        if force || now >= next_poll {
            force = false;
            let list = poller.poll(&api, &present, interval);
            if updates.send(list).is_err() {
                return;
            }
            ctx.request_repaint();
            next_poll = Instant::now() + interval;
        }

        let wait = next_poll
            .min(next_scan)
            .saturating_duration_since(Instant::now());
        let first = commands.recv_timeout(wait);
        // Several queued refresh requests make one refresh.
        let mut next = Some(first);
        while let Some(cmd) = next.take() {
            if !handle_command(cmd, &mut interval, &mut next_poll, &mut next_scan, &mut force) {
                return;
            }
            next = commands.try_recv().ok().map(Ok);
        }
    }
}

/// Applies one command; `false` means the thread should stop.
fn handle_command(
    cmd: Result<Command, RecvTimeoutError>,
    interval: &mut Duration,
    next_poll: &mut Instant,
    next_scan: &mut Instant,
    force: &mut bool,
) -> bool {
    {
        match cmd {
            Ok(Command::RefreshNow) => {
                // Re-enumerate too, so a freshly plugged dongle shows up.
                *next_scan = Instant::now();
                *force = true;
            }
            Ok(Command::SetPollSecs(s)) => {
                let new = poll_interval(s);
                if new != *interval {
                    *next_poll = *next_poll - *interval + new;
                    *interval = new;
                }
            }
            Ok(Command::Quit) | Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    true
}

fn pids(p: &[Present]) -> BTreeSet<u16> {
    p.iter().map(|p| p.info.pid).collect()
}

/// Keeps the last good reading per device across polls.
struct Poller {
    last: HashMap<String, DeviceStatus>,
    preferred: HashMap<u16, CString>,
    /// Passive listeners of Barracuda receivers, by product id.
    listeners: HashMap<u16, barracuda::Listener>,
    /// When each Barracuda was last queried actively.
    last_active: HashMap<u16, Instant>,
    /// Lets a listener ask for a refresh after a headset event.
    refresh: Sender<Command>,
}

impl Default for Poller {
    fn default() -> Self {
        Self {
            last: HashMap::new(),
            preferred: HashMap::new(),
            listeners: HashMap::new(),
            last_active: HashMap::new(),
            refresh: mpsc::channel().0,
        }
    }
}

impl Poller {
    fn poll(&mut self, api: &HidApi, present: &[Present], interval: Duration) -> Vec<DeviceStatus> {
        self.update_listeners(api, present);
        // Group wired + wireless variants of the same device; wired first,
        // because the cable connection also reports charging reliably.
        let mut groups: Vec<(String, Vec<&Present>)> = Vec::new();
        for p in present {
            let id = p.info.id();
            match groups.iter_mut().find(|(gid, _)| *gid == id) {
                Some((_, v)) => v.push(p),
                None => groups.push((id, vec![p])),
            }
        }
        let mut out = Vec::new();
        for (id, mut variants) in groups {
            variants.sort_by_key(|p| !p.info.wired);
            let mut result = Err(ReadError::Failed("nicht gelesen".into()));
            for p in &variants {
                let mut pref = self.preferred.get(&p.info.pid).cloned();
                // The Barracuda's active query is asked at most once per
                // poll interval, whatever triggered this poll.
                let allow_active = match p.info.driver {
                    Driver::Standard { .. } => true,
                    Driver::Barracuda => self
                        .last_active
                        .get(&p.info.pid)
                        .is_none_or(|t| t.elapsed() + Duration::from_secs(1) >= interval),
                };
                if allow_active && p.info.driver == Driver::Barracuda {
                    self.last_active.insert(p.info.pid, Instant::now());
                }
                let listener = self.listeners.get(&p.info.pid);
                result = read_device(api, p, &mut pref, listener, allow_active);
                if let Some(path) = pref {
                    self.preferred.insert(p.info.pid, path);
                }
                if result.is_ok() {
                    break;
                }
                thread::sleep(REQUEST_GAP);
            }
            let info = variants[0].info;
            let prev = self.last.get(&id).and_then(|s| s.battery);
            let status = match result {
                Err(ReadError::NotDue) if self.last.contains_key(&id) => {
                    out.push(self.last[&id].clone());
                    continue;
                }
                Err(ReadError::NotDue) => DeviceStatus {
                    id: id.clone(),
                    name: info.name.into(),
                    kind: info.kind,
                    link: Link::Error("noch nicht gelesen".into()),
                    battery: None,
                },
                Ok(b) => DeviceStatus {
                    id: id.clone(),
                    name: info.name.into(),
                    kind: info.kind,
                    link: Link::Online,
                    battery: Some(BatteryReading {
                        raw: b.raw,
                        raw_max: b.raw_max,
                        charging: b.charging,
                        read_at: Instant::now(),
                    }),
                },
                Err(ReadError::Asleep) => DeviceStatus {
                    id: id.clone(),
                    name: info.name.into(),
                    kind: info.kind,
                    link: Link::Asleep,
                    battery: prev,
                },
                Err(ReadError::Failed(msg)) => {
                    log::warn!("{id}: {msg}");
                    DeviceStatus {
                        id: id.clone(),
                        name: info.name.into(),
                        kind: info.kind,
                        link: Link::Error(msg),
                        battery: prev,
                    }
                }
            };
            self.last.insert(id, status.clone());
            out.push(status);
            thread::sleep(REQUEST_GAP);
        }
        // Forget devices that were unplugged.
        self.last.retain(|id, _| out.iter().any(|s| &s.id == id));
        out
    }

    fn update_listeners(&mut self, api: &HidApi, present: &[Present]) {
        self.listeners
            .retain(|pid, l| l.is_alive() && present.iter().any(|p| p.info.pid == *pid));
        for p in present {
            if p.info.driver != Driver::Barracuda || self.listeners.contains_key(&p.info.pid) {
                continue;
            }
            let tx = self.refresh.clone();
            let on_event = move || {
                let _ = tx.send(Command::RefreshNow);
            };
            if let Some(l) = barracuda::Listener::start(api, &p.interfaces, on_event) {
                self.listeners.insert(p.info.pid, l);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// --probe

/// Prints every Razer HID interface and the raw answers of all supported
/// devices. Debug aid for `lu-battery --probe`.
pub fn probe() -> anyhow::Result<()> {
    let api = HidApi::new()?;
    println!("Razer HID interfaces (VID 1532):");
    let mut all: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == RAZER_VID)
        .collect();
    all.sort_by_key(|d| (d.product_id(), d.interface_number(), d.usage_page(), d.usage()));
    for d in &all {
        let known = devices::lookup(d.product_id())
            .map(|i| i.name)
            .unwrap_or("(kein Akku / unbekannt)");
        println!(
            "  {:04x}:{:04x} MI_{:02} usage {:04x}/{:04x}  {:<24} {}",
            d.vendor_id(),
            d.product_id(),
            d.interface_number(),
            d.usage_page(),
            d.usage(),
            known,
            d.path().to_string_lossy()
        );
    }
    println!();
    for p in enumerate(&api) {
        println!("{} ({:04x}) [{:?}]", p.info.name, p.info.pid, p.info.driver);
        match p.info.driver {
            Driver::Standard { transaction_id } => {
                for iface in candidate_order(&p, None) {
                    print!(
                        "  MI_{:02} {:04x}/{:04x}: ",
                        iface.interface_number, iface.usage_page, iface.usage
                    );
                    let dev = match api.open_path(&iface.path) {
                        Ok(d) => d,
                        Err(e) => {
                            println!("öffnen fehlgeschlagen: {e}");
                            continue;
                        }
                    };
                    let req = protocol::battery_request(transaction_id);
                    match query(&dev, &req) {
                        Ok(r) => {
                            print!("battery args = {:02x?} ", &r.args[..4]);
                            match read_standard(&dev, transaction_id) {
                                Ok(b) => {
                                    println!(
                                        "→ raw {}/{} = {:.1} %, {:?}",
                                        b.raw,
                                        b.raw_max,
                                        f32::from(b.raw) * 100.0 / f32::from(b.raw_max),
                                        b.charging
                                    );
                                    break;
                                }
                                Err(e) => println!("→ {e:?}"),
                            }
                        }
                        Err(e) => println!("{e:?}"),
                    }
                    thread::sleep(REQUEST_GAP);
                }
            }
            Driver::Barracuda => barracuda::probe(&api, &p.interfaces),
        }
    }
    Ok(())
}
