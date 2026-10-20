#![forbid(unsafe_code)]
fn main() -> eframe::Result<()> { eframe::run_native("Booru Dataset Manager", eframe::NativeOptions::default(), Box::new(|_cc| Ok(Box::new(BdmApp)))) }
struct BdmApp;
impl eframe::App for BdmApp { fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) { egui::CentralPanel::default().show(ctx, |ui| { ui.heading("Booru Dataset Manager"); ui.label("Bootstrap UI only; ingestion, database, AI, export and backup workflows are not implemented yet."); }); } }
