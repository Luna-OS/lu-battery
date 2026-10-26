//! Razer device discovery and battery polling. Owned by `code`.
//!
//! A background thread enumerates Razer HID devices, reads every known device
//! with a battery every `poll_secs` (or right away on [`Command::RefreshNow`]
//! and when a device is plugged in or out) and sends the full device list to
//! the app over a channel. When Razer Synapse logs a battery change for a
//! device (see `synapse.rs`), that device is read over HID at once.

pub mod barracuda;
pub mod devices;
pub mod protocol;
pub mod synapse;

use crate::model::{BatteryReading, Charging, DeviceStatus, Link, ReadSource};
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

/// Messages to the polling thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Re-enumerate and read every device.
    RefreshNow,
    /// Read one device (by id) right away, e.g. after Synapse logged news.
    RefreshDevice(String),
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

/// The Barracuda's E1 01 → GET → E1 00 sequence runs at most this often.
const BARRACUDA_MIN_INTERVAL: Duration = Duration::from_secs(5);

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
    Duration::from_secs(u64::from(secs.clamp(2, 300)))
}

/// What the next poll has to read.
#[derive(Debug, Default, PartialEq, Eq)]
enum Due {
    #[default]
    Nothing,
    /// Only these devices (Synapse or the headset reported news).
    Devices(BTreeSet<String>),
    All,
}

impl Due {
    fn add(&mut self, other: Due) {
        *self = match (std::mem::take(self), other) {
            (Due::All, _) | (_, Due::All) => Due::All,
            (Due::Nothing, d) | (d, Due::Nothing) => d,
            (Due::Devices(mut a), Due::Devices(b)) => {
                a.extend(b);
                Due::Devices(a)
            }
        };
    }
}

struct Loop {
    interval: Duration,
    next_poll: Instant,
    next_scan: Instant,
    due: Due,
}

impl Loop {
    /// Applies one command; `false` means the thread should stop.
    fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::RefreshNow => {
                // Re-enumerate too, so a freshly plugged dongle shows up.
                self.next_scan = Instant::now();
                self.due.add(Due::All);
            }
            Command::RefreshDevice(id) => self.due.add(Due::Devices(BTreeSet::from([id]))),
            Command::SetPollSecs(s) => {
                let new = poll_interval(s);
                if new != self.interval {
                    self.next_poll = self.next_poll - self.interval + new;
                    self.interval = new;
                }
            }
            Command::Quit => return false,
        }
        true
    }
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
    let synapse = {
        let tx = self_tx.clone();
        synapse::start(move |id| {
            log::debug!("synapse news for {id}");
            let _ = tx.send(Command::RefreshDevice(id.to_string()));
        })
    };
    let mut poller = Poller::new(self_tx, synapse);
    let mut present = enumerate(&api);
    let mut state = Loop {
        interval: poll_interval(poll_secs),
        next_poll: Instant::now(),
        next_scan: Instant::now() + HOTPLUG_INTERVAL,
        due: Due::All,
    };

    loop {
        let now = Instant::now();
        if now >= state.next_scan {
            state.next_scan = now + HOTPLUG_INTERVAL;
            if let Err(e) = api.refresh_devices() {
                log::warn!("refresh_devices: {e}");
            }
            let fresh = enumerate(&api);
            if pids(&fresh) != pids(&present) {
                log::info!("device set changed");
                state.due.add(Due::All);
            }
            present = fresh;
        }
        if now >= state.next_poll {
            state.due.add(Due::All);
        }
        let due = std::mem::take(&mut state.due);
        if due != Due::Nothing {
            let list = poller.poll(&api, &present, &due);
            if updates.send(list).is_err() {
                return;
            }
            ctx.request_repaint();
            if due == Due::All {
                state.next_poll = Instant::now() + state.interval;
            }
        }

        let wait = state
            .next_poll
            .min(state.next_scan)
            .saturating_duration_since(Instant::now());
        let mut next = match commands.recv_timeout(wait) {
            Ok(cmd) => Some(cmd),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Everything queued is handled before the next poll, so several
        // refresh requests make one poll.
        while let Some(cmd) = next.take() {
            if !state.handle(cmd) {
                return;
            }
            next = commands.try_recv().ok();
        }
    }
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
    /// Latest values from Synapse's logs (fallback only).
    synapse: synapse::Values,
}

impl Poller {
    fn new(refresh: Sender<Command>, synapse: synapse::Values) -> Self {
        Self {
            last: HashMap::new(),
            preferred: HashMap::new(),
            listeners: HashMap::new(),
            last_active: HashMap::new(),
            refresh,
            synapse,
        }
    }

    fn poll(&mut self, api: &HidApi, present: &[Present], due: &Due) -> Vec<DeviceStatus> {
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
            let wanted = match due {
                Due::All => true,
                Due::Devices(ids) => ids.contains(&id),
                Due::Nothing => false,
            };
            if !wanted && let Some(last) = self.last.get(&id) {
                out.push(last.clone());
                continue;
            }
            variants.sort_by_key(|p| !p.info.wired);
            let status = self.read_group(api, &id, &variants);
            self.last.insert(id, status.clone());
            out.push(status);
        }
        // Forget devices that were unplugged.
        self.last.retain(|id, _| out.iter().any(|s| &s.id == id));
        out
    }

    fn read_group(&mut self, api: &HidApi, id: &str, variants: &[&Present]) -> DeviceStatus {
        let mut result = Err(ReadError::Failed("nicht gelesen".into()));
        for p in variants {
            let mut pref = self.preferred.get(&p.info.pid).cloned();
            // The Barracuda's active query runs at most every
            // BARRACUDA_MIN_INTERVAL, whatever triggered this poll.
            let allow_active = match p.info.driver {
                Driver::Standard { .. } => true,
                Driver::Barracuda => self
                    .last_active
                    .get(&p.info.pid)
                    .is_none_or(|t| t.elapsed() >= BARRACUDA_MIN_INTERVAL),
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
        thread::sleep(REQUEST_GAP);

        let info = variants[0].info;
        let prev = self.last.get(id);
        let status = |link, battery| DeviceStatus {
            id: id.to_string(),
            name: info.name.into(),
            kind: info.kind,
            link,
            battery,
        };
        match result {
            Ok(b) => status(
                Link::Online,
                Some(BatteryReading {
                    source: ReadSource::Hid,
                    raw: b.raw,
                    raw_max: b.raw_max,
                    charging: b.charging,
                    read_at: Instant::now(),
                }),
            ),
            Err(ReadError::NotDue) => match prev {
                Some(prev) => prev.clone(),
                None => status(Link::Error("noch nicht gelesen".into()), None),
            },
            Err(ReadError::Asleep) => status(Link::Asleep, prev.and_then(|s| s.battery)),
            Err(ReadError::Failed(msg)) => {
                log::warn!("{id}: {msg}");
                let from_synapse = self
                    .synapse
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(id)
                    .copied();
                match from_synapse {
                    // HID failed: Synapse's own reading is the next best thing.
                    Some(v) => status(
                        Link::Online,
                        Some(BatteryReading {
                            source: ReadSource::Synapse,
                            raw: u16::from(v.percent),
                            raw_max: 100,
                            charging: v.charging,
                            read_at: v.at,
                        }),
                    ),
                    None => status(Link::Error(msg), prev.and_then(|s| s.battery)),
                }
            }
        }
    }

    fn update_listeners(&mut self, api: &HidApi, present: &[Present]) {
        self.listeners
            .retain(|pid, l| l.is_alive() && present.iter().any(|p| p.info.pid == *pid));
        for p in present {
            if p.info.driver != Driver::Barracuda || self.listeners.contains_key(&p.info.pid) {
                continue;
            }
            let tx = self.refresh.clone();
            let id = p.info.id();
            let on_event = move || {
                let _ = tx.send(Command::RefreshDevice(id.clone()));
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
    all.sort_by_key(|d| {
        (
            d.product_id(),
            d.interface_number(),
            d.usage_page(),
            d.usage(),
        )
    });
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
    println!();
    synapse::probe();
    Ok(())
}
