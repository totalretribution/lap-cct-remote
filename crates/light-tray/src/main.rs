//! `light-tray`: a tray icon that controls a LAP CCT LED panel.
//!
//! - Menu (right-click, or left-click where the desktop allows): every button, plus pairing.
//! - Scroll on the icon: brighter / dimmer.
//! - Middle-click: power.
//! - Pairing opens a window showing what's happening.
//!
//! Uses StatusNotifierItem over D-Bus, so it shows on KDE, Cinnamon, and GNOME with the
//! AppIndicator extension. Threads: the tray service runs on its own, presses and pairing run one
//! at a time on a worker, and the main thread shows the pairing window when asked.

mod autostart;
mod bluez;
mod config;
mod controller;
mod dialog;
mod icon;
mod pairing;
mod radio;
mod update;

use std::fs::File;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use controller::{Controller, Error, PairProgress};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, Disposition, StandardItem};
use ksni::{MenuItem, Orientation, ToolTip};
use light_protocol::{Button, RemoteId};
use pairing::{Phase, Session};

/// Presses allowed to wait behind the one being sent; more are dropped (fast scrolling).
const MAX_QUEUED: usize = 2;

/// Work for the worker thread.
enum Job {
    Press(Button),
    Pair(Session),
    Unpair,
}

/// Requests for the main (window) thread.
enum Ui {
    ShowPairing(Session),
    Quit,
}

struct LightTray {
    controller: Controller,
    jobs: Sender<Job>,
    ui: Sender<Ui>,
    quitting: Arc<AtomicBool>,
    queued: Arc<AtomicUsize>,
    paired: Option<RemoteId>,
    /// The pairing attempt in progress, if any.
    pairing: Option<Session>,
    error: Option<String>,
    /// A newer release's tag (`v0.2.0`), once the update check finds one.
    new_version: Option<String>,
    /// Whether the tray starts at login; read again each time the menu opens.
    autostart: bool,
}

impl LightTray {
    fn is_pairing(&self) -> bool {
        self.pairing.is_some()
    }

    fn press(&mut self, button: Button) {
        if self.paired.is_none() || self.is_pairing() {
            return;
        }
        if self.queued.fetch_add(1, Ordering::SeqCst) > MAX_QUEUED {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let _ = self.jobs.send(Job::Press(button));
    }

    /// Starts pairing; `None` if it's already running.
    fn start_pairing(&mut self) -> Option<Session> {
        if self.is_pairing() {
            return None;
        }
        let session = Session::new();
        self.pairing = Some(session.clone());
        self.error = None;
        let _ = self.jobs.send(Job::Pair(session.clone()));
        Some(session)
    }

    fn cancel_pairing(&mut self) {
        if let Some(session) = &self.pairing {
            session.cancel();
        }
    }

    fn reload(&mut self) {
        match self.controller.pairing() {
            Ok(p) => self.paired = p.map(|p| p.remote_id()),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn status_line(&self) -> String {
        match (self.pairing.as_ref().map(Session::phase), self.paired) {
            (
                Some(Phase::Listening {
                    heard: Some(id), ..
                }),
                _,
            ) => {
                format!("Heard {id}: press POWER once more")
            }
            (Some(_), _) => "Pairing: press POWER twice on your remote".to_owned(),
            (None, Some(id)) => format!("Paired to remote {id}"),
            (None, None) => "No remote paired".to_owned(),
        }
    }
}

fn item(
    label: &str,
    icon: &str,
    enabled: bool,
    activate: impl Fn(&mut LightTray) + Send + 'static,
) -> MenuItem<LightTray> {
    StandardItem {
        label: label.to_owned(),
        icon_name: icon.to_owned(),
        enabled,
        activate: Box::new(activate),
        ..Default::default()
    }
    .into()
}

fn info(label: String, disposition: Disposition) -> MenuItem<LightTray> {
    StandardItem {
        label,
        enabled: false,
        disposition,
        ..Default::default()
    }
    .into()
}

fn button_icon(button: Button) -> &'static str {
    match button {
        Button::Power => "system-shutdown-symbolic",
        Button::Night => "weather-clear-night-symbolic",
        Button::Brighter => "list-add-symbolic",
        Button::Dimmer => "list-remove-symbolic",
        Button::Cooler | Button::Warmer => "",
    }
}

impl ksni::Tray for LightTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "light-remote".into()
    }

    fn title(&self) -> String {
        "Light remote".into()
    }

    fn icon_name(&self) -> String {
        icon::NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon::tray_pixmaps()
    }

    fn overlay_icon_name(&self) -> String {
        if self.is_pairing() {
            "bluetooth-active-symbolic".into()
        } else {
            String::new()
        }
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn tool_tip(&self) -> ToolTip {
        let mut description = self.status_line();
        if let Some(e) = &self.error {
            description = format!("{description}\n{e}");
        } else if self.paired.is_some() && !self.is_pairing() {
            description.push_str("\nScroll to dim, middle-click for power");
        }
        if let Some(v) = &self.new_version {
            description = format!("{description}\nUpdate {v} available");
        }
        ToolTip {
            title: "Light remote".into(),
            description,
            ..Default::default()
        }
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.press(Button::Power);
    }

    fn scroll(&mut self, delta: i32, orientation: Orientation) {
        if matches!(orientation, Orientation::Vertical) && delta != 0 {
            self.press(if delta > 0 {
                Button::Brighter
            } else {
                Button::Dimmer
            });
        }
    }

    fn menu_about_to_show(&mut self) {
        // Pick up pairing changes made outside the tray (e.g. a deleted remote.json)
        if !self.is_pairing() {
            self.reload();
        }
        self.autostart = autostart::enabled();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let pairing = self.is_pairing();
        let can_press = self.paired.is_some() && !pairing;
        let mut menu = vec![info(self.status_line(), Disposition::Normal)];
        if let Some(e) = &self.error {
            menu.push(info(e.clone(), Disposition::Warning));
        }
        menu.push(MenuItem::Separator);
        for button in Button::ALL {
            menu.push(item(
                button.label(),
                button_icon(button),
                can_press,
                move |t| t.press(button),
            ));
        }
        menu.push(MenuItem::Separator);
        if pairing {
            menu.push(item(
                "Cancel pairing",
                "process-stop-symbolic",
                true,
                LightTray::cancel_pairing,
            ));
        } else {
            let label = if self.paired.is_some() {
                "Pair a different remote…"
            } else {
                "Pair remote…"
            };
            menu.push(item(label, "bluetooth-symbolic", true, |t| {
                if let Some(session) = t.start_pairing() {
                    let _ = t.ui.send(Ui::ShowPairing(session));
                }
            }));
            if self.paired.is_some() {
                menu.push(item("Unpair", "", true, |t| {
                    let _ = t.jobs.send(Job::Unpair);
                }));
            }
        }
        menu.push(MenuItem::Separator);
        menu.push(
            CheckmarkItem {
                label: "Start at login".into(),
                checked: self.autostart,
                activate: Box::new(|t: &mut Self| {
                    let on = !t.autostart;
                    match autostart::set(on) {
                        Ok(()) => t.autostart = on,
                        Err(e) => t.error = Some(format!("Couldn't change starting at login: {e}")),
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        if let Some(v) = &self.new_version {
            menu.push(item(
                &format!("Get update {v}…"),
                "software-update-available-symbolic",
                true,
                |_| update::open_releases(),
            ));
        }
        menu.push(item("Quit", "application-exit-symbolic", true, |t| {
            t.cancel_pairing();
            t.quitting.store(true, Ordering::SeqCst);
            let _ = t.ui.send(Ui::Quit);
        }));
        menu
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        // Keep running: the panel may come back (e.g. after a desktop restart)
        true
    }
}

/// Runs presses and pairing one at a time, reporting back to the tray and the pairing window.
fn worker(
    controller: &Controller,
    jobs: &Receiver<Job>,
    queued: &AtomicUsize,
    tray: &Handle<LightTray>,
) {
    for job in jobs {
        match job {
            Job::Press(button) => {
                let result = controller.press(button);
                queued.fetch_sub(1, Ordering::SeqCst);
                let message = result.err().map(|e| e.to_string());
                if let Some(m) = &message {
                    notify("Couldn't press the light's button", m);
                }
                tray.update(|t| t.error = message);
            }
            Job::Pair(session) => {
                let result = controller.pair(&session.cancel_handle(), |progress| {
                    match progress {
                        PairProgress::Listening => session.set_phase(Phase::Listening {
                            heard: None,
                            presses: 0,
                        }),
                        PairProgress::Heard(id, presses) => session.heard(id, presses),
                    }
                    tray.update(|_| ());
                });
                session.set_phase(match result {
                    Ok(p) => Phase::Paired(p.remote_id()),
                    Err(Error::Cancelled) => Phase::Cancelled,
                    Err(Error::NothingHeard) => Phase::NothingHeard,
                    Err(e) => Phase::Failed(e.to_string()),
                });
                tray.update(|t| {
                    t.pairing = None;
                    t.reload();
                });
            }
            Job::Unpair => {
                let result = controller.unpair();
                tray.update(|t| {
                    t.reload();
                    t.error = result.err().map(|e| e.to_string());
                });
            }
        }
    }
}

fn notify(summary: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .appname("Light remote")
        .icon(icon::NAME)
        .summary(summary)
        .body(body)
        .show();
}

/// Keeps a second copy from starting; held for the life of the process.
fn single_instance() -> Option<File> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let file = File::create(dir.join("light-tray.lock")).ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn main() -> ExitCode {
    let Some(_instance) = single_instance() else {
        eprintln!("light-tray: already running");
        // Started from the menu: say where it is rather than doing nothing
        notify(
            "Light remote is already running",
            "Use its icon in the system tray.",
        );
        return ExitCode::SUCCESS;
    };
    let controller = match Controller::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("light-tray: {e}");
            return ExitCode::FAILURE;
        }
    };

    let (jobs_tx, jobs_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let quitting = Arc::new(AtomicBool::new(false));
    let mut tray = LightTray {
        controller: controller.clone(),
        jobs: jobs_tx,
        ui: ui_tx,
        quitting: Arc::clone(&quitting),
        queued: Arc::clone(&queued),
        paired: None,
        pairing: None,
        error: None,
        new_version: None,
        autostart: autostart::enabled(),
    };
    tray.reload();

    // Wait for a tray host rather than failing, e.g. when started at login before the panel
    let handle = match tray.assume_sni_available(true).spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("light-tray: can't show a tray icon: {e}");
            eprintln!(
                "light-tray: on GNOME, install the \"AppIndicator and KStatusNotifierItem Support\" extension"
            );
            return ExitCode::FAILURE;
        }
    };

    let worker_handle = handle.clone();
    thread::spawn(move || worker(&controller, &jobs_rx, &queued, &worker_handle));

    // Tell the user about a newer release once, when it's first found
    let update_handle = handle.clone();
    update::spawn(move |version| {
        let first = update_handle
            .update(|t| t.new_version.replace(version.clone()).as_ref() != Some(&version))
            .unwrap_or(false);
        if first {
            notify(
                &format!("Light remote {version} is available"),
                &format!(
                    "You have v{}. Choose \"Get update\" in the tray menu to download it.",
                    update::VERSION
                ),
            );
        }
    });

    // The main thread owns the windowing event loop, so the pairing window opens here
    while let Ok(Ui::ShowPairing(session)) = ui_rx.recv() {
        let tray = handle.clone();
        let restart: dialog::Restart =
            Box::new(move || tray.update(LightTray::start_pairing).flatten());
        if let Err(e) = dialog::show(
            session.clone(),
            restart,
            Arc::clone(&quitting),
            icon::window_icon(),
        ) {
            eprintln!("light-tray: can't open the pairing window: {e}");
            notify(
                "Pairing",
                "Couldn't open the pairing window; the tray menu shows progress.",
            );
        }
        if quitting.load(Ordering::SeqCst) {
            break;
        }
    }
    if !quitting.load(Ordering::SeqCst) {
        eprintln!("light-tray: the tray service stopped; exiting");
    }
    handle.shutdown().wait();
    ExitCode::SUCCESS
}
