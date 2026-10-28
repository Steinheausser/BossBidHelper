use crate::data::filter::{course_rows, GlobalFilters, WindowSel};
use crate::data::model::{ForecastQuery, ModelParams};
use crate::data::query::{
    hot_courses_with_min_filled, latest_regular_term, natural_section, plan_bid, HotCourse, DEFAULT_HOT_MIN_FILLED,
};
use crate::data::stats::{instructor_ranking, term_summaries, InstructorStats, TermSummary};
use crate::data::{AppData, BidRow, Term};
use crate::ui::widgets::{money, normalise_selection, pct, section_heading, stat_card, term_plot, window_selector};
use crate::ui::{memo, theme, CourseOptions, Ctx, Tab};
use egui::{RichText, Ui};
use egui_plot::{Bar, BarChart, Corner, Legend, Line, LineStyle, MarkerShape, PlotPoints, Points};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Shared with the CLI's `hot --min-filled` default.
const HOT_MIN_FILLED: usize = DEFAULT_HOT_MIN_FILLED;

/// The courses for "Most contested courses": Round 1 Window 1 of `term`, ranked as the CLI `hot` command ranks them.
fn hot_list(data: &AppData, filters: &GlobalFilters, term: Term) -> Vec<HotCourse> {
    hot_courses_with_min_filled(data, filters, term, WindowSel::ROUND1_W1, HOT_MIN_FILLED)
}

#[derive(Default)]
pub struct OverviewState {
    hot_courses: Option<(GlobalFilters, Term, Vec<HotCourse>)>,
    options: Option<((usize, String), CourseOptions)>,
    course: Option<(CourseKey, CourseData)>,
}

impl OverviewState {
    pub fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        egui::ScrollArea::vertical().auto_shrink([true, false]).show(ui, |ui| match cx.selection.course.clone() {
            None => self.dashboard(ui, cx),
            Some(code) => self.course_view(ui, cx, &code),
        });
    }

    fn dashboard(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let data = cx.data;
        ui.heading("Welcome");
        ui.label("Pick a course on the left to see its bidding history, or start with the most contested courses below.");
        ui.add_space(8.0);

        ui.horizontal_wrapped(|ui| {
            let terms = data.terms.first().zip(data.terms.last()).map(|(a, b)| format!("{a} – {b}")).unwrap_or_default();
            stat_card(ui, "Terms loaded", data.terms.len().to_string(), terms, None);
            stat_card(ui, "Courses", data.courses.len().to_string(), "", None);
            stat_card(ui, "Section-window results", data.rows.len().to_string(), "after cleaning", None);
            stat_card(ui, "Bidding for", data.default_target_term().label(), "next regular term", Some(theme::ACCENT));
        });

        let Some(term) = latest_regular_term(data) else { return };
        let stale = !matches!(&self.hot_courses, Some((f, t, _)) if f == cx.filters && *t == term);
        if stale {
            self.hot_courses = Some((cx.filters.clone(), term, hot_list(data, cx.filters, term)));
        }
        let hot = &self.hot_courses.as_ref().unwrap().2;

        section_heading(ui, &format!("Most contested courses — {term}, Round 1 Window 1"));
        ui.label(
            RichText::new(format!("Courses with at least {HOT_MIN_FILLED} filled sections, ranked by median clearing price. Click a course to open it."))
                .small()
                .color(theme::muted(ui)),
        );
        let mut picked = None;
        theme::card(ui).show(ui, |ui| {
            if hot.is_empty() {
                ui.label(RichText::new(format!("No course had {HOT_MIN_FILLED} or more filled sections in {term}, Round 1 Window 1.")).color(theme::muted(ui)));
                return;
            }
            egui::Grid::new("hot_courses").striped(true).min_col_width(60.0).show(ui, |ui| {
                for h in ["#", "Course", "Title", "Median clearing", "Sections filled"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, c) in hot.iter().take(25).enumerate() {
                    ui.label(format!("{}", i + 1));
                    if ui.link(&c.code).clicked() {
                        picked = Some(c.code.clone());
                    }
                    ui.label(&c.description);
                    ui.label(RichText::new(money(c.median_clearing)).strong());
                    ui.label(format!("{}/{}", c.sections_filled, c.sections_available));
                    ui.end_row();
                }
            });
        });
        if let Some(code) = picked {
            if let Some(c) = data.course(&code) {
                cx.saved.auto_save_click(&code, &c.description);
            }
            cx.selection.select_course(&code);
            cx.selection.sel = WindowSel::ROUND1_W1;
        }
    }

    fn course_view(&mut self, ui: &mut Ui, cx: &mut Ctx, code: &str) {
        let data = cx.data;
        let Some(info) = data.course(code) else { return };

        ui.horizontal(|ui| {
            ui.label(RichText::new(code).size(24.0).strong().color(theme::ACCENT));
            ui.label(RichText::new(&info.description).size(18.0));
        });
        ui.label(RichText::new(format!("{} · last offered {}", info.school, info.last_term)).color(theme::muted(ui)));
        ui.add_space(4.0);

        let opts = memo(&mut self.options, (Arc::as_ptr(data) as usize, code.to_string()), |(_, c)| CourseOptions::new(data, c));
        normalise_selection(&opts.windows, &mut cx.selection.sel, true);
        ui.horizontal(|ui| {
            window_selector(ui, "overview", &opts.windows, &mut cx.selection.sel, true);
            ui.add_space(12.0);
            if ui.button("🎯 Plan a bid").clicked() {
                *cx.tab = Tab::Prediction;
            }
        });
        if !cx.selection.sel.is_specific() {
            ui.label(RichText::new("Tip: prices differ a lot between windows; pick a specific round and window for meaningful numbers.").small().color(theme::WARN));
        }
        ui.add_space(6.0);

        // The instructor selection does not change anything on this page, so it is not part of the key.
        let key = CourseKey {
            data: Arc::as_ptr(data) as usize,
            code: code.to_string(),
            sel: cx.selection.sel,
            target: cx.selection.target.unwrap_or_else(|| data.default_target_term()),
            filters: cx.filters.clone(),
        };
        let target = key.target;
        let d = memo(&mut self.course, key, |k| CourseData::new(data, cx.filters, cx.params, k));
        let Some(last) = d.summaries.last() else {
            ui.label("No results for this course in the selected round/window.");
            return;
        };

        // Headline cards.
        ui.horizontal_wrapped(|ui| {
            let clearing = last.clearing_median.map(money).unwrap_or_else(|| "No fill".into());
            let range = match (last.clearing_low, last.clearing_high) {
                (Some(lo), Some(hi)) if hi > lo => format!("range {} – {}", money(lo), money(hi)),
                _ => String::new(),
            };
            stat_card(ui, &format!("Clearing price · {}", last.term), clearing, range, None);
            let fill_color = if last.fill_rate() > 0.66 { theme::BAD } else if last.fill_rate() > 0.33 { theme::WARN } else { theme::GOOD };
            // Outside one round and window a section can be offered in several windows, so these cards count section-windows.
            let one_window = cx.selection.sel.is_specific();
            let (filled_title, open_note) = if one_window {
                ("Sections that filled", "of open sections")
            } else {
                ("Section-windows that filled", "of open section-windows")
            };
            stat_card(
                ui,
                filled_title,
                format!("{}/{}", last.filled, last.available),
                format!("{} {open_note}", pct(last.fill_rate())),
                Some(fill_color),
            );
            let sections_note = if one_window {
                format!("{} sections", last.sections)
            } else {
                format!("{} sections, {} section-windows", last.sections, last.section_windows)
            };
            stat_card(ui, "Seats open / left after", format!("{} / {}", last.seats_open, last.seats_left), sections_note, None);
            if let Some((bid, confidence)) = &d.bid80 {
                stat_card(
                    ui,
                    &format!("Bid for 80% chance · {}", target),
                    money(*bid),
                    format!("{confidence} confidence · any section"),
                    Some(theme::ACCENT),
                );
            }
        });

        // Price history chart.
        section_heading(ui, "Clearing price history");
        ui.label(
            RichText::new("Each dot is one section. Filled sections show the real clearing price; hollow dots did not fill, so any bid ≥ e$10 got in (their reported min bid is shown).")
                .small()
                .color(theme::muted(ui)),
        );
        let muted = theme::muted(ui);
        term_plot("overview_price", d.terms.clone(), None)
            .height(300.0)
            .legend(Legend::default().position(Corner::LeftBottom))
            .include_y(0.0)
            .label_formatter(|name, v| if name.is_empty() { String::new() } else { format!("{name}\n{}", money(v.y)) })
            .show(ui, |p| {
                p.points(Points::new(PlotPoints::new(d.filled.clone())).name("Filled section (clearing price)").color(theme::ACCENT).radius(4.0).shape(MarkerShape::Circle));
                p.points(Points::new(PlotPoints::new(d.open.clone())).name("Did not fill").color(muted).radius(4.0).filled(false).shape(MarkerShape::Circle));
                p.line(Line::new(PlotPoints::new(d.clearing.clone())).name("Median clearing price").color(theme::ACCENT).width(2.5));
                p.line(Line::new(PlotPoints::new(d.median.clone())).name("Median of median bids").color(theme::PURPLE).style(LineStyle::dashed_loose()).width(1.5));
            });

        section_heading(ui, "Seats per term");
        term_plot("overview_seats", d.terms.clone(), None).height(170.0).legend(Legend::default().position(Corner::LeftTop)).include_y(0.0).show(ui, |p| {
            p.bar_chart(BarChart::new(d.open_bars.clone()).name("Seats open before window").color(theme::ACCENT));
            p.bar_chart(BarChart::new(d.left_bars.clone()).name("Seats left after window").color(theme::GOOD));
        });

        // Latest term section table.
        section_heading(ui, &format!("Sections in {}", last.term));
        theme::card(ui).show(ui, |ui| {
            egui::Grid::new("latest_sections").striped(true).min_col_width(50.0).show(ui, |ui| {
                let show_window = !cx.selection.sel.is_specific();
                for h in ["Section", "Instructor", "Window", "Seats before / after", "Min bid", "Median bid", "Outcome"] {
                    if h == "Window" && !show_window {
                        continue;
                    }
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for r in &d.latest {
                    ui.label(&*r.section);
                    ui.label(&*r.instructor);
                    if show_window {
                        ui.label(format!("{} W{}", r.round, r.window));
                    }
                    ui.label(format!("{} / {}", r.before_proc, r.after_proc));
                    ui.label(if r.min_bid > 0.0 { money(r.min_bid) } else { "–".into() });
                    ui.label(if r.median_bid > 0.0 { money(r.median_bid) } else { "–".into() });
                    let (txt, color) = if !r.available() {
                        ("No seats", theme::muted(ui))
                    } else if r.filled() {
                        ("Filled", theme::BAD)
                    } else {
                        ("Seats left", theme::GOOD)
                    };
                    ui.label(RichText::new(txt).color(color));
                    ui.end_row();
                }
            });
        });

        // Instructor comparison within the course.
        section_heading(ui, "Instructors for this course");
        let mut plan_for = None;
        theme::card(ui).show(ui, |ui| {
            egui::Grid::new("course_instructors").striped(true).min_col_width(50.0).show(ui, |ui| {
                for h in ["Instructor", "Sections", "Filled", "Median clearing", "vs. other sections", "Last taught", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for s in &d.instructors {
                    ui.label(&s.name);
                    ui.label(s.sections.to_string());
                    ui.label(pct(s.fill_rate));
                    ui.label(s.median_clearing.map(money).unwrap_or_else(|| "–".into()));
                    premium_label(ui, s.premium, s.compared);
                    ui.label(s.last_term.label());
                    if ui.small_button("Plan bid").clicked() {
                        plan_for = Some(s.name.clone());
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(name) = plan_for {
            cx.selection.instructor = Some(name);
            *cx.tab = Tab::Prediction;
        }
    }
}

/// Identifies the course page's derived data. Instructor choice is not an input: this page does
/// not use it.
#[derive(Debug, PartialEq)]
struct CourseKey {
    /// Address of the loaded data set; a reload gives a new one.
    data: usize,
    code: String,
    sel: WindowSel,
    target: Term,
    filters: GlobalFilters,
}

/// Everything the course page draws for one `CourseKey`.
struct CourseData {
    summaries: Vec<TermSummary>,
    terms: Vec<Term>,
    /// Recommended bid for an 80% chance and its confidence; only for a specific round and window.
    bid80: Option<(f64, String)>,
    /// (x, price) for sections that filled and for those that did not.
    filled: Vec<[f64; 2]>,
    open: Vec<[f64; 2]>,
    /// Median clearing price and median of median bids per term.
    clearing: Vec<[f64; 2]>,
    median: Vec<[f64; 2]>,
    open_bars: Vec<Bar>,
    left_bars: Vec<Bar>,
    /// Sections of the latest term, in section order.
    latest: Vec<BidRow>,
    instructors: Vec<InstructorStats>,
}

impl CourseData {
    fn new(data: &AppData, filters: &GlobalFilters, params: &ModelParams, key: &CourseKey) -> Self {
        let rows = course_rows(data, &key.code, key.sel, filters);
        let summaries = term_summaries(&rows);
        let terms: Vec<Term> = summaries.iter().map(|s| s.term).collect();

        let bid80 = if key.sel.is_specific() {
            let q = ForecastQuery { course: key.code.as_str(), sel: key.sel, instructor: None, target: key.target };
            plan_bid(data, filters, params, &q, &[0.8], &[]).and_then(|p| p.recommended.first().map(|l| (l.bid, p.confidence)))
        } else {
            None
        };

        let index: BTreeMap<Term, f64> = terms.iter().enumerate().map(|(i, t)| (*t, i as f64)).collect();
        let (mut filled, mut open) = (Vec::new(), Vec::new());
        for r in rows.iter().filter(|r| r.available() && r.min_bid > 0.0) {
            // Spread sections a little so they do not overlap.
            let jitter = ((r.section.bytes().map(|b| b as u32).sum::<u32>() % 7) as f64 - 3.0) * 0.03;
            let p = [index[&r.term] + jitter, r.min_bid];
            if r.filled() { filled.push(p) } else { open.push(p) }
        }
        let clearing = summaries.iter().enumerate().filter_map(|(i, s)| s.clearing_median.map(|c| [i as f64, c])).collect();
        let median = summaries.iter().enumerate().filter_map(|(i, s)| s.median_bid.map(|c| [i as f64, c])).collect();
        let open_bars = summaries.iter().enumerate().map(|(i, s)| Bar::new(i as f64 - 0.17, s.seats_open as f64).width(0.32)).collect();
        let left_bars = summaries.iter().enumerate().map(|(i, s)| Bar::new(i as f64 + 0.17, s.seats_left as f64).width(0.32)).collect();

        let mut latest: Vec<BidRow> = match summaries.last() {
            Some(last) => rows.iter().copied().filter(|r| r.term == last.term).cloned().collect(),
            None => Vec::new(),
        };
        latest.sort_by(|a, b| natural_section(&a.section).cmp(&natural_section(&b.section)).then(a.round.cmp(&b.round)).then(a.window.cmp(&b.window)));

        let mut instructors = instructor_ranking(&rows);
        instructors.sort_by(|a, b| b.last_term.cmp(&a.last_term).then(b.premium.total_cmp(&a.premium)));

        Self { summaries, terms, bid80, filled, open, clearing, median, open_bars, left_bars, latest, instructors }
    }
}

pub fn premium_label(ui: &mut Ui, premium: f64, compared: usize) {
    if compared == 0 {
        ui.label(RichText::new("only section").color(theme::muted(ui)));
        return;
    }
    let color = if premium > 0.1 { theme::BAD } else if premium < -0.1 { theme::GOOD } else { theme::muted(ui) };
    ui.label(RichText::new(format!("{:+.0}%", premium * 100.0)).color(color))
        .on_hover_text(format!("Compared against other sections of the same course/term/window {compared} time(s). Positive = harder to get."));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{LoadReport, RoundKind, TermKind};
    use crate::ui::prediction::LEVELS;
    use crate::ui::test_data::sample_data;
    use std::collections::{BTreeSet, HashMap};

    fn key(data: &Arc<AppData>, filters: &GlobalFilters, sel: WindowSel, target: Term) -> CourseKey {
        CourseKey { data: Arc::as_ptr(data) as usize, code: "TEST101".to_string(), sel, target, filters: filters.clone() }
    }

    /// Sections of one course in 2025-26 T2, Round 1 Window 1: one per entry of `filled`. A filled section
    /// clears at e$20; the others keep four seats.
    fn rows_for(code: &str, filled: &[bool]) -> Vec<BidRow> {
        let term = Term { year: 2025, kind: TermKind::T2 };
        filled
            .iter()
            .enumerate()
            .map(|(i, &is_filled)| BidRow {
                term,
                session: Arc::from("Regular Academic Session"),
                round: RoundKind::Round1,
                window: 1,
                course_code: Arc::from(code),
                section: Arc::from(format!("G{}", i + 1)),
                instructor: Arc::from("Ann Lee"),
                school: Arc::from("SCIS"),
                vacancy: 40,
                opening_vacancy: 10,
                before_proc: 10,
                dice: 0,
                after_proc: if is_filled { 0 } else { 4 },
                enrolled: if is_filled { 10 } else { 6 },
                median_bid: 22.0,
                min_bid: 20.0,
            })
            .collect()
    }

    #[test]
    fn the_hot_list_leaves_out_courses_with_fewer_than_two_filled_sections() {
        let mut rows = rows_for("ONEFILL", &[true, false]);
        rows.extend(rows_for("TWOFILL", &[true, true, false]));
        rows.extend(rows_for("NOFILL", &[false, false]));
        let data = AppData::new(rows, HashMap::new(), LoadReport::default());
        let filters = GlobalFilters::default();
        let term = Term { year: 2025, kind: TermKind::T2 };

        let hot: Vec<String> = hot_list(&data, &filters, term).into_iter().map(|h| h.code).collect();
        assert_eq!(hot, vec!["TWOFILL"]);
        // One filled section would be enough for the query layer's default of one, so ONEFILL is the course this leaves out.
        assert!(hot_courses_with_min_filled(&data, &filters, term, WindowSel::ROUND1_W1, 1).iter().any(|h| h.code == "ONEFILL"));
    }

    #[test]
    fn the_80_percent_card_is_low_confidence_when_no_section_filled() {
        // The sample course with every section keeping its seats, so nothing filled in any window.
        let rows = sample_data().rows.into_iter().map(|mut r| { r.after_proc = r.before_proc; r }).collect();
        let data = Arc::new(AppData::new(rows, HashMap::new(), LoadReport::default()));
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let target = data.default_target_term();
        let d = CourseData::new(&data, &filters, &params, &key(&data, &filters, WindowSel::ROUND1_W1, target));

        let (_, confidence) = d.bid80.clone().expect("the sections had seats open");
        assert_eq!(confidence, "Low");
        assert_eq!(d.summaries.last().and_then(|s| s.clearing_median), None);
    }

    #[test]
    fn course_data_is_rebuilt_only_for_window_target_filters_or_data() {
        let data = Arc::new(sample_data());
        let other_data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let excluded = GlobalFilters { excluded_sessions: BTreeSet::from(["Freshman Session".to_string()]) };
        let params = ModelParams::default();
        let target = data.default_target_term();
        let w1 = WindowSel::ROUND1_W1;
        let w2 = WindowSel { round: Some(RoundKind::Round1), window: Some(2) };

        let frames = [
            (&data, &filters, w1, target),
            (&data, &filters, w1, target),
            (&data, &filters, w2, target),
            (&data, &filters, w2, target.next_regular()),
            (&data, &excluded, w2, target.next_regular()),
            (&data, &excluded, w2, target.next_regular()),
            (&other_data, &excluded, w2, target.next_regular()),
        ];
        let mut slot = None;
        let mut builds = 0;
        for (data, filters, sel, target) in frames {
            let k = key(data, filters, sel, target);
            memo(&mut slot, k, |k| {
                builds += 1;
                CourseData::new(data, filters, &params, k)
            });
        }
        assert_eq!(builds, 5);
    }

    #[test]
    fn overview_80_percent_bid_matches_the_planner_to_the_cent() {
        let data = Arc::new(sample_data());
        let filters = GlobalFilters::default();
        let params = ModelParams::default();
        let target = data.default_target_term();
        let k = key(&data, &filters, WindowSel::ROUND1_W1, target);
        let d = CourseData::new(&data, &filters, &params, &k);
        let q = ForecastQuery { course: "TEST101", sel: WindowSel::ROUND1_W1, instructor: None, target };
        let planner = plan_bid(&data, &filters, &params, &q, &LEVELS, &[]).expect("course exists");
        let eighty = LEVELS.iter().position(|&l| l == 0.8).expect("80% is a planner level");
        let (bid, confidence) = d.bid80.clone().expect("specific window has an 80% bid");
        assert_eq!(bid, planner.recommended[eighty].bid);
        assert_eq!(confidence, planner.confidence);

        let all = CourseData::new(&data, &filters, &params, &key(&data, &filters, WindowSel::default(), target));
        assert!(all.bid80.is_none(), "no 80% bid when all windows are shown");
        assert!(!all.summaries.is_empty());
    }
}
