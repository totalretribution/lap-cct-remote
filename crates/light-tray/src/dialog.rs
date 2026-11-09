//! The pairing window: what's happening while pairing, and how it ended.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Vec2, ViewportCommand};

use crate::controller::{PAIR_PRESSES, PAIR_TIMEOUT};
use crate::pairing::{Phase, Session};

/// Starts a new attempt from the window's "Try again"; `None` if one is already running.
pub type Restart = Box<dyn Fn() -> Option<Session>>;

/// Shows the window until it's closed (or the tray quits). Closing it cancels pairing still in progress.
pub fn show(
    session: Session,
    restart: Restart,
    quitting: Arc<AtomicBool>,
    icon: Option<egui::IconData>,
) -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Pair remote")
        .with_app_id("light-tray")
        .with_inner_size([380.0, 290.0])
        .with_resizable(false);
    if let Some(icon) = &icon {
        viewport = viewport.with_icon(Arc::new(icon.clone()));
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "light-remote-pairing",
        options,
        Box::new(move |cc| {
            session.repaint_with(&cc.egui_ctx);
            let icon = icon.map(|i| {
                let size = [i.width as usize, i.height as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &i.rgba);
                cc.egui_ctx
                    .load_texture("light-remote", image, egui::TextureOptions::LINEAR)
            });
            Ok(Box::new(PairDialog {
                session,
                restart,
                quitting,
                icon,
            }))
        }),
    )
}

struct PairDialog {
    session: Session,
    restart: Restart,
    quitting: Arc<AtomicBool>,
    icon: Option<egui::TextureHandle>,
}

impl Drop for PairDialog {
    fn drop(&mut self) {
        self.session.cancel();
    }
}

/// Size of the bulb at the top of the window.
const ICON_SIZE: f32 = 56.0;
/// Height of the bar holding Cancel / Try again / Close.
const BUTTON_BAR: f32 = 44.0;
const GREEN: Color32 = Color32::from_rgb(0x3a, 0xa8, 0x5c);
const RED: Color32 = Color32::from_rgb(0xd9, 0x48, 0x48);

impl eframe::App for PairDialog {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.quitting.load(Ordering::SeqCst) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        let phase = self.session.phase();

        egui::Panel::bottom("buttons")
            .show_separator_line(false)
            .resizable(false)
            .exact_size(BUTTON_BAR)
            .show(ui, |ui| {
                let size = egui::vec2(ui.available_width(), BUTTON_BAR);
                ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
                    if phase.in_progress() {
                        if ui.button("Cancel").clicked() {
                            self.session.cancel();
                        }
                    } else {
                        if ui.button("Close").clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                        let retry = matches!(
                            phase,
                            Phase::NothingHeard | Phase::Cancelled | Phase::Failed(_)
                        );
                        if retry
                            && ui.button("Try again").clicked()
                            && let Some(session) = (self.restart)()
                        {
                            session.repaint_with(&ctx);
                            self.session = session;
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                if let Some(icon) = &self.icon {
                    let size = Vec2::splat(ICON_SIZE);
                    ui.add(egui::Image::new(egui::load::SizedTexture::new(
                        icon.id(),
                        size,
                    )));
                    ui.add_space(6.0);
                }
                body(ui, &phase, &self.session);
            });
        });

        // Keep the countdown moving
        if phase.in_progress() {
            ctx.request_repaint_after(Duration::from_millis(250));
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

fn body(ui: &mut egui::Ui, phase: &Phase, session: &Session) {
    let heading = |ui: &mut egui::Ui, text: &str, color: Option<Color32>| {
        let mut text = RichText::new(text).heading().strong();
        if let Some(c) = color {
            text = text.color(c);
        }
        ui.label(text);
    };
    match phase {
        Phase::Starting => {
            ui.add(egui::Spinner::new().size(28.0));
            ui.add_space(8.0);
            heading(ui, "Getting ready…", None);
            ui.label("Turning on Bluetooth scanning.");
        }
        Phase::Listening { heard, presses } => {
            heading(
                ui,
                if *presses == 0 {
                    "Press POWER twice"
                } else {
                    "Press POWER once more"
                },
                None,
            );
            ui.add_space(4.0);
            ui.label("on your light's remote, holding it near this PC.");
            ui.weak("The light turns off and back on.");
            ui.add_space(14.0);
            press_dots(ui, *presses);
            ui.add_space(10.0);
            let left = session.time_left().unwrap_or(PAIR_TIMEOUT).as_secs();
            let status = match heard {
                Some(id) => format!("Heard remote {id} · {left} s left"),
                None => format!("Listening… {left} s left"),
            };
            ui.horizontal(|ui| {
                // Centre the spinner and text together
                let width = 22.0
                    + ui.fonts_mut(|f| {
                        f.layout_no_wrap(
                            status.clone(),
                            egui::TextStyle::Body.resolve(ui.style()),
                            Color32::WHITE,
                        )
                        .size()
                        .x
                    });
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.add(egui::Spinner::new().size(14.0));
                ui.weak(status);
            });
        }
        Phase::Paired(id) => {
            heading(ui, "Paired", Some(GREEN));
            ui.add_space(4.0);
            ui.label(format!("This PC now controls the light as remote {id}."));
            ui.add_space(14.0);
            press_dots(ui, PAIR_PRESSES);
            ui.add_space(10.0);
            ui.weak("Use the tray icon to control it.");
        }
        Phase::NothingHeard => {
            heading(ui, "No remote heard", Some(RED));
            ui.add_space(6.0);
            ui.label(format!(
                "No remote pressed POWER twice in {} s.",
                PAIR_TIMEOUT.as_secs()
            ));
            ui.label("Try again, closer to the PC.");
        }
        Phase::Cancelled => {
            heading(ui, "Pairing cancelled", None);
            ui.add_space(6.0);
            ui.label("Nothing was changed.");
        }
        Phase::Failed(message) => {
            heading(ui, "Pairing failed", Some(RED));
            ui.add_space(6.0);
            ui.add(egui::Label::new(message.as_str()).wrap());
        }
    }
}

/// One dot per press needed, filled for each heard.
fn press_dots(ui: &mut egui::Ui, heard: usize) {
    const RADIUS: f32 = 7.0;
    const GAP: f32 = 14.0;
    #[allow(clippy::cast_precision_loss)]
    let width = PAIR_PRESSES as f32 * (RADIUS * 2.0) + (PAIR_PRESSES - 1) as f32 * GAP;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, RADIUS * 2.0), Sense::hover());
    let accent = ui.visuals().selection.bg_fill;
    let stroke = ui.visuals().widgets.inactive.fg_stroke.color;
    for i in 0..PAIR_PRESSES {
        #[allow(clippy::cast_precision_loss)]
        let x = rect.left() + RADIUS + i as f32 * (RADIUS * 2.0 + GAP);
        let centre = egui::pos2(x, rect.center().y);
        if i < heard {
            ui.painter().circle_filled(centre, RADIUS, accent);
        } else {
            ui.painter()
                .circle_stroke(centre, RADIUS - 1.0, egui::Stroke::new(1.5, stroke));
        }
    }
}
