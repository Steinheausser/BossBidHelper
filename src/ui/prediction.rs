use crate::data::backtest::BacktestReport;
use crate::data::filter::{course_rows, GlobalFilters, WindowSel};
use crate::data::model::{forecast, Confidence, Forecast, ForecastQuery, ModelParams, MIN_BID};
use crate::data::query::{plan_bid, LevelBid, DEFAULT_LEVELS};
use crate::data::stats::term_summaries;
use crate::data::{AppData, BidRow, Term, TermKind};
use crate::ui::widgets::{confidence_badge, money, normalise_selection, pct, section_heading, stat_card, term_plot, window_selector};
use crate::ui::{memo, theme, CourseOptions, Ctx};
use egui::{RichText, Ui};
use egui_plot::{Corner, HLine, Legend, Line, LineStyle, MarkerShape, Plot, PlotPoints, Points, VLine};
use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub const LEVELS: [f64; 5] = DEFAULT_LEVELS;

pub struct PredictionState {
    my_bid: f64,
    backtest: Option<BacktestReport>,
    backtest_rx: Option<Receiver<BacktestReport>>,
    options: Option<((usize, String), CourseOptions)>,
    plan: Option<(PlanKey, Plan)>,
}

impl Default for PredictionState {
    fn default() -> Self {
        Self { my_bid: 20.0, backtest: None, backtest_rx: None, options: None, plan: None }
    }
}

impl PredictionState {
    /// Drops the cached course data, e.g. after the data set is reloaded.
    pub fn clear_cache(&mut self) {
        self.options = None;
        self.plan = None;
    }

    pub fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if let Some(rx) = &self.backtest_rx {
            if let Ok(r) = rx.try_recv() {
                self.backtest = Some(r);
                self.backtest_rx = None;
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
        }

        let Some(code) = cx.selection.course.clone() else {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.heading("Bid Planner");
                ui.label("Select a course on the left to estimate how much to bid.");
            });
            return;
        };
        egui::ScrollArea::vertical().auto_shrink([true, false]).show(ui, |ui| self.planner(ui, cx, &code));
    }

    fn planner(&mut self, ui: &mut Ui, cx: &mut Ctx, code: &str) {
        let data = cx.data;
        let Some(info) = data.course(code) else { return };

        ui.horizontal(|ui| {
            ui.label(RichText::new(code).size(24.0).strong().color(theme::ACCENT));
            ui.label(RichText::new(&info.description).size(18.0));
        });

        // Controls.
        let opts = memo(&mut self.options, (Arc::as_ptr(data) as usize, code.to_string()), |(_, c)| CourseOptions::new(data, c));
        normalise_selection(&opts.windows, &mut cx.selection.sel, false);
        if cx.selection.instructor.as_ref().is_some_and(|i| !opts.instructors.contains(i)) {
            cx.selection.instructor = None;
        }
        let default_target = data.default_target_term();
        let target = *cx.selection.target.get_or_insert(default_target);
        let target_options = target_terms(default_target);

        theme::card(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                window_selector(ui, "planner", &opts.windows, &mut cx.selection.sel, false);
                ui.separator();
                ui.label("Instructor");
                egui::ComboBox::from_id_salt("planner_instructor")
                    .selected_text(cx.selection.instructor.as_deref().unwrap_or("Any section"))
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cx.selection.instructor, None, "Any section");
                        for i in &opts.instructors {
                            ui.selectable_value(&mut cx.selection.instructor, Some(i.clone()), i);
                        }
                    });
                ui.separator();
                ui.label("Bidding for");
                egui::ComboBox::from_id_salt("planner_target")
                    .selected_text(target.label())
                    .show_ui(ui, |ui| {
                        for t in target_options {
                            ui.selectable_value(&mut cx.selection.target, Some(t), t.label());
                        }
                    });
            });
        });
        let target = cx.selection.target.unwrap_or(default_target);

        let key = PlanKey::new(data, cx.filters, code, cx.selection.sel, cx.selection.instructor.as_deref(), target);
        let plan = memo(&mut self.plan, key, |k| Plan::new(data, cx.filters, cx.params, k));
        let Some(d) = plan.drawn.as_ref() else {
            // The warnings say why there is no forecast, as the CLI's do. The message below is only for a plan without any.
            ui.add_space(12.0);
            if plan.warnings.is_empty() {
                ui.label(RichText::new("No past sections had open seats in this round/window, so there is nothing to base a bid on.").color(theme::WARN));
            }
            warning_lines(ui, &plan.warnings);
            return;
        };
        let f = &d.fc;

        // Recommended bids, rounded up to the cent exactly as the CLI reports them.
        section_heading(ui, &format!("Recommended bids — {} · {}", cx.selection.sel.label(), target));
        ui.horizontal_wrapped(|ui| {
            for (level, rec) in LEVELS.iter().zip(&d.bids) {
                let color = if *level == 0.8 { Some(theme::ACCENT) } else { None };
                let sub = if rec.is_minimum_bid { "minimum bid".to_string() } else { String::new() };
                stat_card(ui, &format!("{} chance", pct(*level)), money(rec.bid), sub, color);
            }
        });
        ui.horizontal_wrapped(|ui| {
            confidence_badge(ui, plan.confidence);
            ui.label(
                RichText::new(format!(
                    "Based on {} past section-results over {} terms{}.",
                    f.observations.len(),
                    f.n_terms,
                    cx.selection.instructor.as_ref().map(|_| format!(" ({} with this instructor)", f.n_instructor_obs)).unwrap_or_default()
                ))
                .color(theme::muted(ui)),
            );
        });
        warning_lines(ui, &plan.warnings);

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            stat_card(ui, "Chance the section fills", pct(f.fill_prob), "if not, any bid ≥ e$10 wins", None);
            stat_card(ui, "Seats open in this window", pct(f.availability), "of past sections", None);
            let trend_color = if f.trend_per_year > 0.05 { Some(theme::BAD) } else if f.trend_per_year < -0.05 { Some(theme::GOOD) } else { None };
            stat_card(
                ui,
                "Price trend",
                format!("{:+.0}% / year", f.trend_per_year * 100.0),
                if f.trend_applied { "applied to forecast" } else { "for info; not extrapolated" },
                trend_color,
            );
            if let Some(med) = d.median {
                stat_card(ui, "Typical clearing price", money(med), "when it fills", None);
            }
        });

        // Try-a-bid: only the slider and this one probability are computed per frame.
        section_heading(ui, "Try a bid");
        theme::card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = 380.0;
                ui.label("My bid");
                let hi = d.max_bid.max(self.my_bid);
                ui.add(egui::Slider::new(&mut self.my_bid, MIN_BID..=hi).prefix("e$").fixed_decimals(2).clamping(egui::SliderClamping::Never));
                let p = f.success_prob(self.my_bid);
                let color = if p >= 0.8 { theme::GOOD } else if p >= 0.5 { theme::WARN } else { theme::BAD };
                ui.add_space(16.0);
                ui.label(RichText::new(format!("{} chance of a seat", pct(p))).size(20.0).strong().color(color));
            });
        });

        success_curve(ui, d, self.my_bid);
        history_chart(ui, &d.history, f.target, cx.selection.instructor.is_some());

        ui.horizontal(|ui| {
            if ui.button("★ Save this plan").clicked() {
                let bids: Vec<f64> = d.bids.iter().map(|b| b.bid).collect();
                cx.saved.add_prediction(code, &info.description, cx.selection.sel, cx.selection.instructor.clone(), target, &bids, plan.confidence);
            }
        });

        egui::CollapsingHeader::new("Past sections used (weights)").show(ui, |ui| observations_table(ui, f));
        egui::CollapsingHeader::new("How is this calculated?").show(ui, |ui| self.explainer(ui, cx));
    }

    fn explainer(&mut self, ui: &mut Ui, cx: &Ctx) {
        ui.label("For each past section in the same round and window:");
        ui.label("• If it filled, the Min Bid is the clearing price: bids at or above it got a seat.");
        ui.label("• If it did not fill, any bid of at least e$10 got a seat, whatever its Min Bid says.");
        ui.label("So P(seat | bid) = P(no fill) + P(fill) × P(clearing price ≤ bid).");
        ui.add_space(4.0);
        ui.label("Recent terms count more (1.5-year half-life), the same term type (T1/T2) counts more, and if you pick an instructor their sections count ~3× more than other sections of the course, so thin histories borrow from the course as a whole. Sections in the same term are down-weighted together because they share that term's demand.");
        ui.label("The fill chance is pulled toward the fill rate of this window across all courses, and the price distribution is smoothed (kernel density) with more spread when there is less data.");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let running = self.backtest_rx.is_some();
            if ui.add_enabled(!running, egui::Button::new("Run backtest on all courses")).clicked() {
                let (tx, rx) = std::sync::mpsc::channel();
                let data = cx.data.clone();
                let params = *cx.params;
                let sel = cx.selection.sel;
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let scope = if sel.is_specific() { Some(sel) } else { Some(WindowSel::ROUND1_W1) };
                    tx.send(crate::data::backtest::run(&data, scope, &params)).ok();
                    ctx.request_repaint();
                });
                self.backtest_rx = Some(rx);
            }
            if running {
                ui.spinner();
            }
        });
        ui.label(
            RichText::new("Replays past terms: predicts each section using only earlier data, then checks whether the recommended bid would have won. \"won seat\" should be close to the target; lower avg bid at the same rate is better; loss is the quantile (pinball) loss.")
                .small()
                .color(theme::muted(ui)),
        );
        if let Some(r) = &self.backtest {
            ui.add(egui::Label::new(RichText::new(r.to_text()).monospace()).wrap_mode(egui::TextWrapMode::Extend));
        }
    }
}

/// Identifies the planner's derived data. Any change to a field rebuilds it. The "My bid" slider
/// is not an input: it only moves one marker and one `success_prob` call. Model parameters are
/// fixed for the session (set once in `App::new`), so they are not part of the key.
#[derive(Debug, PartialEq)]
struct PlanKey {
    /// Address of the loaded data set; a reload gives a new one.
    data: usize,
    code: String,
    sel: WindowSel,
    instructor: Option<String>,
    target: Term,
    filters: GlobalFilters,
}

impl PlanKey {
    fn new(data: &Arc<AppData>, filters: &GlobalFilters, code: &str, sel: WindowSel, instructor: Option<&str>, target: Term) -> Self {
        Self {
            data: Arc::as_ptr(data) as usize,
            code: code.to_string(),
            sel,
            instructor: instructor.map(str::to_string),
            target,
            filters: filters.clone(),
        }
    }
}

/// Everything the planner shows for one `PlanKey`.
struct Plan {
    /// Things to know before trusting the numbers, from `query::plan_bid`, so the GUI says what the CLI says.
    /// Shown under the recommended bids, or on their own when there is no forecast.
    warnings: Vec<String>,
    /// The confidence `plan_bid` reports: Low whenever no past section filled.
    confidence: Confidence,
    /// `None` when no past section had seats open in this round and window, so there is nothing to draw.
    drawn: Option<Drawn>,
}

/// The forecast, and what the planner draws from it.
struct Drawn {
    fc: Forecast,
    /// Recommended bid for each of `LEVELS`, from `query::plan_bid`, so they match the CLI to the cent.
    bids: Vec<LevelBid>,
    /// Median clearing price, when any past section filled.
    median: Option<f64>,
    /// Right-hand end of the success curve.
    max_bid: f64,
    /// (bid, chance in %) from the minimum bid to `max_bid`.
    curve: Vec<[f64; 2]>,
    /// (recommended bid, chance in %) for each of `bids`.
    marks: Vec<[f64; 2]>,
    history: History,
}

impl Plan {
    /// The warnings, confidence and bids come from `query::plan_bid`, so they match the CLI. The chart also
    /// needs the forecast itself, which exists only when some past section had seats open.
    fn new(data: &AppData, filters: &GlobalFilters, params: &ModelParams, key: &PlanKey) -> Plan {
        let q = ForecastQuery { course: key.code.as_str(), sel: key.sel, instructor: key.instructor.as_deref(), target: key.target };
        let (warnings, confidence, bids) = match plan_bid(data, filters, params, &q, &LEVELS, &[]) {
            Some(p) => (p.warnings, confidence_from_label(&p.confidence), p.recommended),
            None => (Vec::new(), Confidence::Low, Vec::new()),
        };
        let drawn = forecast(data, filters, &q, params).map(|fc| {
            let max_bid = fc.chart_max_bid();
            let steps = 160;
            let curve = (0..=steps)
                .map(|i| {
                    let b = MIN_BID + (max_bid - MIN_BID) * i as f64 / steps as f64;
                    [b, fc.success_prob(b) * 100.0]
                })
                .collect();
            let marks = bids.iter().map(|b| [b.bid, fc.success_prob(b.bid) * 100.0]).collect();
            let history = History::new(course_rows(data, &key.code, key.sel, filters), &fc, key.instructor.as_deref());
            Drawn { median: fc.median_clearing(), fc, bids, max_bid, curve, marks, history }
        });
        Plan { warnings, confidence, drawn }
    }
}

/// The confidence a `BidPlan` label names. The labels come from `Confidence::label`, so anything else
/// is taken as the cautious Low.
fn confidence_from_label(label: &str) -> Confidence {
    [Confidence::Low, Confidence::Medium, Confidence::High].into_iter().find(|c| c.label() == label).unwrap_or(Confidence::Low)
}

/// Each warning on its own line, in the warning colour.
fn warning_lines(ui: &mut Ui, warnings: &[String]) {
    for w in warnings {
        ui.label(RichText::new(format!("⚠ {w}")).color(theme::WARN));
    }
}

/// Points for "Clearing prices and forecast". Each term is one x position.
struct History {
    terms: Vec<Term>,
    /// Filled sections taught by the chosen instructor.
    mine: Vec<[f64; 2]>,
    /// Filled sections taught by anyone else (all filled sections when no instructor is chosen).
    others: Vec<[f64; 2]>,
    /// Median clearing price per term.
    line: Vec<[f64; 2]>,
    /// Forecast 10, 25, 50, 75 and 90% clearing prices; `None` when no past section filled.
    quantiles: Option<[f64; 5]>,
}

impl History {
    fn new(rows: Vec<&BidRow>, fc: &Forecast, chosen: Option<&str>) -> Self {
        let rows: Vec<_> = rows.into_iter().filter(|r| r.term < fc.target).collect();
        let summaries = term_summaries(&rows);
        let terms: Vec<Term> = summaries.iter().map(|s| s.term).collect();
        let index: BTreeMap<Term, f64> = terms.iter().enumerate().map(|(i, t)| (*t, i as f64)).collect();
        let (mut mine, mut others) = (Vec::new(), Vec::new());
        for r in rows.iter().filter(|r| r.filled()) {
            let p = [index[&r.term], r.min_bid.max(MIN_BID)];
            if chosen.is_some_and(|c| c == &*r.instructor) { mine.push(p) } else { others.push(p) }
        }
        let line = summaries.iter().enumerate().filter_map(|(i, s)| s.clearing_median.map(|c| [i as f64, c])).collect();
        let quantiles = fc.observations.iter().any(|o| o.filled).then(|| [0.1, 0.25, 0.5, 0.75, 0.9].map(|q| fc.price_quantile(q)));
        Self { terms, mine, others, line, quantiles }
    }
}

fn target_terms(default: Term) -> Vec<Term> {
    let mut v = vec![default, default.next_regular()];
    let summer_year = if default.kind == TermKind::T1 { default.year } else { default.year - 1 };
    v.push(Term { year: summer_year, kind: TermKind::T3A });
    v.push(Term { year: summer_year, kind: TermKind::T3B });
    v.sort();
    v.dedup();
    v
}

fn success_curve(ui: &mut Ui, d: &Drawn, my_bid: f64) {
    section_heading(ui, "Chance of getting a seat vs. bid");
    let muted = theme::muted(ui);
    Plot::new("success_curve")
        .height(280.0)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .include_y(0.0)
        .include_y(100.0)
        .include_x(0.0)
        .x_axis_label("Bid (e$)")
        .y_axis_formatter(|m, _| format!("{:.0}%", m.value))
        .label_formatter(|_, v| format!("bid {}\n{:.0}% chance", money(v.x), v.y))
        .legend(Legend::default().position(Corner::LeftTop))
        .show(ui, |p| {
            p.line(Line::new(PlotPoints::new(d.curve.clone())).name("P(seat)").color(theme::ACCENT).width(2.5).fill(0.0).fill_alpha(0.12));
            p.points(Points::new(PlotPoints::new(d.marks.clone())).name("Recommended bids").color(theme::ACCENT).radius(4.5).shape(MarkerShape::Diamond));
            p.vline(VLine::new(my_bid).name("My bid").color(theme::WARN).width(1.5));
            p.hline(HLine::new(80.0).color(muted.linear_multiply(0.4)).style(LineStyle::dashed_loose()));
        });
}

fn history_chart(ui: &mut Ui, h: &History, target: Term, chosen: bool) {
    section_heading(ui, "Clearing prices and forecast");
    let tx = h.terms.len() as f64;
    let muted = theme::muted(ui);
    term_plot("planner_history", h.terms.clone(), Some(format!("{} (forecast)", target.label())))
        .height(280.0)
        .include_y(0.0)
        .include_x(tx + 0.3)
        .legend(Legend::default().position(Corner::LeftBottom))
        .label_formatter(|name, v| if name.is_empty() { String::new() } else { format!("{name}\n{}", money(v.y)) })
        .show(ui, |p| {
            let others_name = if chosen { "Other instructors" } else { "Filled sections" };
            p.points(Points::new(PlotPoints::new(h.others.clone())).name(others_name).color(muted).radius(3.5));
            if chosen {
                p.points(Points::new(PlotPoints::new(h.mine.clone())).name("Chosen instructor").color(theme::ACCENT).radius(5.0));
            }
            p.line(Line::new(PlotPoints::new(h.line.clone())).name("Median clearing price").color(theme::ACCENT).width(2.0));
            if let Some([q10, q25, q50, q75, q90]) = h.quantiles {
                p.line(Line::new(PlotPoints::new(vec![[tx, q10], [tx, q90]])).name("Forecast 10–90%").color(theme::PURPLE.linear_multiply(0.6)).width(6.0));
                p.line(Line::new(PlotPoints::new(vec![[tx, q25], [tx, q75]])).name("Forecast 25–75%").color(theme::PURPLE).width(12.0));
                p.points(Points::new(PlotPoints::new(vec![[tx, q50]])).name("Forecast median").color(egui::Color32::WHITE).radius(4.0).shape(MarkerShape::Diamond));
            }
        });
}

fn observations_table(ui: &mut Ui, f: &Forecast) {
    let total: f64 = f.observations.iter().map(|o| o.weight).sum();
    let mut obs: Vec<_> = f.observations.iter().collect();
    obs.sort_by(|a, b| b.term.cmp(&a.term).then(b.weight.total_cmp(&a.weight)));
    egui::Grid::new("obs_table").striped(true).show(ui, |ui| {
        for h in ["Term", "Section", "Instructor", "Outcome", "Min bid", "Median bid", "Weight"] {
            ui.label(RichText::new(h).strong());
        }
        ui.end_row();
        for o in obs {
            ui.label(o.term.label());
            ui.label(&*o.section);
            ui.label(&*o.instructor);
            if o.filled {
                ui.label(RichText::new("Filled").color(theme::BAD));
            } else {
                ui.label(RichText::new("Seats left").color(theme::GOOD));
            }
            ui.label(if o.min_bid > 0.0 { money(o.min_bid) } else { "–".into() });
            ui.label(if o.median_bid > 0.0 { money(o.median_bid) } else { "–".into() });
            ui.label(format!("{:.1}%", o.weight / total * 100.0));
            ui.end_row();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{LoadReport, RoundKind};
    use crate::ui::test_data::sample_data;
    use std::collections::{BTreeSet, HashMap};

    fn key(data: &Arc<AppData>, filters: &GlobalFilters, sel: WindowSel, instructor: Option<&str>, target: Term) -> PlanKey {
        PlanKey::new(data, filters, "TEST101", sel, instructor, target)
    }

    /// The sample course with every section keeping its seats, so no section filled in any window.
    fn never_filled() -> AppData {
        let rows = sample_data().rows.into_iter().map(|mut r| { r.after_proc = r.before_proc; r }).collect();
        AppData::new(rows, HashMap::new(), LoadReport::default())
    }

    #[test]
    fn plan_key_changes_only_with_its_inputs() {
        let data = Arc::new(sample_data());
        let other_data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let excluded = GlobalFilters { excluded_sessions: BTreeSet::from(["Freshman Session".to_string()]) };
        let target = data.default_target_term();
        let w2 = WindowSel { round: Some(RoundKind::Round1), window: Some(2) };
        let base = key(&data, &filters, WindowSel::ROUND1_W1, None, target);

        assert_eq!(base, key(&data, &filters, WindowSel::ROUND1_W1, None, target));
        assert_ne!(base, key(&data, &filters, WindowSel::ROUND1_W1, Some("ALPHA"), target));
        assert_ne!(base, key(&data, &filters, w2, None, target));
        assert_ne!(base, key(&data, &filters, WindowSel::ROUND1_W1, None, target.next_regular()));
        assert_ne!(base, key(&data, &excluded, WindowSel::ROUND1_W1, None, target));
        assert_ne!(base, key(&other_data, &filters, WindowSel::ROUND1_W1, None, target));
    }

    #[test]
    fn slider_frames_reuse_the_plan_until_an_input_changes() {
        let data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let target = data.default_target_term();
        let mut slot = None;
        let mut builds = 0;
        // (instructor, my bid) per frame: the slider moves, then the instructor changes, then back.
        let frames = [(None, 20.0), (None, 35.0), (None, 55.0), (Some("BETA"), 55.0), (Some("BETA"), 12.0), (None, 12.0)];
        for (instructor, my_bid) in frames {
            let k = key(&data, &filters, WindowSel::ROUND1_W1, instructor, target);
            let plan = memo(&mut slot, k, |k| {
                builds += 1;
                Plan::new(&data, &filters, &params, k)
            });
            let p = plan.drawn.as_ref().expect("sample data has a forecast").fc.success_prob(my_bid);
            assert!((0.0..=1.0).contains(&p));
        }
        assert_eq!(builds, 3);
    }

    #[test]
    fn recommended_bids_are_whole_cents_that_meet_the_chance() {
        let data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let target = data.default_target_term();
        let k = key(&data, &filters, WindowSel::ROUND1_W1, None, target);
        let plan = Plan::new(&data, &filters, &params, &k);
        let d = plan.drawn.as_ref().expect("sample data has a forecast");

        assert_eq!(d.bids.len(), LEVELS.len());
        for (level, rec) in LEVELS.iter().zip(&d.bids) {
            let cents = rec.bid * 100.0;
            assert!((cents - cents.round()).abs() < 1e-6, "{} is not whole cents", rec.bid);
            let exact = d.fc.bid_for(*level);
            assert!(rec.bid >= exact - 1e-9 && rec.bid < exact + 0.01, "{level}: bid {} vs exact {exact}", rec.bid);
            assert!(d.fc.success_prob(rec.bid) >= level - 1e-6, "{level}: bid {} does not reach the chance", rec.bid);
        }
        let cli = plan_bid(&data, &filters, &params, &ForecastQuery { course: "TEST101", sel: k.sel, instructor: None, target }, &LEVELS, &[])
            .expect("course exists");
        let cli_bids: Vec<f64> = cli.recommended.iter().map(|l| l.bid).collect();
        let gui_bids: Vec<f64> = d.bids.iter().map(|l| l.bid).collect();
        assert_eq!(cli_bids, gui_bids);
    }

    #[test]
    fn history_drops_later_terms_and_splits_the_chosen_instructor() {
        let data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        // Target one term earlier than the default, so the latest term must not be drawn.
        let target = Term { year: 2025, kind: TermKind::T1 };
        let k = key(&data, &filters, WindowSel::ROUND1_W1, Some("BETA"), target);
        let plan = Plan::new(&data, &filters, &params, &k);
        let d = plan.drawn.as_ref().expect("sample data has a forecast");

        assert!(d.history.terms.iter().all(|t| *t < target));
        assert_eq!(d.history.terms.len(), 6);
        // Sample data: BETA's filled section every term goes to `mine`; ALPHA's G1 and GAMMA's G4 fill too.
        assert_eq!((d.history.mine.len(), d.history.others.len()), (6, 12));
        assert!(d.history.quantiles.is_some());
    }

    #[test]
    fn the_cached_plan_caps_confidence_and_keeps_the_warnings_when_nothing_filled() {
        let data = Arc::new(never_filled());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let target = data.default_target_term();
        let k = key(&data, &filters, WindowSel::ROUND1_W1, None, target);
        let mut slot = None;
        let plan = memo(&mut slot, k, |k| Plan::new(&data, &filters, &params, k));

        let d = plan.drawn.as_ref().expect("the sections had seats open");
        assert_eq!(d.fc.confidence, Confidence::Medium, "the forecast alone would say Medium");
        assert_eq!(plan.confidence, Confidence::Low, "nothing filled, so there is no clearing price to be confident about");
        assert_eq!(d.median, None);
        assert!(
            plan.warnings.iter().any(|w| w.starts_with("No past section filled in Round 1 Window 1, so the price estimate falls back")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn the_plan_keeps_its_warnings_when_no_past_section_had_seats_open() {
        let data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        // Bidding for a term before any of the sample's history: there is no forecast, but the CLI would still explain why.
        let target = Term { year: 2022, kind: TermKind::T1 };
        let k = key(&data, &filters, WindowSel::ROUND1_W1, None, target);
        let plan = Plan::new(&data, &filters, &params, &k);

        assert!(plan.drawn.is_none());
        assert!(
            plan.warnings.iter().any(|w| w.starts_with("No section in Round 1 Window 1 had seats open before")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn an_instructor_without_history_is_warned_about_by_the_plan() {
        let data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let target = data.default_target_term();
        let k = key(&data, &filters, WindowSel::ROUND1_W1, Some("NOBODY"), target);
        let plan = Plan::new(&data, &filters, &params, &k);

        assert!(plan.drawn.is_some(), "the course-wide data still gives a forecast");
        assert!(
            plan.warnings.contains(&"NOBODY has no history in this window; using course-wide data.".to_string()),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn the_plan_confidence_label_maps_back_to_the_model_level() {
        for level in [Confidence::Low, Confidence::Medium, Confidence::High] {
            assert_eq!(confidence_from_label(level.label()), level);
        }
        assert_eq!(confidence_from_label("Certain"), Confidence::Low);
    }
}
