# Office light remote

A tray icon that controls the office LED panels (LAP TP22-R-CCT, driver EHC_050S105DN_TP02 "50W-2.4G-CCT") from a
Linux PC's built-in Bluetooth, by sending the same Bluetooth LE adverts their remotes send.

## Install

You need Linux with a Bluetooth adapter, BlueZ, Rust, polkit (`pkexec`) and `setcap`. The installer checks these.

```
./install.sh
```

Then right-click the tray icon, choose **Pair remote…**, and press **POWER** twice on your light's remote
(the light turns off and back on).

To remove it, run `./uninstall.sh`.

## Packages

`./package.sh` builds `packages/light-remote_<version>_amd64.deb` and `packages/light-remote-<version>.x86_64.rpm`.
The binaries are built against glibc 2.31, so the packages install on Debian 11, Ubuntu 20.04, Mint 20 and newer,
and on Fedora. It needs zig and `cargo install --locked cargo-deb cargo-generate-rpm cargo-zigbuild`.

The packages install to `/usr` (`/usr/bin/light-tray`, `/usr/libexec/light-radio`, the icon and a menu entry), and
start the tray at login for every user (`/etc/xdg/autostart`). Only members of the `light-remote` group can use the
light: installing creates the group and adds whoever installed it through `sudo` or `pkexec`. Add anyone else with
`sudo usermod -aG light-remote <user>`; they need to log out and back in.

The `.deb` maintainer is a placeholder (`root@localhost`); set a real one in `crates/light-tray/Cargo.toml` before
distributing the packages further.

### Releases

GitHub Actions (`.github/workflows/packages.yml`) checks the code and builds both packages on every push to `main` and
every pull request; they're attached to the run as an artifact.

To release, create a release on GitHub with a tag like `v0.2.0` and publish it. The workflow builds the packages at
that version (`0.2.0`) and attaches them to the release; there's no need to change `Cargo.toml`. Versions are numbers
and dots (letters allowed, but no `-`), as Debian and RPM require. Locally, `./package.sh 0.2.0` does the same.

## Use

The icon shows on KDE, Cinnamon, and GNOME with the "AppIndicator and KStatusNotifierItem Support" extension.

| Do | Gets |
| --- | --- |
| Right-click (or left-click) | Menu: Power, Night, Brighter, Dimmer, Cooler, Warmer; pair / unpair; quit |
| Scroll on the icon | Brighter / dimmer |
| Middle-click | Power |

The tooltip and the top of the menu show which remote it's paired to. **Pair remote…** opens a window that shows
pairing as it happens: a countdown, a dot for each POWER press heard, and the result, with Cancel / Try again.
Closing the window cancels pairing. Errors from presses show as notifications.

## Layout

| Crate | What |
| --- | --- |
| `crates/light-tray` | The program. `main.rs` is the tray (`ksni`, StatusNotifierItem over D-Bus) and the threads. `dialog.rs` is the pairing window (`egui`/`eframe`); `pairing.rs` is the pairing state the tray, worker and window share. `controller.rs` pairs and presses; `config.rs` is the pairing file; `radio.rs` runs the helper; `bluez.rs` turns on LE scanning for pairing. `icon.rs` and `assets/` are the icon and menu entry; `packaging/` has the package install scripts |
| `crates/light-radio` | The helper the tray runs for radio access, and the only privileged part: a raw HCI socket. `send` transmits one advert and refuses anything that isn't a remote advert with a valid CRC; `listen` prints remote presses heard |
| `crates/protocol` | `light-protocol`: `Advert` (parse, check, build presses), `Button`, `Address`, CRC-16. `no_std`, no dependencies; shared by both |

**Privileges.** `light-radio` is installed to `/usr/local/libexec` as `root:<your primary group>` by `install.sh`, or to
`/usr/libexec` as `root:light-remote` by the packages; either way mode `0750`, with the `cap_net_raw` file capability. The tray itself never runs privileged. Pairing asks BlueZ (over
D-Bus, as you) for LE scanning with duplicates reported; `light-radio listen` only reads the reports.

**Threads.** The tray service runs on its own thread; presses and pairing run one at a time on a worker; the main
thread owns the windowing event loop and opens the pairing window when asked.

**Icon.** `assets/light-remote.svg` is the source; the PNGs next to it are rendered from it
(`magick -background none -density 384 light-remote.svg -resize 64x64 light-remote-64.png`, and so on). The tray
gets them built in, and `install.sh` puts them in the icon theme for the menu entry and notifications.

**Ordering.** Presses run one at a time on a worker thread, under a lock in `~/.config/light-remote/.lock` around the
sequence number and the send. Fast scrolling queues at most two presses behind the one being sent.

## Development

```
cargo clippy --workspace --all-targets
```

To run an uninstalled build, give your helper the capability and point the tray at it:

```
cargo build
pkexec setcap cap_net_raw+ep "$PWD/target/debug/light-radio"
LIGHT_RADIO=$PWD/target/debug/light-radio target/debug/light-tray
```

Each rebuild of `light-radio` drops the capability, so run `setcap` again after one.

The protocol is described in `crates/protocol/src/lib.rs`.

## Licence

GPL-3.0-or-later: you can redistribute and/or modify it under the terms of the GNU General Public License as published
by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See
[LICENSE](LICENSE).
