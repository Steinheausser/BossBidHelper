//! The original `--backtest` and `--tune` flags, which predate the subcommands. They print the same
//! report as before, so existing scripts keep working.

use super::{output, Cli, CliError};
use crate::config::{self, Config};
use crate::data::filter::WindowSel;
use crate::data::model::ModelParams;
use crate::data::{self, AppData};

/// Runs `--tune` (grid search) or `--backtest` (accuracy on past terms). Called only with one of them set.
pub fn run(cli: &Cli) -> Result<(), CliError> {
    let config = Config::load();
    let folder = cli.data_dir_arg();
    let dir = config::resolve_data_dir(folder.as_deref(), &config);

    eprintln!("Loading {} ...", output::ascii(&dir.display().to_string()));
    let data = data::loader::load_all_data(&dir).map_err(CliError::data)?;
    for f in &data.report.files {
        println!(
            "  {:<22} kept {:>6}  special-round {:>5}  invalid {:>3}  dup {:>3}  {}",
            output::ascii(&f.name),
            f.rows_kept,
            f.rows_special_rounds,
            f.rows_invalid,
            f.duplicates,
            output::ascii(f.error.as_deref().unwrap_or(""))
        );
    }
    println!(
        "{} rows, {} courses in {} ms\n",
        data.rows.len(),
        data.courses.len(),
        data.report.millis
    );

    let params = ModelParams::default();
    if cli.tune {
        tune_params(&data, params);
        return Ok(());
    }
    for scope in [Some(WindowSel::ROUND1_W1), None] {
        println!("{}", output::ascii(&data::backtest::run(&data, scope, &params).to_text()));
    }
    Ok(())
}

/// Grid search over model parameters, scored by mean pinball loss across all windows.
pub fn tune_params(data: &AppData, base: ModelParams) {
    let mut results = Vec::new();
    for half_life_years in [0.75, 1.5, 3.0] {
        for other_instructor_weight in [0.1, 0.3, 0.6] {
            for trend_damping in [0.0, 0.5, 1.0] {
                for min_bandwidth in [0.06, 0.12, 0.25] {
                    for fill_prior_strength in [0.5, 2.0, 5.0] {
                        let p = ModelParams {
                            half_life_years,
                            other_instructor_weight,
                            trend_damping,
                            min_bandwidth,
                            fill_prior_strength,
                            ..base
                        };
                        let report = data::backtest::run(data, None, &p);
                        let levels = &report.methods[0].levels;
                        let n = levels.len() as f64;
                        let loss = levels.iter().map(|l| l.mean_pinball()).sum::<f64>() / n;
                        let calib = levels.iter().map(|l| (l.hit_rate() - l.level).abs()).sum::<f64>() / n;
                        println!("loss {loss:.4} calib {calib:.4} {p:?}");
                        results.push((loss, calib, p));
                    }
                }
            }
        }
    }
    results.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("\nBest:");
    for (loss, calib, p) in results.iter().take(10) {
        println!("loss {loss:.4} calib {calib:.4} {p:?}");
    }
}
