pub mod overview;
pub mod prediction;
pub mod ranking;
pub mod saved_searches;
pub mod sidebar;
pub mod theme;
pub mod widgets;

use crate::config::Config;
use crate::data::filter::{instructors_for_course, windows_for_course, GlobalFilters, WindowSel};
use crate::data::model::ModelParams;
use crate::data::{AppData, RoundKind, Term};
use egui::RichText;
use overview::OverviewState;
use prediction::PredictionState;
use ranking::RankingState;
use saved_searches::SavedState;
use serde::{Deserialize, Serialize};
use sidebar::Sidebar;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum Tab {
    Overview,
    Prediction,
    Ranking,
    SavedSearches,
}

/// What the user is looking at; shared by the Overview and Bid Planner tabs.
#[derive(Clone, Serialize, Deserialize)]
pub struct Selection {
    pub course: Option<String>,
    pub sel: WindowSel,
    pub instructor: Option<String>,
    #[serde(skip)]
    pub target: Option<Term>,
}

impl Default for Selection {
    fn default() -> Self {
        Self { course: None, sel: WindowSel::ROUND1_W1, instructor: None, target: None }
    }
}

impl Selection {
    pub fn select_course(&mut self, code: &str) {
        if self.course.as_deref() != Some(code) {
            self.course = Some(code.to_string());
            self.instructor = None;
        }
    }
}

/// Everything a tab needs to draw itself.
pub struct Ctx<'a> {
    pub data: &'a Arc<AppData>,
    pub filters: &'a GlobalFilters,
    pub params: &'a ModelParams,
    pub selection: &'a mut Selection,
    pub saved: &'a mut SavedState,
    pub tab: &'a mut Tab,
}

/// Returns the value cached in `slot` for `key`. `build` runs only when there is no cached value
/// or `key` differs from the one it was built for.
pub fn memo<K: PartialEq, V>(slot: &mut Option<(K, V)>, key: K, build: impl FnOnce(&K) -> V) -> &V {
    if slot.as_ref().is_none_or(|(k, _)| *k != key) {
        let value = build(&key);
        *slot = Some((key, value));
    }
    slot.as_ref().map(|(_, v)| v).expect("slot was filled above")
}

/// What a course offers in the data: the round/windows it was bid in and who taught it.
pub struct CourseOptions {
    pub windows: Vec<(RoundKind, u8)>,
    pub instructors: Vec<String>,
}

impl CourseOptions {
    pub fn new(data: &AppData, code: &str) -> Self {
        Self { windows: windows_for_course(data, code), instructors: instructors_for_course(data, code) }
    }
}

/// Optional command-line overrides: `--course IS111 --tab planner`.
#[derive(Default)]
pub struct Launch {
    pub course: Option<String>,
    pub tab: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Persisted {
    tab: Option<Tab>,
    filters: GlobalFilters,
    selection: Selection,
}

enum LoadState {
    Loading(Receiver<Result<AppData, String>>, Instant),
    Ready(Arc<AppData>),
    Failed(String),
}

pub struct App {
    load: LoadState,
    data_dir: PathBuf,
    config: Config,
    tab: Tab,
    filters: GlobalFilters,
    params: ModelParams,
    selection: Selection,
    sidebar: Sidebar,
    overview: OverviewState,
    prediction: PredictionState,
    ranking: RankingState,
    saved: SavedState,
    show_settings: bool,
    folder_input: String,
}

const STORAGE_KEY: &str = "bid_helper_state_v2";

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, data_dir: PathBuf, config: Config, launch: Launch) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut persisted: Persisted = cc.storage.and_then(|s| eframe::get_value(s, STORAGE_KEY)).unwrap_or_default();
        if let Some(c) = launch.course {
            persisted.selection.select_course(&c);
        }
        if let Some(t) = launch.tab {
            persisted.tab = match t.to_ascii_lowercase().as_str() {
                "planner" | "prediction" => Some(Tab::Prediction),
                "instructors" | "ranking" => Some(Tab::Ranking),
                "saved" => Some(Tab::SavedSearches),
                _ => Some(Tab::Overview),
            };
        }
        let mut app = Self {
            load: LoadState::Failed(String::new()),
            folder_input: data_dir.display().to_string(),
            data_dir,
            config,
            tab: persisted.tab.unwrap_or(Tab::Overview),
            filters: persisted.filters,
            params: ModelParams::default(),
            selection: persisted.selection,
            sidebar: Sidebar::default(),
            overview: OverviewState::default(),
            prediction: PredictionState::default(),
            ranking: RankingState::default(),
            saved: SavedState::load(),
            show_settings: false,
        };
        app.start_loading(&cc.egui_ctx);
        app
    }

    fn start_loading(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let dir = self.data_dir.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::data::loader::load_all_data(&dir);
            tx.send(result).ok();
            ctx.request_repaint();
        });
        self.load = LoadState::Loading(rx, Instant::now());
    }

    fn set_data_dir(&mut self, ctx: &egui::Context, dir: PathBuf) {
        self.folder_input = dir.display().to_string();
        self.data_dir = dir.clone();
        self.config.data_dir = Some(dir);
        self.config.save();
        self.start_loading(ctx);
    }

    fn poll_loading(&mut self) {
        if let LoadState::Loading(rx, _) = &self.load {
            if let Ok(result) = rx.try_recv() {
                self.load = match result {
                    Ok(data) => {
                        let data = Arc::new(data);
                        if let Some(c) = &self.selection.course {
                            if data.course(c).is_none() {
                                self.selection.course = None;
                            }
                        }
                        self.sidebar.reset();
                        self.overview = OverviewState::default();
                        // Cache keys hold the data's address, which a new data set can reuse.
                        self.prediction.clear_cache();
                        self.ranking = RankingState::default();
                        LoadState::Ready(data)
                    }
                    Err(e) => LoadState::Failed(e),
                };
            }
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("BOSS Bid Helper").size(18.0).strong().color(theme::ACCENT));
                    ui.add_space(16.0);
                    for (tab, label) in [
                        (Tab::Overview, "📊  Overview"),
                        (Tab::Prediction, "🎯  Bid Planner"),
                        (Tab::Ranking, "👥  Instructors"),
                        (Tab::SavedSearches, "★  Saved"),
                    ] {
                        let selected = self.tab == tab;
                        let text = if selected { RichText::new(label).strong() } else { RichText::new(label) };
                        if ui.selectable_label(selected, text).clicked() {
                            self.tab = tab;
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚙ Data folder").clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        egui::widgets::global_theme_preference_switch(ui);
                        if let LoadState::Ready(data) = &self.load {
                            let excluded = self.filters.excluded_sessions.len();
                            let label = if excluded == 0 { "Sessions: all".to_string() } else { format!("Sessions: {} hidden", excluded) };
                            ui.menu_button(label, |ui| {
                                ui.label(RichText::new("Include rows from these sessions").small());
                                for s in &data.sessions {
                                    let mut on = !self.filters.excluded_sessions.contains(&**s);
                                    if ui.checkbox(&mut on, &**s).changed() {
                                        if on {
                                            self.filters.excluded_sessions.remove(&**s);
                                        } else {
                                            self.filters.excluded_sessions.insert(s.to_string());
                                        }
                                    }
                                }
                            });
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("📁 {}", self.data_dir.display())).small());
                    if let LoadState::Ready(data) = &self.load {
                        let r = &data.report;
                        let terms = data.terms.first().zip(data.terms.last()).map(|(a, b)| format!("{} – {}", a, b)).unwrap_or_default();
                        ui.separator();
                        ui.label(RichText::new(format!(
                            "{} files · {} rows · {} courses · {} · loaded in {} ms",
                            r.files.len(),
                            data.rows.len(),
                            data.courses.len(),
                            terms,
                            r.millis
                        ))
                        .small());
                        let errors: Vec<_> = r.errors().collect();
                        if !errors.is_empty() {
                            ui.separator();
                            let text = errors.iter().map(|f| format!("{}: {}", f.name, f.error.as_deref().unwrap_or(""))).collect::<Vec<_>>().join("\n");
                            ui.label(RichText::new(format!("⚠ {} file(s) skipped", errors.len())).small().color(theme::WARN)).on_hover_text(text);
                        }
                    }
                });
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut new_dir = None;
        let mut reload = false;
        egui::Window::new("Data folder").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.label("Folder containing the BOSS Overall Results exports (.xls / .xlsx), one file per term.");
            ui.label(RichText::new("Files are cleaned automatically on load: repeated headers, debris rows, freshmen/exchange rounds and duplicates are dropped.").small().color(theme::muted(ui)));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.folder_input).desired_width(460.0));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_directory(&self.data_dir).pick_folder() {
                        new_dir = Some(p);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Use this folder").clicked() {
                    new_dir = Some(PathBuf::from(self.folder_input.trim()));
                }
                if ui.button("Reload").clicked() {
                    reload = true;
                }
                if ui.button("Reset to default").clicked() {
                    new_dir = Some(crate::config::default_data_dir());
                }
            });
            if let LoadState::Ready(data) = &self.load {
                ui.separator();
                egui::Grid::new("file_report").striped(true).show(ui, |ui| {
                    for h in ["File", "Term", "Rows", "Freshmen/Exchange", "Dropped", "Status"] {
                        ui.label(RichText::new(h).small().strong());
                    }
                    ui.end_row();
                    for f in &data.report.files {
                        ui.label(&f.name);
                        ui.label(f.term.map(|t| t.label()).unwrap_or_default());
                        ui.label(f.rows_kept.to_string());
                        ui.label(f.rows_special_rounds.to_string());
                        ui.label((f.rows_invalid + f.duplicates).to_string());
                        match &f.error {
                            Some(e) => ui.label(RichText::new(e).color(theme::BAD)),
                            None => ui.label(RichText::new("OK").color(theme::GOOD)),
                        };
                        ui.end_row();
                    }
                });
            }
        });
        self.show_settings = open;
        if let Some(d) = new_dir {
            self.set_data_dir(ctx, d);
        } else if reload {
            self.start_loading(ctx);
        }
    }

    fn not_ready_screen(&mut self, ctx: &egui::Context) {
        let mut retry = false;
        let mut browse = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                match &self.load {
                    LoadState::Loading(_, started) => {
                        ui.spinner();
                        ui.label(format!("Reading bid data from\n{}", self.data_dir.display()));
                        ui.label(RichText::new(format!("{:.1}s", started.elapsed().as_secs_f32())).small());
                    }
                    LoadState::Failed(msg) => {
                        ui.label(RichText::new("Could not load bid data").size(20.0).color(theme::BAD));
                        ui.label(msg);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                            browse = ui.button("📁 Choose folder…").clicked();
                            retry = ui.button("Retry").clicked();
                        });
                    }
                    LoadState::Ready(_) => {}
                }
            });
        });
        if browse {
            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                self.set_data_dir(ctx, p);
            }
        } else if retry {
            self.start_loading(ctx);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_loading();
        self.top_bar(ctx);
        self.status_bar(ctx);
        if self.show_settings {
            self.settings_window(ctx);
        }

        let data = match &self.load {
            LoadState::Ready(d) => d.clone(),
            LoadState::Loading(..) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return self.not_ready_screen(ctx);
            }
            LoadState::Failed(_) => return self.not_ready_screen(ctx),
        };

        if self.tab != Tab::SavedSearches {
            egui::SidePanel::left("course_sidebar")
                .resizable(true)
                .default_width(300.0)
                .width_range(240.0..=460.0)
                .show(ctx, |ui| {
                    if let Some(code) = self.sidebar.ui(ui, &data, self.selection.course.as_deref()) {
                        if let Some(c) = data.course(&code) {
                            self.saved.auto_save_click(&code, &c.description);
                        }
                        self.selection.select_course(&code);
                    }
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut cx = Ctx {
                data: &data,
                filters: &self.filters,
                params: &self.params,
                selection: &mut self.selection,
                saved: &mut self.saved,
                tab: &mut self.tab,
            };
            match *cx.tab {
                Tab::Overview => self.overview.ui(ui, &mut cx),
                Tab::Prediction => self.prediction.ui(ui, &mut cx),
                Tab::Ranking => self.ranking.ui(ui, &mut cx),
                Tab::SavedSearches => self.saved_ui(ui, &data),
            }
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let p = Persisted { tab: Some(self.tab), filters: self.filters.clone(), selection: self.selection.clone() };
        eframe::set_value(storage, STORAGE_KEY, &p);
    }
}

impl App {
    fn saved_ui(&mut self, ui: &mut egui::Ui, data: &Arc<AppData>) {
        if let Some(entry) = self.saved.ui(ui) {
            if data.course(&entry.course_code).is_some() {
                self.selection.select_course(&entry.course_code);
                if let (Some(r), Some(w)) = (entry.round, entry.window) {
                    self.selection.sel = WindowSel { round: Some(r), window: Some(w) };
                }
                self.selection.instructor = entry.instructor.clone();
                self.tab = if entry.has_prediction() { Tab::Prediction } else { Tab::Overview };
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_data {
    use crate::data::{AppData, BidRow, LoadReport, RoundKind, Term, TermKind};
    use std::collections::HashMap;
    use std::sync::Arc;

    const REGULAR: &str = "Regular Academic Session";
    const FRESHMAN: &str = "Freshman Session";

    /// One course (TEST101) over seven terms: four sections in window 1, one in window 2.
    pub fn sample_data() -> AppData {
        let kinds = [(2022, TermKind::T1), (2022, TermKind::T2), (2023, TermKind::T1), (2023, TermKind::T2), (2024, TermKind::T1), (2024, TermKind::T2), (2025, TermKind::T1)];
        let mut rows = Vec::new();
        for (i, (year, kind)) in kinds.into_iter().enumerate() {
            let term = Term { year, kind };
            let base = 20.0 + 3.0 * i as f64;
            // (section, instructor, (seats before, seats after), min bid, session)
            let sections = [
                ("G1", "ALPHA", (10, 0), base, REGULAR),
                ("G2", "BETA", (10, 0), base * 1.4, REGULAR),
                ("G3", "ALPHA", (12, 4), 12.0, REGULAR),
                ("G4", "GAMMA", (6, 0), base * 2.0, FRESHMAN),
            ];
            for (section, instructor, seats, min_bid, session) in sections {
                rows.push(row(term, 1, session, section, instructor, seats, min_bid));
            }
            rows.push(row(term, 2, REGULAR, "G1", "ALPHA", (8, 0), base * 0.9));
        }
        AppData::new(rows, HashMap::new(), LoadReport::default())
    }

    fn row(term: Term, window: u8, session: &str, section: &str, instructor: &str, (before, after): (i32, i32), min_bid: f64) -> BidRow {
        BidRow {
            term,
            session: Arc::from(session),
            round: RoundKind::Round1,
            window,
            course_code: Arc::from("TEST101"),
            section: Arc::from(section),
            instructor: Arc::from(instructor),
            school: Arc::from("SCIS"),
            vacancy: 40,
            opening_vacancy: 10,
            before_proc: before,
            dice: 0,
            after_proc: after,
            enrolled: before - after,
            median_bid: min_bid * 1.2,
            min_bid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::memo;

    #[test]
    fn memo_builds_again_only_when_the_key_changes() {
        let mut slot = None;
        let mut builds = 0;
        for key in [1, 1, 2, 2, 1] {
            let v = *memo(&mut slot, key, |k| {
                builds += 1;
                k * 10
            });
            assert_eq!(v, key * 10);
        }
        assert_eq!(builds, 3);
    }
}
