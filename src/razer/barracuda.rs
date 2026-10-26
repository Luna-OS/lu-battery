//! Razer Barracuda X (2022) over its 2.4 GHz receiver, 1532:0550.
//!
//! Razer does not document this protocol. What we know comes from:
//! - github.com/nightdevx/barracuda-x-battery (`barracuda.py`, tested on a 0550)
//! - github.com/mehdibouchami/razer-barracuda-x-battery (`src/Protocol.cs`, YS-Tech framing)
//! - github.com/leag/razer-barracuda (`docs/PROTOCOL.md`, 0552 Macronix receiver)
//!
//! The receiver (YS-Tech bridge) exposes HID only on interface MI_03:
//! COL01 = consumer control (media keys), COL02 = vendor page 0xFF00, report id 0x02,
//! 64-byte reports. Through COL02 the headset's Airoha chip speaks frames
//! `50 41 …` (to the headset) and `50 49 …` (from the headset):
//!
//! ```text
//! 50 49 <group> <seq/…> … [8..10] = payload length (LE), [10] = cmd, [11] = op, [12] = len, [13..] = value
//! ```
//!
//! An input report on COL02 is `02 <n> <n stream bytes…>`; one report can carry
//! several frames. Group 0x08 ("customer data"): cmd 0x21 = battery percent
//! (0..=100), cmd 0x2A = cable plugged (0/1); op 0x01 = reply to a GET,
//! op 0x02 = unsolicited event.
//!
//! The headset sends events on its own only when the cable is plugged or
//! pulled. Actively asking for the battery needs the receiver switched into
//! "remote mode" (`E1 01`) first and back (`E1 00`) afterwards. That toggle
//! only routes commands to the headset and changes no headset setting.
//!
//! What lu-battery sends, and nothing else (agreed with luna, tested on the
//! user's 0550 on 2026-10-26: 47 %, audio and mic kept working):
//!
//! 1. `50 41 0E <seq> 02 E1 01`: remote mode on, reply `[10]=0E [11]=80|seq [12]=0`
//! 2. `50 41 08 <seq> 03 21 00 00`: GET battery, reply `[10]=21 [11]=01 [12]=1 [13]=percent`
//! 3. `50 41 08 <seq> 03 2A 00 00`: GET cable, reply value 0/1
//! 4. `50 41 0E <seq> 02 E1 00`: remote mode off, always (drop guard)
//!
//! Each command goes out as output report `02 80 <len> <crc lo> <crc hi> <cmd…>`
//! (64 bytes, CRC-16/XMODEM over the report with the CRC bytes zero); the
//! bridge acks via GetInputReport(0x02) with `'O'` (taken) or `'F'` (busy).
//! Replies arrive as input reports. 500 ms timeout per reply; the poller asks
//! at most every 5 s (`BARRACUDA_MIN_INTERVAL`, agreed with luna). In between, a
//! passive listener picks up the unsolicited events (cable plugged or pulled,
//! battery while charging); repeated identical events are ignored.
//! The value is the device's percent: raw/raw_max = pct/100.

use super::{Interface, RawBattery, ReadError};
use crate::model::Charging;
use hidapi::{HidApi, HidDevice};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const VENDOR_USAGE_PAGE: u16 = 0xFF00;
pub const REPORT_ID: u8 = 0x02;
pub const REPORT_LEN: usize = 64;

pub const GROUP_CUSTOMER: u8 = 0x08;
pub const CMD_BATTERY: u8 = 0x21;
pub const CMD_CABLE: u8 = 0x2A;
pub const OP_REPLY: u8 = 0x01;
pub const OP_EVENT: u8 = 0x02;

/// CRC-16/XMODEM (poly 0x1021, init 0), used by the YS-Tech bridge.
pub fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The stream bytes an input report carries (`02 <n> bytes…`), empty for
/// anything else (e.g. media-key reports).
pub fn chunk(report: &[u8]) -> &[u8] {
    if report.len() >= 2 && report[0] == REPORT_ID {
        let n = usize::from(report[1]).min(report.len() - 2);
        &report[2..2 + n]
    } else {
        &[]
    }
}

/// Reassembles `50 49` frames from report chunks.
#[derive(Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            let Some(start) = self.buf.windows(2).position(|w| w == [0x50, 0x49]) else {
                // Keep a header cut in half.
                let keep = self.buf.last() == Some(&0x50);
                self.buf.clear();
                if keep {
                    self.buf.push(0x50);
                }
                return out;
            };
            self.buf.drain(..start);
            if self.buf.len() < 10 {
                return out;
            }
            let end = 10 + (usize::from(self.buf[8]) | usize::from(self.buf[9]) << 8);
            if self.buf.len() < end {
                return out;
            }
            out.push(self.buf.drain(..end).collect());
        }
    }
}

/// A customer-data value from the headset: (cmd, op, value bytes).
pub fn customer_value(frame: &[u8]) -> Option<(u8, u8, &[u8])> {
    if frame.len() < 13 || frame[2] != GROUP_CUSTOMER {
        return None;
    }
    let len = usize::from(frame[12]);
    let value = frame.get(13..13 + len)?;
    Some((frame[10], frame[11], value))
}

/// Battery percent from a frame, if it is a valid battery value.
pub fn battery_percent(frame: &[u8]) -> Option<u8> {
    match customer_value(frame) {
        Some((CMD_BATTERY, OP_REPLY | OP_EVENT, [pct, ..])) if *pct <= 100 => Some(*pct),
        _ => None,
    }
}

fn vendor_interface(ifaces: &[Interface]) -> Option<&Interface> {
    ifaces.iter().find(|i| i.usage_page == VENDOR_USAGE_PAGE)
}

// ---------------------------------------------------------------------------
// Active query: E1 01 → GET 0x21 / GET 0x2A → E1 00. Nothing else is ever sent.

/// How long to wait for a reply frame.
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);

/// An open vendor collection of the receiver.
pub struct Dongle {
    dev: HidDevice,
    frames: Frames,
    seq: u8,
}

impl Dongle {
    pub fn open(api: &HidApi, iface: &Interface) -> Result<Self, ReadError> {
        let dev = api
            .open_path(&iface.path)
            .map_err(|e| ReadError::Failed(format!("öffnen: {e}")))?;
        Ok(Self {
            dev,
            frames: Frames::default(),
            seq: 0,
        })
    }

    /// Wraps one Airoha command into a YS-Tech output report:
    /// `02 80 <len> <crc lo> <crc hi> <payload…>`, padded to 64 bytes,
    /// CRC-16/XMODEM over the whole report with the CRC bytes zero.
    pub fn output_report(payload: &[u8]) -> [u8; REPORT_LEN] {
        let mut r = [0u8; REPORT_LEN];
        let n = payload.len().min(REPORT_LEN - 5);
        r[0] = REPORT_ID;
        r[1] = 0x80;
        r[2] = n as u8;
        r[5..5 + n].copy_from_slice(&payload[..n]);
        let [lo, hi] = crc16_xmodem(&r).to_le_bytes();
        r[3] = lo;
        r[4] = hi;
        r
    }

    /// Sends one command; the bridge acknowledges through GetInputReport
    /// with 'O' (taken) or 'F' (busy, retry).
    fn send_command(&mut self, payload: &[u8]) -> Result<(), ReadError> {
        let report = Self::output_report(payload);
        for attempt in 0..5 {
            if attempt > 0 {
                thread::sleep(Duration::from_millis(50));
            }
            self.dev
                .write(&report)
                .map_err(|e| ReadError::Failed(format!("senden: {e}")))?;
            let mut status = [0u8; REPORT_LEN];
            status[0] = REPORT_ID;
            let n = self
                .dev
                .get_input_report(&mut status)
                .map_err(|e| ReadError::Failed(format!("Bestätigung: {e}")))?;
            match (n >= 2).then_some(status[1]) {
                Some(ACK_TAKEN) => return Ok(()),
                Some(ACK_BUSY) => continue,
                other => {
                    return Err(ReadError::Failed(format!(
                        "Empfänger lehnt ab ({other:02x?})"
                    )));
                }
            }
        }
        Err(ReadError::Failed("Empfänger beschäftigt".into()))
    }

    /// Sends `50 41 <group> <seq> <body…>` and waits for the first reply
    /// frame that `matches(frame, seq)`.
    fn request(
        &mut self,
        group: u8,
        body: &[u8],
        matches: impl Fn(&[u8], u8) -> bool,
    ) -> Result<Option<Vec<u8>>, ReadError> {
        self.seq = self.seq % 127 + 1;
        let seq = self.seq;
        let mut payload = vec![0x50, 0x41, group, seq];
        payload.extend_from_slice(body);
        self.send_command(&payload)?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let mut buf = [0u8; REPORT_LEN];
            let n = self
                .dev
                .read_timeout(&mut buf, left.as_millis().max(1) as i32)
                .map_err(|e| ReadError::Failed(format!("lesen: {e}")))?;
            for f in self.frames.feed(chunk(&buf[..n])) {
                log::trace!("barracuda frame {f:02x?}");
                if f.len() >= 12 && matches(&f, seq) {
                    return Ok(Some(f));
                }
            }
        }
    }

    /// `E1 01` / `E1 00`. `Ok(true)` when the receiver confirmed.
    fn remote_mode(&mut self, on: bool) -> Result<bool, ReadError> {
        let reply = self.request(0x0E, &[0x02, 0xE1, u8::from(on)], |f, seq| {
            f[10] == 0x0E && f[11] == 0x80 | seq
        })?;
        Ok(reply.is_some_and(|f| f.len() > 12 && f[12] == 0))
    }

    /// Customer-data GET (`03 <cmd> 00 00`); the value bytes of the reply.
    fn get(&mut self, cmd: u8) -> Result<Option<Vec<u8>>, ReadError> {
        let reply = self.request(GROUP_CUSTOMER, &[0x03, cmd, 0x00, 0x00], |f, _| {
            f[2] == GROUP_CUSTOMER && f[10] == cmd && f[11] == OP_REPLY
        })?;
        Ok(reply.and_then(|f| customer_value(&f).map(|(_, _, v)| v.to_vec())))
    }

    /// The whole read: remote mode on, battery + cable, remote mode off.
    /// Returns (percent, cable plugged).
    pub fn read_battery(&mut self) -> Result<(u8, Option<bool>), ReadError> {
        let session = RemoteSession { dongle: self };
        if !session.dongle.remote_mode(true)? {
            // The receiver did not reach the headset: off, out of range or
            // in Bluetooth mode.
            return Err(ReadError::Asleep);
        }
        let pct = session.dongle.get(CMD_BATTERY)?;
        let cable = session.dongle.get(CMD_CABLE)?;
        drop(session);
        let pct = match pct.as_deref() {
            Some([p, ..]) if *p <= 100 => *p,
            Some(v) => {
                return Err(ReadError::Failed(format!("ungültiger Akkuwert {v:02x?}")));
            }
            None => return Err(ReadError::Asleep),
        };
        let cable = match cable.as_deref() {
            Some([0, ..]) => Some(false),
            Some([1, ..]) => Some(true),
            _ => None,
        };
        Ok((pct, cable))
    }
}

const ACK_TAKEN: u8 = b'O';
const ACK_BUSY: u8 = b'F';

/// Switches the receiver back with `E1 00` when dropped, also on errors and
/// panics (release builds unwind, see Cargo.toml). Created before `E1 01` is
/// sent, so a lost reply still gets reset.
struct RemoteSession<'a> {
    dongle: &'a mut Dongle,
}

impl Drop for RemoteSession<'_> {
    fn drop(&mut self) {
        match self.dongle.remote_mode(false) {
            Ok(true) => {}
            Ok(false) => log::warn!("barracuda: E1 00 not confirmed"),
            Err(e) => log::warn!("barracuda: E1 00 failed: {e:?}"),
        }
    }
}

fn to_raw(pct: u8, cable: Option<bool>) -> RawBattery {
    // The headset only says whether the cable is plugged in; on the cable it
    // charges, at 100 % it is full.
    let charging = match cable {
        Some(true) if pct >= 100 => Charging::Full,
        Some(true) => Charging::Charging,
        Some(false) => Charging::Discharging,
        None => Charging::Unknown,
    };
    RawBattery {
        raw: u16::from(pct),
        raw_max: 100,
        charging,
    }
}

// ---------------------------------------------------------------------------
// Passive listening for unsolicited events (cable plugged / pulled, followed
// by the battery value). Only reads; Windows gives every open handle its own
// copy of each input report, so this runs next to the active queries.

#[derive(Debug, Default, Clone, Copy)]
struct Passive {
    /// Battery percent from an event that the poller has not used yet.
    fresh_pct: Option<u8>,
    /// Last battery percent seen in an event.
    last_pct: Option<u8>,
    cable: Option<bool>,
}

pub struct Listener {
    state: Arc<Mutex<Passive>>,
    alive: Arc<AtomicBool>,
}

impl Listener {
    /// Starts a thread reading the vendor collection. `on_event` is called
    /// after each battery or cable event (e.g. to trigger a refresh).
    pub fn start(
        api: &HidApi,
        ifaces: &[Interface],
        on_event: impl Fn() + Send + 'static,
    ) -> Option<Self> {
        let iface = vendor_interface(ifaces)?;
        let dev = api.open_path(&iface.path).ok()?;
        let state = Arc::new(Mutex::new(Passive::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let (st, al) = (state.clone(), alive.clone());
        thread::Builder::new()
            .name("barracuda-listen".into())
            .spawn(move || {
                let mut frames = Frames::default();
                let mut buf = [0u8; REPORT_LEN];
                while al.load(Ordering::Relaxed) {
                    let Ok(n) = dev.read_timeout(&mut buf, 1000) else {
                        break; // receiver unplugged
                    };
                    for f in frames.feed(chunk(&buf[..n])) {
                        let Some((cmd, OP_EVENT, value)) = customer_value(&f) else {
                            continue;
                        };
                        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                        // While charging the headset repeats the cable event
                        // every few seconds; only real changes count.
                        let changed = match (cmd, value) {
                            (CMD_BATTERY, [p, ..]) if *p <= 100 => {
                                s.fresh_pct = Some(*p);
                                s.last_pct.replace(*p) != Some(*p)
                            }
                            (CMD_CABLE, [c, ..]) => s.cable.replace(*c != 0) != Some(*c != 0),
                            _ => false,
                        };
                        drop(s);
                        log::debug!("barracuda event {cmd:02x} {value:02x?}");
                        if changed {
                            on_event();
                        }
                    }
                }
                al.store(false, Ordering::Relaxed);
            })
            .ok()?;
        Some(Self { state, alive })
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    fn take(&self) -> Passive {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let out = *s;
        s.fresh_pct = None;
        out
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

/// One poll: the active query, or a fresh event value if the query failed.
pub fn read(
    api: &HidApi,
    ifaces: &[Interface],
    listener: Option<&Listener>,
    allow_active: bool,
) -> Result<RawBattery, ReadError> {
    let passive = listener.map(Listener::take).unwrap_or_default();
    if !allow_active {
        return match passive.fresh_pct {
            Some(pct) => Ok(to_raw(pct, passive.cable)),
            None => Err(ReadError::NotDue),
        };
    }
    let Some(iface) = vendor_interface(ifaces) else {
        return Err(ReadError::Failed("Akku nicht lesbar".into()));
    };
    let active = Dongle::open(api, iface).and_then(|mut d| d.read_battery());
    match (active, passive.fresh_pct) {
        (Ok((pct, cable)), _) => Ok(to_raw(pct, cable)),
        (Err(_), Some(pct)) => Ok(to_raw(pct, passive.cable)),
        (Err(ReadError::Failed(msg)), None) => {
            log::warn!("barracuda: {msg}");
            Err(ReadError::Failed("Akku nicht lesbar".into()))
        }
        (Err(e), None) => Err(e),
    }
}

/// `--probe`: report descriptor and a few seconds of passive listening.
pub fn probe(api: &HidApi, ifaces: &[Interface]) {
    for i in ifaces {
        println!(
            "  MI_{:02} {:04x}/{:04x}",
            i.interface_number, i.usage_page, i.usage
        );
        let dev = match api.open_path(&i.path) {
            Ok(d) => d,
            Err(e) => {
                println!("    öffnen fehlgeschlagen: {e}");
                continue;
            }
        };
        let mut desc = [0u8; 4096];
        match dev.get_report_descriptor(&mut desc) {
            Ok(n) => println!("    report descriptor: {:02x?}", &desc[..n]),
            Err(e) => println!("    report descriptor: {e}"),
        }
        if i.usage_page != VENDOR_USAGE_PAGE {
            continue;
        }
        drop(dev);
        println!("    aktive Abfrage (E1 01 → GET 21/2A → E1 00) …");
        match Dongle::open(api, i).and_then(|mut d| d.read_battery()) {
            Ok((pct, cable)) => println!(
                "    → Akku {pct} % (raw {pct}/100), Kabel {cable:?} → {:?}",
                to_raw(pct, cable).charging
            ),
            Err(e) => println!("    → {e:?}"),
        }
        let Ok(dev) = api.open_path(&i.path) else {
            continue;
        };
        println!("    lausche 3 s passiv …");
        let mut frames = Frames::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            let mut buf = [0u8; REPORT_LEN];
            match dev.read_timeout(&mut buf, 250) {
                Ok(0) => {}
                Ok(n) => {
                    println!("    in: {:02x?}", &buf[..n]);
                    for f in frames.feed(chunk(&buf[..n])) {
                        println!("    frame: {f:02x?} battery={:?}", battery_percent(&f));
                    }
                }
                Err(e) => {
                    println!("    read: {e}");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    // Captures from a 0550 receiver, quoted in nightdevx/barracuda-x-battery.
    #[test]
    fn ack_and_battery_reply_in_one_report() {
        let ack = hex("50 49 01 c0 59 31 29 01 03 00 0e 84 00");
        let reply = hex("50 49 08 fa b8 ce 17 01 04 00 21 01 01 26");
        let mut report = vec![REPORT_ID, (ack.len() + reply.len()) as u8];
        report.extend(&ack);
        report.extend(&reply);
        let frames = Frames::default().feed(chunk(&report));
        assert_eq!(frames, vec![ack.clone(), reply.clone()]);
        assert_eq!(battery_percent(&ack), None);
        assert_eq!(battery_percent(&reply), Some(0x26));
    }

    #[test]
    fn cable_event() {
        let report = hex(
            "02 1c 50 49 08 fc 04 2d 67 00 04 00 21 02 01 35 50 49 08 fd 04 2d 67 00 04 00 2a 02 01 01",
        );
        let frames = Frames::default().feed(chunk(&report));
        assert_eq!(frames.len(), 2);
        assert_eq!(battery_percent(&frames[0]), Some(0x35));
        assert_eq!(
            customer_value(&frames[1]),
            Some((CMD_CABLE, OP_EVENT, &[1u8][..]))
        );
    }

    #[test]
    fn frame_split_across_reports() {
        let link = hex("50 49 08 f7 0d 32 69 00 04 00 21 02 01 40");
        let mut frames = Frames::default();
        assert!(frames.feed(&link[..5]).is_empty());
        assert_eq!(frames.feed(&link[5..]), vec![link.clone()]);
    }

    #[test]
    fn output_report_layout() {
        // GET battery with seq 3.
        let r = Dongle::output_report(&[0x50, 0x41, 0x08, 0x03, 0x03, 0x21, 0x00, 0x00]);
        assert_eq!(&r[..3], &[0x02, 0x80, 8]);
        assert_eq!(&r[5..13], &[0x50, 0x41, 0x08, 0x03, 0x03, 0x21, 0x00, 0x00]);
        assert!(r[13..].iter().all(|&b| b == 0));
        let mut zeroed = r;
        zeroed[3] = 0;
        zeroed[4] = 0;
        assert_eq!(u16::from_le_bytes([r[3], r[4]]), crc16_xmodem(&zeroed));
    }

    #[test]
    fn charging_from_cable() {
        assert_eq!(to_raw(55, Some(true)).charging, Charging::Charging);
        assert_eq!(to_raw(100, Some(true)).charging, Charging::Full);
        assert_eq!(to_raw(55, Some(false)).charging, Charging::Discharging);
        assert_eq!(to_raw(55, None).raw_max, 100);
    }

    #[test]
    fn rejects_out_of_range_percent() {
        let f = hex("50 49 08 fa b8 ce 17 01 04 00 21 01 01 ff");
        assert_eq!(battery_percent(&f), None);
    }
}
