//! The paired remote, saved in `$XDG_CONFIG_HOME/light-remote/remote.json` as
//! `{"address": "<12 hex>", "data": "<62 hex>", "seq": <next sequence number>}`.

use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::PathBuf;

use light_protocol::{Address, Advert, RemoteId};
use serde::{Deserialize, Serialize};

use crate::controller::Error;

/// The remote this PC pretends to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pairing {
    /// The address the remote sends from.
    pub address: Address,
    /// One of the remote's adverts, the template for new presses.
    pub template: Advert,
    /// The sequence number for the next press.
    pub seq: u8,
}

impl Pairing {
    /// Pairs with the remote that sent `advert`, carrying on from its sequence number.
    #[must_use]
    pub fn from_heard(address: Address, advert: Advert) -> Self {
        Pairing {
            address,
            template: advert,
            seq: advert.seq().wrapping_add(1),
        }
    }

    #[must_use]
    pub fn remote_id(&self) -> RemoteId {
        self.template.remote_id()
    }
}

#[derive(Serialize, Deserialize)]
struct PairingFile {
    address: String,
    data: String,
    seq: u8,
}

/// Where the pairing lives, and the lock that keeps presses from different processes in order.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The usual place: `$XDG_CONFIG_HOME/light-remote`, or `~/.config/light-remote`.
    pub fn from_env() -> Result<Self, Error> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or(Error::NoHome)?;
        Ok(Store::at(base.join("light-remote")))
    }

    #[must_use]
    pub fn at(dir: PathBuf) -> Self {
        Store { dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("remote.json")
    }

    /// Takes the exclusive lock, held until the returned file is dropped.
    pub fn lock(&self) -> Result<File, Error> {
        fs::create_dir_all(&self.dir).map_err(|e| Error::config(&self.dir, e))?;
        let path = self.dir.join(".lock");
        let file = File::create(&path).map_err(|e| Error::config(&path, e))?;
        file.lock().map_err(|e| Error::config(&path, e))?;
        Ok(file)
    }

    pub fn load(&self) -> Result<Option<Pairing>, Error> {
        let path = self.file();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::config(&path, e)),
        };
        let bad = |what: &str| Error::BadConfig {
            path: path.clone(),
            reason: what.to_owned(),
        };
        let file: PairingFile = serde_json::from_str(&text).map_err(|e| bad(&e.to_string()))?;
        Ok(Some(Pairing {
            address: file.address.parse().map_err(|_| bad("bad address"))?,
            template: file.data.parse().map_err(|e| bad(&format!("data: {e}")))?,
            seq: file.seq,
        }))
    }

    /// Saves atomically (write, then rename).
    pub fn save(&self, pairing: &Pairing) -> Result<(), Error> {
        let file = PairingFile {
            address: pairing.address.to_string(),
            data: pairing.template.to_string(),
            seq: pairing.seq,
        };
        let json = serde_json::to_string_pretty(&file).expect("plain struct serialises");
        fs::create_dir_all(&self.dir).map_err(|e| Error::config(&self.dir, e))?;
        let path = self.file();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json + "\n").map_err(|e| Error::config(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| Error::config(&path, e))
    }

    pub fn remove(&self) -> Result<(), Error> {
        let path = self.file();
        match fs::remove_file(&path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(Error::config(&path, e)),
            _ => Ok(()),
        }
    }
}
