pub mod cli;
pub mod config;
pub mod data;
pub mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = match cli::Cli::parse_args() {
        Ok(cli) => cli,
        Err(code) => return code,
    };
    match cli.gui_launch() {
        Ok(Some(launch)) => open_app(cli.data_dir_arg(), launch),
        Ok(None) => cli::run(cli),
        Err(err) => cli::fail(&err, cli.json),
    }
}

/// Opens the desktop app on the data folder given on the command line, or the usual default.
fn open_app(data_dir: Option<PathBuf>, launch: ui::Launch) -> ExitCode {
    let config = config::Config::load();
    let data_dir = config::resolve_data_dir(data_dir.as_deref(), &config);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SMU BOSS Bid Helper")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    let result = eframe::run_native(
        "SMU BOSS Bid Helper",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, data_dir, config, launch)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: could not open the window: {err}");
            ExitCode::FAILURE
        }
    }
}
