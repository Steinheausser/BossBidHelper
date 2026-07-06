pub mod data;
pub mod ui;

use eframe::egui;
use std::sync::Arc;
use std::path::Path;

fn main() -> Result<(), eframe::Error> {
    let data_dir = Path::new("data");
    println!("Loading data from {:?}...", data_dir);
    let app_data = match data::loader::load_all_data(data_dir) {
        Ok(d) => {
            println!("Loaded {} rows successfully.", d.rows.len());
            d
        },
        Err(e) => {
            eprintln!("Failed to load data: {}", e);
            std::process::exit(1);
        }
    };
    
    let arc_data = Arc::new(app_data);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "SMU Bid Analyser",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc, arc_data)))),
    )
}
