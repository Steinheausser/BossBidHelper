pub mod overview;
pub mod prediction;
pub mod ranking;
pub mod saved_searches;
pub mod widgets;

use std::sync::Arc;
use crate::data::AppData;
use overview::OverviewState;
use prediction::PredictionState;
use ranking::RankingState;
use saved_searches::SavedState;

#[derive(PartialEq)]
pub enum Tab {
    Overview,
    Prediction,
    Ranking,
    SavedSearches,
}

pub struct App {
    pub data: Arc<AppData>,
    pub active_tab: Tab,
    pub overview: OverviewState,
    pub prediction: PredictionState,
    pub ranking: RankingState,
    pub saved: SavedState,
    pub include_1a: bool,
    pub include_1b: bool,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, data: Arc<AppData>) -> Self {
        Self {
            data: data.clone(),
            active_tab: Tab::Overview,
            overview: OverviewState::new(data.clone()),
            prediction: PredictionState::new(),
            ranking: RankingState::new(data.clone()),
            saved: SavedState::load(),
            include_1a: true,
            include_1b: true,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Only repaint on active UI interaction, preventing flickering/high CPU
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Overview, "Overview");
                ui.selectable_value(&mut self.active_tab, Tab::Prediction, "Price Prediction");
                ui.selectable_value(&mut self.active_tab, Tab::Ranking, "Professor Ranking");
                ui.selectable_value(&mut self.active_tab, Tab::SavedSearches, "Saved Searches");
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.include_1a, "Include 1A Session");
                ui.checkbox(&mut self.include_1b, "Include 1B Session");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_tab {
                Tab::Overview => self.overview.ui(ui, &self.data, &mut self.saved, self.include_1a, self.include_1b, &mut self.active_tab, &mut self.prediction),
                Tab::Prediction => self.prediction.ui(ui, &self.data, &mut self.saved, self.include_1a, self.include_1b),
                Tab::Ranking => self.ranking.ui(ui, &self.data, self.include_1a, self.include_1b),
                Tab::SavedSearches => self.saved.ui(ui, &mut self.prediction, &mut self.active_tab),
            }
        });
    }
}
