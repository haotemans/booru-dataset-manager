#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_profiles;
mod app;
mod catalog;
mod collections;
mod dialogs;
mod gallery;
mod image_editor;
mod ingest;
mod media_review;
mod model;
mod pages;
mod paths;
mod redownload;
mod review;
mod review_ui;
mod similarity_review;
mod sources;
mod tags;
mod theme;
mod thumbnails;
mod widgets;
mod workflows;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let capture_path = args
        .iter()
        .position(|arg| arg == "--capture")
        .and_then(|i| args.get(i + 1));
    let capture = capture_path.is_some();
    let compact = std::env::args().any(|arg| arg == "--compact");
    eframe::run_native(
        "Booru Dataset Manager",
        eframe::NativeOptions {
            persistence_path: Some(
                capture_path
                    .map(|path| std::path::Path::new(path).join(".state"))
                    .unwrap_or_else(|| paths::data_directory().join("app.ron")),
            ),
            persist_window: !capture && !compact,
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(if compact {
                    [1050.0, 720.0]
                } else {
                    [1440.0, 960.0]
                })
                .with_min_inner_size([1050.0, 720.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::BdmApp::new(cc)))),
    )
}
