//! Controls the office light as its remote.
//!
//! [`Controller`] is everything the tray needs: pair with a remote by hearing it, then send presses as that remote.
//! Radio access goes through the small privileged `light-radio` helper; everything else runs as the user.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use light_protocol::{Address, Advert, Button, RemoteId};

use crate::bluez;
use crate::config::{Pairing, Store};
use crate::radio::Radio;

/// How long pairing listens for.
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(45);
/// The button pairing listens for. Pressed twice, it turns the light off and back on, leaving it as it was.
pub const PAIR_BUTTON: Button = Button::Power;
/// How many POWER presses from one remote pairing needs, so a colleague's press can't pair the wrong one.
pub const PAIR_PRESSES: usize = 2;
/// The adapter used (the helper is fixed to the same one).
const ADAPTER: &str = "hci0";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no remote paired yet")]
    NotPaired,
    #[error("no remote heard twice; try again, closer to the PC")]
    NothingHeard,
    #[error("cancelled")]
    Cancelled,
    #[error("can't find the home folder")]
    NoHome,
    #[error("{path}: {source}")]
    Config {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {reason}")]
    BadConfig { path: PathBuf, reason: String },
    #[error("{} not found; is light-remote installed?", .0.display())]
    RadioMissing(PathBuf),
    #[error("{0}")]
    Radio(String),
    #[error("{0}")]
    Bluetooth(String),
}

impl Error {
    pub(crate) fn config(path: &std::path::Path, source: std::io::Error) -> Self {
        Error::Config {
            path: path.to_owned(),
            source,
        }
    }
}

/// Stops a pairing in progress from another thread.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<Mutex<CancelState>>);

#[derive(Debug, Default)]
struct CancelState {
    cancelled: bool,
    listener: Option<u32>,
}

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Cancel::default()
    }

    pub fn cancel(&self) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cancelled = true;
        if let Some(pid) = state.listener {
            // SAFETY: plain kill(2) of our own child, which is still running while its pid is registered.
            unsafe {
                libc::kill(
                    libc::pid_t::try_from(pid).expect("pid fits pid_t"),
                    libc::SIGTERM,
                );
            }
        }
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancelled
    }

    /// Registers the listener to stop; false if already cancelled.
    fn watch(&self, pid: Option<u32>) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.listener = pid;
        !state.cancelled
    }
}

/// Something that happened while pairing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairProgress {
    /// Listening; press POWER twice.
    Listening,
    /// Heard a POWER press from this remote, the given number of different presses from it so far.
    Heard(RemoteId, usize),
}

/// Pairs with and presses the light's remote.
#[derive(Clone, Debug)]
pub struct Controller {
    store: Store,
    radio: Radio,
}

impl Controller {
    /// The installed setup: the user's config folder and the installed helper.
    pub fn from_env() -> Result<Self, Error> {
        Ok(Controller::new(Store::from_env()?, Radio::from_env()))
    }

    #[must_use]
    pub fn new(store: Store, radio: Radio) -> Self {
        Controller { store, radio }
    }

    /// The paired remote, if any.
    pub fn pairing(&self) -> Result<Option<Pairing>, Error> {
        self.store.load()
    }

    /// Presses a button once. Presses from every process go out one at a time, in sequence order.
    pub fn press(&self, button: Button) -> Result<(), Error> {
        let _lock = self.store.lock()?;
        let mut pairing = self.store.load()?.ok_or(Error::NotPaired)?;
        let advert = pairing
            .template
            .press(pairing.seq, button, fastrand::u8(..));
        // Use up the sequence number before sending, so a failed send never repeats it
        pairing.seq = pairing.seq.wrapping_add(1);
        self.store.save(&pairing)?;
        self.radio.send(pairing.address, &advert)
    }

    /// Listens for [`PAIR_PRESSES`] POWER presses from one remote and pairs with it.
    pub fn pair(
        &self,
        cancel: &Cancel,
        mut progress: impl FnMut(PairProgress),
    ) -> Result<Pairing, Error> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let _scan = bluez::Scan::start(ADAPTER)?;
        let mut listener = self.radio.listen(PAIR_TIMEOUT)?;
        if !cancel.watch(Some(listener.pid())) {
            return Err(Error::Cancelled);
        }
        progress(PairProgress::Listening);

        // Per remote heard: its latest POWER press and how many different presses we've had
        let mut heard: Vec<(Address, Advert, usize)> = Vec::new();
        let mut paired = None;
        while let Some((address, advert)) = listener.next_press() {
            if advert.button() != Some(PAIR_BUTTON) {
                continue;
            }
            let i = match heard
                .iter()
                .position(|(a, t, _)| *a == address && t.same_remote(&advert))
            {
                Some(i) if heard[i].1.seq() == advert.seq() => continue, // the same press again
                Some(i) => i,
                None => {
                    heard.push((address, advert, 0));
                    heard.len() - 1
                }
            };
            let entry = &mut heard[i];
            entry.1 = advert;
            entry.2 += 1;
            progress(PairProgress::Heard(advert.remote_id(), entry.2));
            if entry.2 >= PAIR_PRESSES {
                paired = Some(Pairing::from_heard(address, advert));
                break;
            }
        }
        cancel.watch(None);
        let finished = listener.finish();

        if let Some(pairing) = paired {
            let _lock = self.store.lock()?;
            self.store.save(&pairing)?;
            return Ok(pairing);
        }
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        finished?;
        Err(Error::NothingHeard)
    }

    /// Forgets the paired remote.
    pub fn unpair(&self) -> Result<(), Error> {
        let _lock = self.store.lock()?;
        self.store.remove()
    }
}
