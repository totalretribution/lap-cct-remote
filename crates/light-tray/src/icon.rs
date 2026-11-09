//! The app icon: a bulb, half warm and half cool. The PNGs are rendered from `assets/light-remote.svg`;
//! the SVG is also installed into the icon theme as `light-remote`, for the menu entry and notifications.

use eframe::egui::IconData;

/// Icon theme name (installed by install.sh).
pub const NAME: &str = "light-remote";

const TRAY_SIZES: [&[u8]; 4] = [
    include_bytes!("../assets/light-remote-22.png"),
    include_bytes!("../assets/light-remote-32.png"),
    include_bytes!("../assets/light-remote-48.png"),
    include_bytes!("../assets/light-remote-64.png"),
];
const WINDOW: &[u8] = include_bytes!("../assets/light-remote-128.png");

/// The icon in several sizes, as the tray wants them (ARGB32, big-endian). Built in, so it shows
/// whether or not the icon theme has it.
pub fn tray_pixmaps() -> Vec<ksni::Icon> {
    TRAY_SIZES
        .iter()
        .filter_map(|png| eframe::icon_data::from_png_bytes(png).ok())
        .map(|icon| ksni::Icon {
            width: i32::try_from(icon.width).unwrap_or(0),
            height: i32::try_from(icon.height).unwrap_or(0),
            data: icon
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, a]| [a, r, g, b])
                .collect(),
        })
        .collect()
}

/// The pairing window's icon.
pub fn window_icon() -> Option<IconData> {
    eframe::icon_data::from_png_bytes(WINDOW).ok()
}
