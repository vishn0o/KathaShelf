#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod awards;
mod covers;
mod db;
mod models;
mod openlibrary;
mod reader;
mod scan;

use app::BookApp;

/// Register the reader fonts. Native egui apps load fonts through
/// `FontDefinitions` instead of CSS: Inter for UI text, Lora for the serif
/// brand/headings, and Manjari for Malayalam glyphs.
fn configure_fonts(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        FontData::from_static(include_bytes!("../assets/Inter.ttf")).into(),
    );
    fonts.font_data.insert(
        "Lora".to_owned(),
        FontData::from_static(include_bytes!("../assets/Lora.ttf")).into(),
    );
    fonts.font_data.insert(
        "Manjari".to_owned(),
        FontData::from_static(include_bytes!("../assets/Manjari-Regular.ttf")).into(),
    );

    fonts.families.insert(
        FontFamily::Proportional,
        vec![
            "Inter".to_owned(),
            "Manjari".to_owned(),
            "Ubuntu-Light".to_owned(),
            "egui-icons".to_owned(),
        ],
    );
    // `Theme::serif` asks for `FontFamily::Name("Lora")`. A named family must
    // be registered explicitly; otherwise headings silently fall back to the
    // proportional UI font.
    fonts.families.insert(
        FontFamily::Name("Lora".into()),
        vec![
            "Lora".to_owned(),
            "Manjari".to_owned(),
            "Inter".to_owned(),
            "egui-icons".to_owned(),
        ],
    );
    fonts.families.insert(
        FontFamily::Monospace,
        vec![
            "Hack".to_owned(),
            "Ubuntu-Light".to_owned(),
            "egui-icons".to_owned(),
        ],
    );
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Thalir — Vayana · Book Tracker")
            .with_inner_size([1020.0, 680.0])
            .with_min_inner_size([900.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "KathaShelf",
        options,
        Box::new(|cc| {
            configure_fonts(&cc.egui_ctx);
            match BookApp::new() {
                Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
                Err(e) => Err(format!("Failed to initialize KathaShelf: {e}").into()),
            }
        }),
    )
}
