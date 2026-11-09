//! The Bluetooth LE adverts sent by LAP CCT LED panel remotes (eWeLink-style, "50W-2.4G-CCT" driver).
//!
//! A press is about 0.5 s of identical legacy `ADV_NONCONN_IND` adverts on channel 37 from the random
//! address `11:22:33:44:55:66`. The 31 bytes of advert data are:
//!
//! | Bytes | Meaning |
//! | --- | --- |
//! | 0-4   | `02 01 02 1b 05`: flags, then one 27-byte AD type 0x05 field |
//! | 5-14  | `89 8a ee 1b c8 78 f6 4a 00 00`, the same on every remote seen |
//! | 15    | sequence number, +1 per press |
//! | 16-17 | remote ID |
//! | 18-19 | `08 05` |
//! | 20    | group (0, 1 or 2); the light accepts any |
//! | 21    | button code |
//! | 22-27 | zero |
//! | 28    | random seed |
//! | 29-30 | CRC-16 of bytes 13-28, little-endian |
//!
//! Byte strings here are in on-air order, which is also the order HCI uses.

#![no_std]

use core::fmt;
use core::str::FromStr;

/// Length of a remote's advert data.
pub const ADVERT_LEN: usize = 31;

const PREFIX: [u8; 5] = [0x02, 0x01, 0x02, 0x1b, 0x05];
const EWELINK_HEADER: [u8; 6] = [0xee, 0x1b, 0xc8, 0x78, 0xf6, 0x4a];
const SEQ: usize = 15;
const BUTTON: usize = 21;
const SEED: usize = 28;
const CRC_START: usize = 13;
const CRC_AT: usize = 29;

/// Reflected CRC-16 with polynomial 0x1021, init 0xAAAA and no final xor.
#[must_use]
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xAAAA;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// A button on the remote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Power,
    Night,
    Brighter,
    Dimmer,
    Cooler,
    Warmer,
}

impl Button {
    /// Every button, in the order the remote lays them out.
    pub const ALL: [Button; 6] = [
        Button::Power,
        Button::Night,
        Button::Brighter,
        Button::Dimmer,
        Button::Cooler,
        Button::Warmer,
    ];

    /// The code sent in byte 21.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Button::Power => 0x01,
            Button::Night => 0x02,
            Button::Brighter => 0x04,
            Button::Dimmer => 0x05,
            Button::Cooler => 0x06,
            Button::Warmer => 0x07,
        }
    }

    #[must_use]
    pub fn from_code(code: u8) -> Option<Button> {
        Button::ALL.into_iter().find(|b| b.code() == code)
    }

    /// Lower-case name, as typed on the command line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Button::Power => "power",
            Button::Night => "night",
            Button::Brighter => "brighter",
            Button::Dimmer => "dimmer",
            Button::Cooler => "cooler",
            Button::Warmer => "warmer",
        }
    }

    /// Capitalised name, for menus.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Button::Power => "Power",
            Button::Night => "Night",
            Button::Brighter => "Brighter",
            Button::Dimmer => "Dimmer",
            Button::Cooler => "Cooler",
            Button::Warmer => "Warmer",
        }
    }
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A name that isn't a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownButton;

impl fmt::Display for UnknownButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown button")
    }
}

impl core::error::Error for UnknownButton {}

impl FromStr for Button {
    type Err = UnknownButton;

    /// Accepts the button names and a few obvious alternatives (`on`, `up`, `+`, `warm`, ...).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = [0u8; 8];
        let lower = buf.get_mut(..s.len()).ok_or(UnknownButton)?;
        lower.copy_from_slice(s.as_bytes());
        lower.make_ascii_lowercase();
        Ok(
            match core::str::from_utf8(lower).map_err(|_| UnknownButton)? {
                "power" | "toggle" | "on" | "off" => Button::Power,
                "night" | "moon" => Button::Night,
                "brighter" | "up" | "plus" | "+" => Button::Brighter,
                "dimmer" | "down" | "minus" | "-" => Button::Dimmer,
                "cooler" | "cool" => Button::Cooler,
                "warmer" | "warm" => Button::Warmer,
                _ => return Err(UnknownButton),
            },
        )
    }
}

/// Why some bytes aren't a remote advert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Not valid hex, or the wrong number of digits.
    Hex,
    /// Wrong length.
    Length,
    /// Doesn't have the remote's fixed bytes.
    NotRemote,
    /// Has the remote's layout, but the CRC doesn't match.
    Crc,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::Hex => "not the right number of hex digits",
            ParseError::Length => "wrong length for a remote advert",
            ParseError::NotRemote => "not a light remote advert",
            ParseError::Crc => "light remote advert with a bad CRC",
        })
    }
}

impl core::error::Error for ParseError {}

fn parse_hex<const N: usize>(s: &str) -> Result<[u8; N], ParseError> {
    let s = s.as_bytes();
    if s.len() != N * 2 {
        return Err(ParseError::Hex);
    }
    let digit = |c: u8| (c as char).to_digit(16).ok_or(ParseError::Hex);
    let mut out = [0; N];
    for (byte, &[hi, lo]) in out.iter_mut().zip(s.as_chunks::<2>().0) {
        // Two hex digits always fit in a byte
        #[allow(clippy::cast_possible_truncation)]
        let value = (digit(hi)? << 4 | digit(lo)?) as u8;
        *byte = value;
    }
    Ok(out)
}

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    bytes.iter().try_for_each(|b| write!(f, "{b:02x}"))
}

/// A Bluetooth device address, in on-air (HCI) byte order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address(pub [u8; 6]);

impl Address {
    /// The address every remote seen so far sends from; the light ignores others.
    pub const REMOTE: Address = Address([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
}

impl FromStr for Address {
    type Err = ParseError;

    /// 12 hex digits, in on-air order.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_hex(s).map(Address)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

/// The two bytes a remote prints its identity with (bytes 16-17), e.g. `1a2b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RemoteId(pub [u8; 2]);

impl fmt::Display for RemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

/// One remote press's advert data. Only valid remote adverts (fixed bytes and CRC) can be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Advert([u8; ADVERT_LEN]);

impl Advert {
    /// Checks that `data` is a remote advert with a valid CRC.
    pub fn parse(data: &[u8]) -> Result<Advert, ParseError> {
        let data: [u8; ADVERT_LEN] = data.try_into().map_err(|_| ParseError::Length)?;
        if data[..5] != PREFIX || data[7..13] != EWELINK_HEADER {
            return Err(ParseError::NotRemote);
        }
        let advert = Advert(data);
        if advert.stored_crc() != crc16(&data[CRC_START..CRC_AT]) {
            return Err(ParseError::Crc);
        }
        Ok(advert)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; ADVERT_LEN] {
        &self.0
    }

    #[must_use]
    pub fn seq(&self) -> u8 {
        self.0[SEQ]
    }

    #[must_use]
    pub fn button_code(&self) -> u8 {
        self.0[BUTTON]
    }

    /// The button pressed, if it's one we know (group buttons aren't).
    #[must_use]
    pub fn button(&self) -> Option<Button> {
        Button::from_code(self.button_code())
    }

    #[must_use]
    pub fn remote_id(&self) -> RemoteId {
        RemoteId([self.0[16], self.0[17]])
    }

    /// Whether both adverts came from the same remote: everything before the sequence number,
    /// and bytes 16-19, match.
    #[must_use]
    pub fn same_remote(&self, other: &Advert) -> bool {
        self.0[5..SEQ] == other.0[5..SEQ] && self.0[16..20] == other.0[16..20]
    }

    /// A new press from the same remote: this advert with a new sequence number, button and seed.
    #[must_use]
    pub fn press(&self, seq: u8, button: Button, seed: u8) -> Advert {
        let mut data = self.0;
        data[SEQ] = seq;
        data[BUTTON] = button.code();
        data[SEED] = seed;
        let crc = crc16(&data[CRC_START..CRC_AT]);
        data[CRC_AT..].copy_from_slice(&crc.to_le_bytes());
        Advert(data)
    }

    fn stored_crc(&self) -> u16 {
        u16::from_le_bytes([self.0[CRC_AT], self.0[CRC_AT + 1]])
    }
}

impl FromStr for Advert {
    type Err = ParseError;

    /// 62 hex digits.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Advert::parse(&parse_hex::<ADVERT_LEN>(s)?)
    }
}

impl fmt::Display for Advert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}
