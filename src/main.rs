mod app;
mod config;
mod fonts;
mod terminal;
mod theme;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([980.0, 640.0])
            .with_title("Peak Terminal"),
        ..Default::default()
    };

    eframe::run_native(
        "Peak Terminal",
        native_options,
        Box::new(|cc| Ok(Box::new(app::PeakTerminalApp::new(cc)))),
    )
}
