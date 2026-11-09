//! Runs the privileged `light-radio` helper.

use std::io::{BufRead, BufReader, ErrorKind, Lines, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use light_protocol::{Address, Advert};

use crate::controller::Error;

/// Where the helper is installed: by `install.sh` (checked first, so a source install wins), or by the packages.
pub const INSTALL_PATHS: [&str; 2] = ["/usr/local/libexec/light-radio", "/usr/libexec/light-radio"];
/// The group packages let run the helper.
pub const GROUP: &str = "light-remote";

#[derive(Clone, Debug)]
pub struct Radio {
    path: PathBuf,
}

impl Radio {
    /// The helper named by `$LIGHT_RADIO` (for development), or else the installed one.
    #[must_use]
    pub fn from_env() -> Self {
        let path = std::env::var_os("LIGHT_RADIO").map_or_else(
            || {
                let installed = INSTALL_PATHS
                    .iter()
                    .find(|p| std::path::Path::new(p).exists());
                PathBuf::from(installed.unwrap_or(&INSTALL_PATHS[1]))
            },
            PathBuf::from,
        );
        Radio { path }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.path);
        cmd.stdin(Stdio::null());
        cmd
    }

    fn spawn_error(&self, e: &std::io::Error) -> Error {
        if e.kind() == ErrorKind::NotFound {
            Error::RadioMissing(self.path.clone())
        } else if e.kind() == ErrorKind::PermissionDenied {
            Error::Radio(format!(
                "not allowed to run {}: add yourself to the {GROUP} group \
                 (sudo usermod -aG {GROUP} $USER), then log out and back in",
                self.path.display()
            ))
        } else {
            Error::Radio(format!("running {}: {e}", self.path.display()))
        }
    }

    /// Transmits one press. Takes about 0.7 s.
    pub fn send(&self, address: Address, advert: &Advert) -> Result<(), Error> {
        let out = self
            .command()
            .args(["send", &address.to_string(), &advert.to_string()])
            .stdout(Stdio::null())
            .output()
            .map_err(|e| self.spawn_error(&e))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::Radio(helper_message(&out.stderr)))
        }
    }

    /// Starts listening for remote presses for up to `duration`.
    pub fn listen(&self, duration: Duration) -> Result<Listener, Error> {
        let mut child = self
            .command()
            .args(["listen", &duration.as_secs().clamp(1, 300).to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| self.spawn_error(&e))?;
        let lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
        Ok(Listener { child, lines })
    }
}

fn helper_message(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    let text = text.strip_prefix("light-radio: ").unwrap_or(text);
    if text.is_empty() {
        "light-radio failed".to_owned()
    } else {
        text.to_owned()
    }
}

/// A running `light-radio listen`. Stopped when dropped.
pub struct Listener {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Listener {
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The next press heard; `None` once the helper stops (time up, cancelled, or failed).
    pub fn next_press(&mut self) -> Option<(Address, Advert)> {
        for line in self.lines.by_ref() {
            let Ok(line) = line else { return None };
            let mut parts = line.split_whitespace();
            if let (Some(Ok(address)), Some(Ok(advert))) =
                (parts.next().map(str::parse), parts.next().map(str::parse))
            {
                return Some((address, advert));
            }
        }
        None
    }

    /// Stops the helper; an error if it failed rather than finished or was stopped.
    pub fn finish(mut self) -> Result<(), Error> {
        let _ = self.child.kill();
        let status = self.child.wait().map_err(|e| Error::Radio(e.to_string()))?;
        let mut stderr = Vec::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_end(&mut stderr);
        }
        // Killed by us (or a cancel) is fine; a non-zero exit is a real failure
        match status.code() {
            Some(0) | None => Ok(()),
            Some(_) => Err(Error::Radio(helper_message(&stderr))),
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
