//! Table of supported Razer devices. Product ids and transaction ids are taken
//! from openrazer (`driver/razermouse_driver.{c,h}`).

use crate::model::DeviceKind;

pub const RAZER_VID: u16 = 0x1532;

/// How the battery of a device is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    /// The standard 90-byte feature report protocol (`protocol.rs`).
    Standard { transaction_id: u8 },
    /// Barracuda X 2.4 GHz dongle (`barracuda.rs`).
    Barracuda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfo {
    pub pid: u16,
    pub name: &'static str,
    pub kind: DeviceKind,
    pub driver: Driver,
    /// Product id used for the stable device id. Wired and wireless variants
    /// of the same mouse share it, so the overlay shows one row per mouse.
    pub id_pid: u16,
    /// Wired connection (the mouse is on its cable and charging).
    pub wired: bool,
}

impl DeviceInfo {
    /// Stable id, e.g. "1532:00ab".
    pub fn id(&self) -> String {
        device_id(self.id_pid)
    }
}

pub fn device_id(pid: u16) -> String {
    format!("{RAZER_VID:04x}:{pid:04x}")
}

const fn mouse(pid: u16, name: &'static str, tid: u8, id_pid: u16, wired: bool) -> DeviceInfo {
    DeviceInfo {
        pid,
        name,
        kind: DeviceKind::Mouse,
        driver: Driver::Standard {
            transaction_id: tid,
        },
        id_pid,
        wired,
    }
}

/// Devices with a battery. Anything not in here (e.g. the wired
/// Huntsman V3 Pro Mini, 02B0) is ignored.
pub const DEVICES: &[DeviceInfo] = &[
    mouse(0x00AB, "Basilisk V3 Pro", 0x1F, 0x00AB, false),
    mouse(0x00AA, "Basilisk V3 Pro", 0x1F, 0x00AB, true),
    mouse(0x00A6, "Viper V2 Pro", 0x1F, 0x00A6, false),
    mouse(0x00A5, "Viper V2 Pro", 0x1F, 0x00A6, true),
    mouse(0x00B7, "DeathAdder V3 Pro", 0x1F, 0x00B7, false),
    mouse(0x00B6, "DeathAdder V3 Pro", 0x1F, 0x00B7, true),
    mouse(0x007B, "Viper Ultimate", 0x3F, 0x007B, false),
    mouse(0x007A, "Viper Ultimate", 0x3F, 0x007B, true),
    mouse(0x007D, "DeathAdder V2 Pro", 0x3F, 0x007D, false),
    mouse(0x007C, "DeathAdder V2 Pro", 0x3F, 0x007D, true),
    mouse(0x0088, "Basilisk Ultimate", 0x3F, 0x0088, false),
    mouse(0x0086, "Basilisk Ultimate", 0x3F, 0x0088, true),
    DeviceInfo {
        pid: 0x0550,
        name: "Barracuda X",
        kind: DeviceKind::Headset,
        driver: Driver::Barracuda,
        id_pid: 0x0550,
        wired: false,
    },
];

pub fn lookup(pid: u16) -> Option<&'static DeviceInfo> {
    DEVICES.iter().find(|d| d.pid == pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_devices() {
        let b = lookup(0x00AB).unwrap();
        assert_eq!(b.driver, Driver::Standard { transaction_id: 0x1F });
        assert_eq!(b.id(), "1532:00ab");
        assert_eq!(lookup(0x00AA).unwrap().id(), "1532:00ab");
        assert_eq!(lookup(0x0550).unwrap().kind, DeviceKind::Headset);
        assert!(lookup(0x02B0).is_none(), "Huntsman has no battery");
    }

    #[test]
    fn pids_are_unique() {
        for (i, a) in DEVICES.iter().enumerate() {
            for b in &DEVICES[i + 1..] {
                assert_ne!(a.pid, b.pid);
            }
        }
    }
}
