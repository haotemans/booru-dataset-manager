#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_profiles;
mod app;
mod catalog;
mod collections;
mod diagnostics;
mod dialogs;
mod gallery;
mod gelbooru_settings;
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
    if let Some(index) = args.iter().position(|arg| arg == "--repair-crops-png") {
        let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
            let backup =
                std::path::PathBuf::from(args.get(index + 1).ok_or("missing backup directory")?);
            let catalog = args
                .iter()
                .position(|arg| arg == "--catalog")
                .and_then(|index| args.get(index + 1))
                .ok_or("missing --catalog")?;
            if !backup.is_absolute() || !std::path::Path::new(catalog).is_absolute() {
                return Err("absolute paths required".into());
            }
            let apply = args.iter().any(|arg| arg == "--apply");
            let mut repo =
                bdm_infrastructure::catalog::SqliteCatalog::open(std::path::Path::new(catalog))?;
            let report = repo.repair_jpeg_crops_as_png(&backup, apply)?;
            std::fs::write(
                backup.join(if apply { "applied.json" } else { "audit.json" }),
                serde_json::to_vec_pretty(&report)?,
            )?;
            Ok(())
        })();
        if let Err(error) = &outcome {
            eprintln!("PNG remediation failed: {error}");
        }
        std::process::exit(if outcome.is_ok() { 0 } else { 1 });
    }
    let capture_path = args
        .iter()
        .position(|arg| arg == "--capture")
        .and_then(|i| args.get(i + 1));
    let capture = capture_path.is_some();
    let log_directory = capture_path
        .map(|path| std::path::Path::new(path).join("logs"))
        .unwrap_or_else(|| paths::data_directory().join("logs"));
    if let Err(error) = diagnostics::install(&log_directory) {
        eprintln!(
            "Unable to initialize diagnostics at {}: {error}",
            log_directory.display()
        );
    }
    let compact = std::env::args().any(|arg| arg == "--compact");
    diagnostics::event(format!(
        "analysis_python program={:?}",
        bdm_infrastructure::visual::python_command().get_program()
    ));
    let result = eframe::run_native(
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
    );
    match &result {
        Ok(()) => diagnostics::event("shutdown result=ok"),
        Err(error) => diagnostics::event(format!("shutdown result=error error={error}")),
    }
    result
}
