//! Starting the tray at login, per user, through XDG autostart.
//!
//! Packages install `/etc/xdg/autostart/light-tray.desktop`, which starts the tray for everyone. A
//! user turns that off with their own copy in `~/.config/autostart` marked `Hidden=true`, which the
//! desktop reads instead. Without the system entry (a source install), the user's copy is the only one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILE: &str = "light-tray.desktop";
const SYSTEM_DIRS: &str = "/etc/xdg";
const ENTRY: &str = include_str!("../assets/light-tray.desktop");

/// Whether the tray starts when this user logs in.
pub fn enabled() -> bool {
    match fs::read_to_string(user_file()) {
        Ok(entry) => !disabled(&entry),
        Err(_) => system_file().is_some_and(|f| fs::read_to_string(f).is_ok_and(|e| !disabled(&e))),
    }
}

/// Turns starting at login on or off for this user.
pub fn set(on: bool) -> io::Result<()> {
    let user = user_file();
    if system_file().is_some() {
        // The system entry starts it, so the user's copy only ever turns it off
        if on {
            remove(&user)
        } else {
            write(&user, &format!("{ENTRY}Hidden=true\n"))
        }
    } else if on {
        write(&user, ENTRY)
    } else {
        remove(&user)
    }
}

/// `Hidden=true` removes an entry; GNOME's own switch does the same for its sessions.
fn disabled(entry: &str) -> bool {
    entry.lines().map(str::trim).any(|line| {
        line.eq_ignore_ascii_case("Hidden=true")
            || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

fn user_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    config.join("autostart").join(FILE)
}

/// The first system-wide entry, searching `$XDG_CONFIG_DIRS` like the desktop does.
fn system_file() -> Option<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|d| !d.is_empty());
    dirs.as_deref()
        .unwrap_or(SYSTEM_DIRS)
        .split(':')
        .map(|d| Path::new(d).join("autostart").join(FILE))
        .find(|f| f.is_file())
}

fn write(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, contents)
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
