//! The Razer 90-byte HID feature report used by Razer mice (and most other
//! Razer peripherals), as documented by openrazer (`razercommon.h`).
//!
//! Layout (report id 0 is *not* part of these 90 bytes):
//!
//! | offset | field |
//! |---|---|
//! | 0 | status (0x00 in requests) |
//! | 1 | transaction id |
//! | 2..4 | remaining packets (big endian) |
//! | 4 | protocol type (always 0x00) |
//! | 5 | data size (number of used `args`) |
//! | 6 | command class |
//! | 7 | command id |
//! | 8..88 | args |
//! | 88 | crc: XOR of bytes 2..88 |
//! | 89 | reserved |

use std::fmt;

pub const REPORT_LEN: usize = 90;
pub const ARGS_LEN: usize = 80;

/// Command class / id pairs we use. All of them are read-only "get" commands.
pub const CLASS_POWER: u8 = 0x07;
pub const CMD_GET_BATTERY: u8 = 0x80;
pub const CMD_GET_CHARGING: u8 = 0x84;

/// The `status` byte of a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    New,
    Busy,
    Ok,
    Failure,
    /// The receiver could not reach the device (asleep, off, out of range).
    Timeout,
    NotSupported,
    Unknown(u8),
}

impl From<u8> for Status {
    fn from(b: u8) -> Self {
        match b {
            0x00 => Status::New,
            0x01 => Status::Busy,
            0x02 => Status::Ok,
            0x03 => Status::Failure,
            0x04 => Status::Timeout,
            0x05 => Status::NotSupported,
            other => Status::Unknown(other),
        }
    }
}

/// Why a response could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Device is still working on the request; read again shortly.
    Busy,
    /// Device asleep / unreachable.
    Timeout,
    Failure,
    NotSupported,
    /// The response belongs to another request (e.g. Razer Synapse talking
    /// to the device at the same time) or is garbage.
    Mismatch(&'static str),
    BadCrc,
    UnknownStatus(u8),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Busy => write!(f, "Gerät beschäftigt"),
            ParseError::Timeout => write!(f, "Gerät antwortet nicht (schläft?)"),
            ParseError::Failure => write!(f, "Gerät meldet Fehler"),
            ParseError::NotSupported => write!(f, "Befehl nicht unterstützt"),
            ParseError::Mismatch(what) => write!(f, "fremde Antwort ({what})"),
            ParseError::BadCrc => write!(f, "Prüfsumme falsch"),
            ParseError::UnknownStatus(s) => write!(f, "unbekannter Status 0x{s:02x}"),
        }
    }
}

/// XOR of bytes 2..88, as the firmware computes it.
pub fn crc(report: &[u8; REPORT_LEN]) -> u8 {
    report[2..88].iter().fold(0, |acc, b| acc ^ b)
}

/// Builds a request report.
pub fn build_request(
    transaction_id: u8,
    class: u8,
    id: u8,
    data_size: u8,
    args: &[u8],
) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[1] = transaction_id;
    r[5] = data_size;
    r[6] = class;
    r[7] = id;
    let n = args.len().min(ARGS_LEN);
    r[8..8 + n].copy_from_slice(&args[..n]);
    r[88] = crc(&r);
    r
}

/// The parts of an OK response the caller needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub data_size: u8,
    pub args: [u8; ARGS_LEN],
}

/// Checks `resp` against the request it should answer.
pub fn parse_response(
    req: &[u8; REPORT_LEN],
    resp: &[u8; REPORT_LEN],
) -> Result<Response, ParseError> {
    match Status::from(resp[0]) {
        Status::Ok => {}
        Status::Busy => return Err(ParseError::Busy),
        Status::Timeout => return Err(ParseError::Timeout),
        Status::Failure => return Err(ParseError::Failure),
        Status::NotSupported => return Err(ParseError::NotSupported),
        // "New" means the device has not processed anything yet (or we read
        // back our own request); treat like busy.
        Status::New => return Err(ParseError::Busy),
        Status::Unknown(s) => return Err(ParseError::UnknownStatus(s)),
    }
    if resp[6] != req[6] || resp[7] != req[7] {
        return Err(ParseError::Mismatch("command"));
    }
    if resp[1] != req[1] {
        return Err(ParseError::Mismatch("transaction id"));
    }
    if crc(resp) != resp[88] {
        return Err(ParseError::BadCrc);
    }
    let mut args = [0u8; ARGS_LEN];
    args.copy_from_slice(&resp[8..88]);
    Ok(Response {
        data_size: resp[5],
        args,
    })
}

/// Request for the battery level; the answer is in `args[1]`, 0..=255.
pub fn battery_request(transaction_id: u8) -> [u8; REPORT_LEN] {
    build_request(transaction_id, CLASS_POWER, CMD_GET_BATTERY, 0x02, &[])
}

/// Request for the charging state; the answer is in `args[1]`, 0 or 1.
pub fn charging_request(transaction_id: u8) -> [u8; REPORT_LEN] {
    build_request(transaction_id, CLASS_POWER, CMD_GET_CHARGING, 0x02, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_response(req: &[u8; REPORT_LEN], args: &[u8]) -> [u8; REPORT_LEN] {
        let mut r = *req;
        r[0] = 0x02;
        r[8..8 + args.len()].copy_from_slice(args);
        r[88] = crc(&r);
        r
    }

    #[test]
    fn battery_request_layout() {
        let r = battery_request(0x1f);
        assert_eq!(r[0], 0x00);
        assert_eq!(r[1], 0x1f);
        assert_eq!(&r[2..5], &[0, 0, 0]);
        assert_eq!(r[5], 0x02);
        assert_eq!(r[6], 0x07);
        assert_eq!(r[7], 0x80);
        assert!(r[8..88].iter().all(|&b| b == 0));
        // 0x02 ^ 0x07 ^ 0x80
        assert_eq!(r[88], 0x85);
        assert_eq!(r[89], 0x00);
    }

    #[test]
    fn crc_ignores_status_and_transaction_id() {
        let a = battery_request(0x1f);
        let b = battery_request(0x3f);
        assert_eq!(a[88], b[88]);
    }

    #[test]
    fn crc_covers_args() {
        let r = build_request(0x1f, 0x0f, 0x02, 3, &[0x01, 0x05, 0xff]);
        assert_eq!(r[88], 3 ^ 0x0f ^ 0x02 ^ 0x01 ^ 0x05 ^ 0xff);
    }

    #[test]
    fn parses_battery_answer() {
        let req = battery_request(0x1f);
        let resp = ok_response(&req, &[0x00, 0xb4]);
        let parsed = parse_response(&req, &resp).unwrap();
        assert_eq!(parsed.args[1], 0xb4);
    }

    #[test]
    fn status_codes() {
        let req = charging_request(0x1f);
        let mut resp = ok_response(&req, &[0, 1]);
        for (status, err) in [
            (0x01, ParseError::Busy),
            (0x03, ParseError::Failure),
            (0x04, ParseError::Timeout),
            (0x05, ParseError::NotSupported),
            (0x09, ParseError::UnknownStatus(0x09)),
        ] {
            resp[0] = status;
            assert_eq!(parse_response(&req, &resp), Err(err));
        }
    }

    #[test]
    fn rejects_foreign_answers() {
        let req = battery_request(0x1f);
        let other = ok_response(&charging_request(0x1f), &[0, 1]);
        assert_eq!(
            parse_response(&req, &other),
            Err(ParseError::Mismatch("command"))
        );
        let other_tid = ok_response(&battery_request(0x3f), &[0, 10]);
        assert_eq!(
            parse_response(&req, &other_tid),
            Err(ParseError::Mismatch("transaction id"))
        );
        let mut bad = ok_response(&req, &[0, 10]);
        bad[88] ^= 0xff;
        assert_eq!(parse_response(&req, &bad), Err(ParseError::BadCrc));
    }
}
