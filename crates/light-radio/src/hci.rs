//! Just enough of a raw HCI socket to send LE commands and read LE meta events.

use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

const AF_BLUETOOTH: u16 = 31;
const BTPROTO_HCI: libc::c_int = 1;
const SOL_HCI: libc::c_int = 0;
const HCI_FILTER: libc::c_int = 2;
const HCI_CHANNEL_RAW: u16 = 0;

const HCI_COMMAND_PKT: u8 = 0x01;
const HCI_EVENT_PKT: u8 = 0x04;
const OGF_LE: u16 = 0x08;

/// Command Complete event.
pub const EVT_CMD_COMPLETE: u8 = 0x0E;
/// LE Meta event (advertising reports and others).
pub const EVT_LE_META: u8 = 0x3E;

/// Size of a socket option or address struct, as the kernel wants it.
#[allow(clippy::cast_possible_truncation)]
const fn socklen<T>() -> libc::socklen_t {
    size_of::<T>() as libc::socklen_t
}

#[repr(C)]
struct SockaddrHci {
    family: libc::sa_family_t,
    dev: u16,
    channel: u16,
}

#[repr(C)]
struct HciFilter {
    type_mask: u32,
    event_mask: [u32; 2],
    opcode: u16,
}

/// Why an LE command didn't go through.
#[derive(Debug)]
pub enum CommandError {
    Io(io::Error),
    Status { ocf: u16, status: u8 },
    NoReply { ocf: u16 },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Io(e) => write!(f, "HCI socket: {e}"),
            CommandError::Status { ocf, status } => {
                write!(
                    f,
                    "HCI LE command 0x{ocf:04x} failed (status 0x{status:02x})"
                )
            }
            CommandError::NoReply { ocf } => write!(f, "no reply to HCI LE command 0x{ocf:04x}"),
        }
    }
}

impl From<io::Error> for CommandError {
    fn from(e: io::Error) -> Self {
        CommandError::Io(e)
    }
}

/// A raw HCI socket on one adapter, receiving only the given event types.
pub struct HciSocket {
    fd: OwnedFd,
}

impl HciSocket {
    pub fn open(dev: u16, events: &[u8]) -> io::Result<HciSocket> {
        // SAFETY: plain socket(2) call; the result is checked before use.
        let raw = unsafe {
            libc::socket(
                libc::c_int::from(AF_BLUETOOTH),
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                BTPROTO_HCI,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a freshly created descriptor that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };

        let addr = SockaddrHci {
            family: AF_BLUETOOTH,
            dev,
            channel: HCI_CHANNEL_RAW,
        };
        // SAFETY: `addr` is a valid sockaddr_hci and the length matches it.
        let r = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&raw const addr).cast(),
                socklen::<SockaddrHci>(),
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }

        let mask = events.iter().fold(0u64, |m, &e| m | 1 << e);
        #[allow(clippy::cast_possible_truncation)]
        let filter = HciFilter {
            type_mask: 1 << HCI_EVENT_PKT,
            event_mask: [mask as u32, (mask >> 32) as u32],
            opcode: 0,
        };
        // SAFETY: `filter` has the layout of struct hci_ufilter and the length matches it.
        let r = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                SOL_HCI,
                HCI_FILTER,
                (&raw const filter).cast(),
                socklen::<HciFilter>(),
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(HciSocket { fd })
    }

    /// Sends an LE command (OGF 0x08) and waits up to 2 s for its Command Complete; returns its status.
    /// The socket must have been opened with [`EVT_CMD_COMPLETE`].
    pub fn le_command(&self, ocf: u16, params: &[u8]) -> Result<u8, CommandError> {
        let opcode = (OGF_LE << 10) | ocf;
        let mut pkt = vec![HCI_COMMAND_PKT];
        pkt.extend_from_slice(&opcode.to_le_bytes());
        pkt.push(u8::try_from(params.len()).expect("HCI parameters fit in 255 bytes"));
        pkt.extend_from_slice(params);
        self.send(&pkt)?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buf = [0u8; 260];
        while let Some(n) = self.recv(&mut buf, deadline)? {
            let pkt = &buf[..n];
            // 04 0e <len> <ncmd> <opcode lo> <opcode hi> <status> ...
            if pkt.len() >= 7
                && pkt[1] == EVT_CMD_COMPLETE
                && u16::from_le_bytes([pkt[4], pkt[5]]) == opcode
            {
                return Ok(pkt[6]);
            }
        }
        Err(CommandError::NoReply { ocf })
    }

    /// Like [`le_command`](Self::le_command), but anything other than success is an error.
    pub fn le_command_ok(&self, ocf: u16, params: &[u8]) -> Result<(), CommandError> {
        match self.le_command(ocf, params)? {
            0 => Ok(()),
            status => Err(CommandError::Status { ocf, status }),
        }
    }

    fn send(&self, pkt: &[u8]) -> io::Result<()> {
        // SAFETY: the buffer is valid for its length.
        let n = unsafe { libc::send(self.fd.as_raw_fd(), pkt.as_ptr().cast(), pkt.len(), 0) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Waits until `deadline` for one packet; `None` when the time runs out.
    pub fn recv(&self, buf: &mut [u8], deadline: Instant) -> io::Result<Option<usize>> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let mut pfd = libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ms = libc::c_int::try_from(left.as_millis().max(1)).unwrap_or(libc::c_int::MAX);
            // SAFETY: one valid pollfd.
            match unsafe { libc::poll(&raw mut pfd, 1, ms) } {
                0 => return Ok(None),
                r if r < 0 => {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e);
                }
                _ => {}
            }
            // SAFETY: the buffer is valid for its length.
            let n =
                unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            return Ok(Some(n.cast_unsigned()));
        }
    }
}

/// The (address, data) of each advertising report in an LE Advertising Report or
/// LE Extended Advertising Report event. Addresses are in on-air order.
pub fn adv_reports(pkt: &[u8]) -> Vec<([u8; 6], &[u8])> {
    let mut out = Vec::new();
    // 04 3e <len> <subevent> <num reports> <reports...>
    if pkt.len() < 5 || pkt[0] != HCI_EVENT_PKT || pkt[1] != EVT_LE_META {
        return out;
    }
    let (sub, count, body) = (pkt[3], pkt[4], &pkt[5..]);
    // Where in each report the address, data length and data are
    let (addr_at, len_at) = match sub {
        0x02 => (2, 8),  // event type, address type, address, length, data, RSSI
        0x0D => (3, 23), // event type (2), address type, address, PHYs, SID, power, RSSI, interval, direct address, length, data
        _ => return out,
    };
    let trailer = usize::from(sub == 0x02); // legacy reports end with an RSSI byte
    let mut i = 0;
    for _ in 0..count {
        let (Some(addr), Some(&n)) = (body.get(i + addr_at..i + addr_at + 6), body.get(i + len_at))
        else {
            break;
        };
        let start = i + len_at + 1;
        let Some(data) = body.get(start..start + usize::from(n)) else {
            break;
        };
        out.push((addr.try_into().expect("6 bytes"), data));
        i = start + usize::from(n) + trailer;
    }
    out
}
