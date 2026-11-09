//! Checks GitHub releases for a newer version.
//!
//! Only tells the user: packages own /usr/bin, so the tray never replaces itself.

use std::process::Command;
use std::thread;
use std::time::Duration;

use update_informer::{Check, registry};

const REPO: &str = "totalretribution/lap-cct-remote";
pub const RELEASES_URL: &str = "https://github.com/totalretribution/lap-cct-remote/releases/latest";

/// Set to any value to turn the check off.
const OPT_OUT: &str = "LIGHT_REMOTE_NO_UPDATE_CHECK";

/// This build's version: the release's for packages (`package.sh` sets it from the tag), otherwise Cargo.toml's.
pub const VERSION: &str = match option_env!("LIGHT_REMOTE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// Wait after starting before the first check, so the network is up after logging in.
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// How often the tray looks again; GitHub is asked at most once per [`GITHUB_INTERVAL`], and the
/// answer is cached in between (also across restarts).
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);
const GITHUB_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Checks in the background for as long as the tray runs, calling `found` with the newer
/// version's tag (`v0.2.0`) whenever there is one. Offline, rate limited or a bad response: says nothing.
pub fn spawn(found: impl Fn(String) + Send + 'static) {
    if std::env::var_os(OPT_OUT).is_some() {
        return;
    }
    thread::spawn(move || {
        thread::sleep(FIRST_CHECK);
        loop {
            let informer = update_informer::new(registry::GitHub, REPO, VERSION)
                .interval(GITHUB_INTERVAL)
                .timeout(Duration::from_secs(10));
            if let Ok(Some(version)) = informer.check_version() {
                found(version.to_string());
            }
            thread::sleep(RECHECK);
        }
    });
}

/// Opens the latest release's page in the browser.
pub fn open_releases() {
    thread::spawn(|| {
        let _ = Command::new("xdg-open").arg(RELEASES_URL).status();
    });
}
