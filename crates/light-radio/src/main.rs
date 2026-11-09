//! Radio helper for the light remote. It is the only part with raw Bluetooth access (it is installed
//! with the `cap_net_raw` file capability), so it is deliberately small and only handles remote adverts:
//!
//! ```text
//! light-radio send <address, 12 hex digits> <advert data, 62 hex digits>
//! light-radio listen <seconds, 1-300>
//! ```
//!
//! `send` refuses anything that isn't a remote advert with a valid CRC. `listen` prints one line per
//! remote press heard, `<address> <advert data>`; whoever runs it must have LE scanning turned on
//! (`light-tray` asks BlueZ for that).

mod hci;

use std::fs::File;
use std::io::{self, Write};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use hci::{CommandError, EVT_CMD_COMPLETE, EVT_LE_META, HciSocket};
use light_protocol::{Address, Advert};

const HCI_DEV: u16 = 0;
/// Advertising set handle, clear of the ones the kernel uses (1-12).
const ADV_HANDLE: u8 = 0x20;
const MAX_LISTEN_SECONDS: u64 = 300;

// LE command OCFs
const SET_ADV_SET_RANDOM_ADDR: u16 = 0x0035;
const SET_EXT_ADV_PARAMS: u16 = 0x0036;
const SET_EXT_ADV_DATA: u16 = 0x0037;
const SET_EXT_ADV_ENABLE: u16 = 0x0039;
const REMOVE_ADV_SET: u16 = 0x003C;

fn main() -> ExitCode {
    // Stop if whoever started us goes away (set here: the kernel clears it when a capability binary starts)
    // SAFETY: plain prctl/getppid calls.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        if libc::getppid() == 1 {
            return ExitCode::FAILURE;
        }
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["send", address, data] => match (address.parse::<Address>(), data.parse::<Advert>()) {
            (Ok(address), Ok(advert)) => send(address, &advert),
            (Err(_), _) => return usage(),
            (_, Err(e)) => Err(format!("refusing to send: {e}")),
        },
        ["listen", seconds] => match seconds.parse::<u64>() {
            Ok(s @ 1..=MAX_LISTEN_SECONDS) => listen(Duration::from_secs(s)),
            _ => return usage(),
        },
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("light-radio: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: light-radio send <address, 12 hex digits> <advert data, 62 hex digits>\n       \
         light-radio listen <seconds, 1-{MAX_LISTEN_SECONDS}>"
    );
    ExitCode::from(2)
}

/// Transmits one press: about 0.5 s of the advert, like a real remote.
fn send(address: Address, advert: &Advert) -> Result<(), String> {
    // One press at a time, across every caller: lock our own executable
    let lock = File::open("/proc/self/exe").map_err(|e| format!("lock: {e}"))?;
    lock.lock().map_err(|e| format!("lock: {e}"))?;

    // Let a press finish before a SIGTERM/SIGINT takes effect, so the advertising set is always removed
    let _signals = BlockedSignals::new();

    let sock = HciSocket::open(HCI_DEV, &[EVT_CMD_COMPLETE]).map_err(|e| open_error(&e))?;
    remove_set(&sock);
    let result = transmit(&sock, address, advert);
    remove_set(&sock);
    result.map_err(|e| e.to_string())
}

fn transmit(sock: &HciSocket, address: Address, advert: &Advert) -> Result<(), CommandError> {
    #[rustfmt::skip]
    let params = [
        ADV_HANDLE,
        0x10, 0x00,         // properties: legacy ADV_NONCONN_IND
        0x20, 0x00, 0x00,   // min interval 20 ms
        0x20, 0x00, 0x00,   // max interval 20 ms
        0x01,               // channel 37 only
        0x01,               // own address: random
        0x00, 0, 0, 0, 0, 0, 0, // peer address (unused)
        0x00,               // no filter
        0x7F,               // TX power: no preference
        0x01, 0x00, 0x01,   // 1M PHY, no secondary skip, 1M PHY
        0x00, 0x00,         // SID 0, no scan request notifications
    ];
    sock.le_command_ok(SET_EXT_ADV_PARAMS, &params)?;

    let mut addr = vec![ADV_HANDLE];
    addr.extend_from_slice(&address.0);
    sock.le_command_ok(SET_ADV_SET_RANDOM_ADDR, &addr)?;

    let data = advert.as_bytes();
    let mut set_data = vec![
        ADV_HANDLE,
        0x03,
        0x01,
        u8::try_from(data.len()).expect("31 bytes"),
    ];
    set_data.extend_from_slice(data);
    sock.le_command_ok(SET_EXT_ADV_DATA, &set_data)?;

    // Enable for 500 ms (50 x 10 ms); the controller stops it by itself
    sock.le_command_ok(
        SET_EXT_ADV_ENABLE,
        &[0x01, 0x01, ADV_HANDLE, 0x32, 0x00, 0x00],
    )?;
    thread::sleep(Duration::from_millis(600));
    Ok(())
}

/// Disables and removes our advertising set, if there is one; failures are expected and ignored.
fn remove_set(sock: &HciSocket) {
    let _ = sock.le_command(SET_EXT_ADV_ENABLE, &[0x00, 0x01, ADV_HANDLE, 0, 0, 0]);
    let _ = sock.le_command(REMOVE_ADV_SET, &[ADV_HANDLE]);
}

/// Prints each remote press heard for `duration`, once per press rather than once per copy.
fn listen(duration: Duration) -> Result<(), String> {
    let sock = HciSocket::open(HCI_DEV, &[EVT_LE_META]).map_err(|e| open_error(&e))?;
    let deadline = Instant::now() + duration;
    let mut last: Option<Advert> = None;
    let mut buf = [0u8; 300];
    let mut out = io::stdout().lock();
    while let Some(n) = sock
        .recv(&mut buf, deadline)
        .map_err(|e| format!("HCI socket: {e}"))?
    {
        for (addr, data) in hci::adv_reports(&buf[..n]) {
            let Ok(advert) = Advert::parse(data) else {
                continue;
            };
            if last == Some(advert) {
                continue;
            }
            last = Some(advert);
            // The reader has gone: nothing more to do
            if writeln!(out, "{} {advert}", Address(addr))
                .and_then(|()| out.flush())
                .is_err()
            {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn open_error(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(libc::EPERM | libc::EACCES) => {
            format!(
                "no permission for raw Bluetooth ({e}); is light-radio installed with cap_net_raw?"
            )
        }
        Some(libc::ENODEV) => format!("no Bluetooth adapter hci{HCI_DEV} ({e})"),
        _ => format!("opening the Bluetooth adapter: {e}"),
    }
}

/// Holds SIGTERM, SIGINT and SIGHUP back until dropped; any that arrived are then delivered.
struct BlockedSignals(libc::sigset_t);

impl BlockedSignals {
    fn new() -> Self {
        // SAFETY: the sigset is initialised by sigemptyset before use.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut set);
            for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
                libc::sigaddset(&raw mut set, sig);
            }
            libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
            BlockedSignals(set)
        }
    }
}

impl Drop for BlockedSignals {
    fn drop(&mut self) {
        // SAFETY: unblocks the set blocked in new().
        unsafe {
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &raw const self.0, std::ptr::null_mut());
        }
    }
}
