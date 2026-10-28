//! `plan`: recommended bids for one course, and the chance of any bids you have in mind.
//!
//! A thin layer over `query::plan_bid`, which is also what the GUI's Bid Planner shows, so the two
//! agree to the cent. The JSON is the `BidPlan` as the query layer builds it, except that
//! `observations` is empty unless `--observations` is given, and `warnings` also holds the notes the
//! CLI adds: a target already in the data, a bid under the minimum, a chance outside the priced range,
//! and any data file that could not be read.

use super::output::{ascii, money, pct, Table};
use super::{parse_term, parse_window, CliError, Ctx};
use crate::data::filter::WindowSel;
use crate::data::model::{ForecastQuery, MIN_BID};
use crate::data::query::{self, round2, round4, BidPlan, DEFAULT_LEVELS};
use crate::data::Term;
use clap::Args;

/// The chances the model can price. `Forecast::bid_for` clamps to this range (see `model.rs`), so a
/// level outside it gets the bid for the nearer end, and the plan says so.
const PRICED_CHANCES: (f64, f64) = (0.01, 0.995);

/// The chance the text view marks as suggested. It is marked only when it is one of the levels.
const SUGGESTED_CHANCE: f64 = 0.8;

/// Parses a chance such as 0.8 (for 80%). Rejects 0, 1, and values written as percentages such as 80.
/// clap adds the argument name and the value, so these messages say only what is wrong.
fn probability(text: &str) -> Result<f64, String> {
    let p: f64 = text
        .trim()
        .parse()
        .map_err(|_| "not a number; write a chance such as 0.8 for 80%".to_string())?;
    if p > 0.0 && p < 1.0 {
        Ok(p)
    } else {
        Err("must be a chance between 0 and 1, such as 0.8 for 80%".to_string())
    }
}

#[derive(Debug, Args)]
pub struct PlanArgs {
    /// Course code, e.g. IS216. Case and spaces do not matter.
    pub code: String,
    /// Round and window, e.g. R1W1, R1AW2 or 1:1. "R1" means every window of that round, and "all" every round and window together. Prices differ a lot between windows, so a specific one is best.
    #[arg(long, default_value = "R1W1", value_name = "W")]
    pub window: String,
    /// Instructor to price for. Part of the name is enough. Their own past sections count for more.
    #[arg(long, value_name = "NAME")]
    pub instructor: Option<String>,
    /// Term you are bidding for, e.g. "2026-27 T1". Defaults to the regular term after the latest regular (non-summer) term in the data, which `info` reports as default_target_term. A term already in the data gives what the plan would have said then.
    #[arg(long, value_name = "TERM")]
    pub target: Option<String>,
    /// A bid to check, in e$, e.g. --bid 25. Repeat for several; the output gives the chance of a seat at each. Must be zero or more. A bid under e$10 (the BOSS minimum) has no chance.
    #[arg(long, value_name = "E$", allow_negative_numbers = true)]
    pub bid: Vec<f64>,
    /// Chances to recommend a bid for, comma-separated, e.g. 0.5,0.8,0.9. Defaults to 0.5,0.7,0.8,0.9,0.95.
    #[arg(long, value_name = "P", value_delimiter = ',', value_parser = probability)]
    pub levels: Vec<f64>,
    /// Include the past sections the forecast is based on. Without it, the JSON `observations` list is empty and the text view has no table.
    #[arg(long)]
    pub observations: bool,
}

/// The plan and what the text view needs beyond it.
struct Planned {
    plan: BidPlan,
    /// The window as asked. A specific one is a single round and window.
    sel: WindowSel,
    /// The bids checked, to the cent, in the order given.
    bids: Vec<f64>,
}

/// Prints the plan: a text view, or with `--json` the `BidPlan`. Warnings go to stderr in text mode.
pub fn run(ctx: &Ctx, args: &PlanArgs) -> Result<(), CliError> {
    let planned = build(ctx, args)?;
    ctx.warn_text(&planned.plan.warnings);
    ctx.emit(&planned.plan, || println!("{}", render(&planned)))
}

/// Checks the arguments, then asks the query layer for the plan.
fn build(ctx: &Ctx, args: &PlanArgs) -> Result<Planned, CliError> {
    let sel = parse_window(&args.window)?;
    let target = match &args.target {
        Some(text) => parse_term(text)?,
        None => ctx.data.default_target_term(),
    };
    let checked = checked_bids(&args.bid)?;
    let levels = chances(&args.levels);
    let info = ctx.course(&args.code)?;
    let instructor = args
        .instructor
        .as_deref()
        .map(|name| ctx.instructor(&info.code, name))
        .transpose()?;

    let q = ForecastQuery {
        course: &info.code,
        sel,
        instructor: instructor.as_deref(),
        target,
    };
    let mut plan = query::plan_bid(&ctx.data, &ctx.filters, &ctx.params, &q, &levels, &checked)
        .ok_or_else(|| CliError::not_found(format!("no course with code \"{}\"", args.code.trim())))?;

    let bids: Vec<f64> = checked.iter().map(|&b| round2(b)).collect();
    // Data problems first, then a target already in the data, then what the query layer and the
    // arguments say.
    let mut warnings = ctx.data_warnings();
    warnings.extend(past_target_note(target, ctx.data.latest_term()));
    warnings.append(&mut plan.warnings);
    warnings.extend(bid_warnings(&bids));
    warnings.extend(level_warnings(&levels));
    plan.warnings = warnings;
    if !args.observations {
        plan.observations.clear();
    }
    Ok(Planned { plan, sel, bids })
}

/// The chances to price: the caller's, sorted, with repeats removed (two chances that print the same
/// at four decimals count as one). With none given, the defaults.
fn chances(requested: &[f64]) -> Vec<f64> {
    if requested.is_empty() {
        return DEFAULT_LEVELS.to_vec();
    }
    let mut levels = requested.to_vec();
    levels.sort_by(f64::total_cmp);
    levels.dedup_by(|a, b| round4(*a) == round4(*b));
    levels
}

/// Bids must be amounts of zero or more. Negative zero is written as zero.
fn checked_bids(bids: &[f64]) -> Result<Vec<f64>, CliError> {
    bids.iter()
        .map(|&b| {
            if !b.is_finite() {
                Err(CliError::usage(format!(
                    "--bid {b} is not an amount; write it in e$, e.g. --bid 25"
                )))
            } else if b < 0.0 {
                Err(CliError::usage(format!(
                    "--bid {b} is negative; a bid is an amount in e$ of zero or more"
                )))
            } else {
                Ok(if b == 0.0 { 0.0 } else { b })
            }
        })
        .collect()
}

/// A bid under the BOSS minimum gets no chance whatever the history says, so the plan says so.
fn bid_warnings(bids: &[f64]) -> Vec<String> {
    let mut low: Vec<f64> = bids.iter().copied().filter(|&b| b < MIN_BID).collect();
    low.sort_by(f64::total_cmp);
    low.dedup();
    let listed: Vec<String> = low.iter().map(|&b| money(b)).collect();
    match listed.as_slice() {
        [] => Vec::new(),
        [only] => vec![format!(
            "Bid {only} is below the {} minimum bid, so its chance is 0%.",
            money(MIN_BID)
        )],
        _ => vec![format!(
            "Bids {} are below the {} minimum bid, so their chance is 0%.",
            join_and(&listed),
            money(MIN_BID)
        )],
    }
}

/// A chance outside the range the model prices gets the bid for the nearer end of that range.
fn level_warnings(levels: &[f64]) -> Vec<String> {
    let (low, high) = PRICED_CHANCES;
    levels
        .iter()
        .filter_map(|&p| {
            if p < low {
                Some(format!(
                    "Chance {} is below the {} the model can price, so its bid is the one for {}.",
                    chance_label(p),
                    chance_label(low),
                    chance_label(low)
                ))
            } else if p > high {
                Some(format!(
                    "Chance {} is above the {} the model can price, so its bid is the one for {}.",
                    chance_label(p),
                    chance_label(high),
                    chance_label(high)
                ))
            } else {
                None
            }
        })
        .collect()
}

/// A target no later than the latest term in the data. The plan then uses only the terms before it,
/// so it says what the planner would have said at the time.
fn past_target_note(target: Term, latest: Option<Term>) -> Option<String> {
    let latest = latest.filter(|&last| target <= last)?;
    Some(format!(
        "{} is not after the latest term in the data ({}), so the plan uses only the terms before {}.",
        target.label(),
        latest.label(),
        target.label()
    ))
}

/// The text view. Blocks are separated by a blank line: the header, the recommended bids with the
/// forecast's summary, the bids checked and, when asked, the past sections. Warnings are not in it;
/// they go to stderr.
fn render(planned: &Planned) -> String {
    let plan = &planned.plan;
    let mut blocks: Vec<Vec<String>> = vec![header_lines(planned)];

    if plan.recommended.is_empty() {
        blocks.push(vec![
            "No bid can be recommended: there is no usable history for this window (see the warnings).".to_string(),
        ]);
    } else {
        let mut lines = recommended_lines(plan);
        lines.extend(summary_lines(plan));
        blocks.push(lines);
    }
    if !planned.bids.is_empty() {
        blocks.push(checked_lines(planned));
    }
    if !plan.observations.is_empty() {
        blocks.push(vec!["Past sections used:".to_string(), observations_table(plan)]);
    }

    let text = blocks
        .iter()
        .map(|block| block.join("\n"))
        .collect::<Vec<_>>()
        .join("\n\n");
    ascii(&text)
}

fn header_lines(planned: &Planned) -> Vec<String> {
    let plan = &planned.plan;
    let description = one_line(&plan.description);
    let title = if description.is_empty() {
        plan.course.clone()
    } else {
        format!("{}  {description}", plan.course)
    };
    let window = if planned.sel.is_specific() {
        format!("{} ({})", plan.window, plan.window_code)
    } else {
        plan.window.clone()
    };
    let who = plan
        .instructor
        .as_deref()
        .map_or_else(|| "any section".to_string(), one_line);
    vec![
        title,
        format!("{window} | bidding for {} | instructor: {who}", plan.target),
    ]
}

/// One row per recommended bid, under a CHANCE and BID heading. The 80% row is marked as suggested.
fn recommended_lines(plan: &BidPlan) -> Vec<String> {
    let chances: Vec<String> = plan.recommended.iter().map(|r| chance_label(r.probability)).collect();
    let bids: Vec<String> = plan.recommended.iter().map(|r| money(r.bid)).collect();
    let chance_w = chances.iter().map(String::len).max().unwrap_or(0).max("CHANCE".len());
    let bid_w = bids.iter().map(String::len).max().unwrap_or(0).max("BID".len());

    let mut lines = vec![format!("{:<chance_w$}  BID", "CHANCE")];
    for ((rec, chance), bid) in plan.recommended.iter().zip(&chances).zip(&bids) {
        let mut notes = Vec::new();
        if rec.is_minimum_bid {
            notes.push("(minimum bid)");
        }
        if is_suggested(rec.probability) {
            notes.push("<- suggested");
        }
        lines.push(
            format!("{chance:<chance_w$}  {bid:<bid_w$}  {}", notes.join("  "))
                .trim_end()
                .to_string(),
        );
    }
    lines
}

/// The forecast's summary under the recommended bids, as the GUI planner words it.
fn summary_lines(plan: &BidPlan) -> Vec<String> {
    let with_instructor = plan.instructor.as_ref().map_or_else(String::new, |_| {
        format!(" ({} with this instructor)", plan.n_instructor_obs)
    });
    let trend_note = if plan.trend_applied {
        "applied to forecast"
    } else {
        "for info; not extrapolated"
    };
    let clearing = plan
        .median_clearing
        .map_or_else(|| "no past section filled".to_string(), money);
    let band = plan.clearing_quantiles.as_ref().map_or_else(
        || "-".to_string(),
        |q| format!("10th {}, median {}, 90th {}", money(q.p10), money(q.p50), money(q.p90)),
    );
    vec![
        format!(
            "Confidence: {} - based on {} over {}{with_instructor}",
            plan.confidence,
            plural(plan.n_observations, "past section-result"),
            plural(plan.n_terms, "term")
        ),
        format!(
            "Section fills: {} of the time (if it doesn't, any bid >= e${MIN_BID:.0} wins)",
            whole_percent(plan.fill_probability)
        ),
        format!(
            "Seats open in this window: {} of past sections",
            whole_percent(plan.availability)
        ),
        format!(
            "Price trend: {} / year ({trend_note})",
            signed_percent(plan.trend_per_year)
        ),
        format!("Typical clearing price when it fills: {clearing}"),
        format!("Clearing price range when it fills: {band}"),
    ]
}

/// The chance of a seat at each bid asked about. A bid the plan could not price shows "-".
fn checked_lines(planned: &Planned) -> Vec<String> {
    let bids: Vec<String> = planned.bids.iter().map(|&b| money(b)).collect();
    let bid_w = bids.iter().map(String::len).max().unwrap_or(0).max("YOUR BID".len());
    let mut lines = vec![format!("{:<bid_w$}  CHANCE", "YOUR BID")];
    for (i, bid) in bids.iter().enumerate() {
        // The query layer answers each bid in order, so the i-th check belongs to the i-th bid.
        let chance = planned
            .plan
            .bid_checks
            .get(i)
            .map_or_else(|| "-".to_string(), |c| pct(c.probability));
        lines.push(format!("{bid:<bid_w$}  {chance}"));
    }
    lines
}

/// The past sections behind the forecast, with the share of the forecast each one carries.
fn observations_table(plan: &BidPlan) -> String {
    let mut table = Table::new(&[
        "TERM",
        "SECTION",
        "INSTRUCTOR",
        "OUTCOME",
        "MIN BID",
        "MEDIAN BID",
        "WEIGHT",
    ])
    .right_align(&[4, 5, 6]);
    for o in &plan.observations {
        table.push(vec![
            o.term.clone(),
            o.section.clone(),
            o.instructor.clone(),
            (if o.filled { "Filled" } else { "Seats left" }).to_string(),
            price_or_dash(o.min_bid),
            price_or_dash(o.median_bid),
            pct(o.weight_share),
        ]);
    }
    table.render()
}

/// Money, or "-" when the section has no bid (the data stores zero then).
fn price_or_dash(x: f64) -> String {
    if x > 0.0 {
        money(x)
    } else {
        "-".to_string()
    }
}

/// A chance as a short percentage: 0.5 is "50%" and 0.975 is "97.5%".
fn chance_label(p: f64) -> String {
    let text = format!("{:.2}", p * 100.0);
    format!("{}%", text.trim_end_matches('0').trim_end_matches('.'))
}

/// A fraction as a whole percentage: 0.63 is "63%".
fn whole_percent(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// A change as a signed whole percentage: 0.08 is "+8%". Zero has no sign.
fn signed_percent(x: f64) -> String {
    let v = (x * 100.0).round();
    if v == 0.0 {
        "0%".to_string()
    } else {
        format!("{v:+.0}%")
    }
}

/// "1 term", "6 terms".
fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// "a", "a and b", "a, b and c".
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn is_suggested(p: f64) -> bool {
    (p - SUGGESTED_CHANCE).abs() < 1e-9
}

/// Collapses runs of whitespace, newlines included, so a name or title stays on one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ErrorKind;
    use crate::data::filter::GlobalFilters;
    use crate::data::model::ModelParams;
    use crate::data::{FileReport, TermKind};
    use crate::ui::test_data::sample_data;

    /// TEST101 over seven terms, 2022 T1 to 2025 T1. Window 1 has four sections a term (ALPHA, BETA,
    /// ALPHA and GAMMA), and window 2 has one. The default target is 2025-26 T2.
    fn ctx() -> Ctx {
        Ctx {
            data: sample_data(),
            filters: GlobalFilters::default(),
            params: ModelParams::default(),
            json: false,
        }
    }

    fn args(code: &str) -> PlanArgs {
        PlanArgs {
            code: code.to_string(),
            window: "R1W1".to_string(),
            instructor: None,
            target: None,
            bid: Vec::new(),
            levels: Vec::new(),
            observations: false,
        }
    }

    fn rejected(args: &PlanArgs) -> CliError {
        match build(&ctx(), args) {
            Ok(_) => panic!("{args:?} should be rejected"),
            Err(err) => err,
        }
    }

    #[test]
    fn levels_default_then_sort_and_drop_repeats() {
        assert_eq!(chances(&[]), DEFAULT_LEVELS.to_vec());
        assert_eq!(chances(&[0.9, 0.5, 0.8, 0.5]), vec![0.5, 0.8, 0.9]);
        // Two chances that print the same at four decimals are one level.
        assert_eq!(chances(&[0.8, 0.80004]), vec![0.8]);
    }

    #[test]
    fn bids_must_be_finite_and_not_negative() {
        assert_eq!(checked_bids(&[25.0, 0.0]).expect("valid"), vec![25.0, 0.0]);
        let zero = checked_bids(&[-0.0]).expect("zero is fine");
        assert!(zero[0].is_sign_positive(), "negative zero should be written as zero");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -5.0, -0.01] {
            let err = checked_bids(&[bad]).expect_err("should be rejected");
            assert_eq!(err.kind, ErrorKind::Usage, "{bad}");
        }
        assert!(checked_bids(&[-5.0]).expect_err("negative").message.contains("-5"));
    }

    #[test]
    fn bids_under_the_minimum_are_warned_about() {
        assert!(bid_warnings(&[10.0, 25.0]).is_empty());
        let one = bid_warnings(&[9.0]);
        assert_eq!(one.len(), 1, "{one:?}");
        assert!(
            one[0].contains("e$9.00") && one[0].contains("e$10.00") && one[0].contains("0%"),
            "{one:?}"
        );
        let many = bid_warnings(&[9.0, 5.0, 9.0]);
        assert_eq!(many.len(), 1, "{many:?}");
        assert!(many[0].contains("e$5.00 and e$9.00"), "{many:?}");
    }

    #[test]
    fn chances_outside_the_priced_range_are_flagged() {
        assert!(level_warnings(&DEFAULT_LEVELS).is_empty());
        assert!(level_warnings(&[0.01, 0.995]).is_empty());
        let flagged = level_warnings(&[0.005, 0.999]);
        assert_eq!(flagged.len(), 2, "{flagged:?}");
        assert!(
            flagged[0].contains("0.5%") && flagged[0].contains("for 1%"),
            "{flagged:?}"
        );
        assert!(
            flagged[1].contains("99.9%") && flagged[1].contains("for 99.5%"),
            "{flagged:?}"
        );
    }

    #[test]
    fn chance_and_change_labels_read_cleanly() {
        assert_eq!(chance_label(0.5), "50%");
        assert_eq!(chance_label(0.8), "80%");
        assert_eq!(chance_label(0.975), "97.5%");
        assert_eq!(chance_label(0.995), "99.5%");
        assert_eq!(whole_percent(0.63), "63%");
        assert_eq!(signed_percent(0.08), "+8%");
        assert_eq!(signed_percent(-0.1), "-10%");
        assert_eq!(signed_percent(-0.0001), "0%");
        assert_eq!(plural(1, "term"), "1 term");
        assert_eq!(plural(6, "term"), "6 terms");
    }

    #[test]
    fn a_target_already_in_the_data_is_noted() {
        let latest = Some(Term {
            year: 2025,
            kind: TermKind::T1,
        });
        assert!(past_target_note(
            Term {
                year: 2025,
                kind: TermKind::T1
            },
            latest
        )
        .is_some());
        assert!(past_target_note(
            Term {
                year: 2024,
                kind: TermKind::T2
            },
            latest
        )
        .is_some());
        assert!(past_target_note(
            Term {
                year: 2025,
                kind: TermKind::T2
            },
            latest
        )
        .is_none());
        assert!(past_target_note(
            Term {
                year: 2024,
                kind: TermKind::T2
            },
            None
        )
        .is_none());
    }

    #[test]
    fn default_plan_targets_the_next_term_and_marks_the_eighty_percent_level() {
        let planned = build(&ctx(), &args("test101")).expect("the course exists");
        assert_eq!(planned.plan.course, "TEST101");
        assert_eq!(planned.plan.target, "2025-26 T2");
        // Every term before the target is used: 7 terms, 4 window-1 sections each.
        assert_eq!((planned.plan.n_terms, planned.plan.n_observations), (7, 28));
        assert_eq!(planned.plan.recommended.len(), DEFAULT_LEVELS.len());
        assert!(planned.plan.observations.is_empty(), "observations need --observations");
        assert!(planned.plan.warnings.is_empty(), "{:?}", planned.plan.warnings);

        let text = render(&planned);
        assert!(text.is_ascii(), "{text}");
        assert!(text.contains("CHANCE  BID"), "{text}");
        assert!(text.contains("Confidence: "), "{text}");
        let suggested: Vec<&str> = text.lines().filter(|l| l.contains("<- suggested")).collect();
        assert_eq!(suggested.len(), 1, "{text}");
        assert!(suggested[0].starts_with("80%"), "{}", suggested[0]);
        assert!(!text.contains("YOUR BID"), "no bids were asked about");
    }

    #[test]
    fn bids_are_checked_and_warned_when_under_the_minimum() {
        let mut a = args("TEST101");
        a.bid = vec![25.0, 5.0];
        let planned = build(&ctx(), &a).expect("valid");
        assert_eq!(planned.plan.bid_checks.len(), 2);
        assert_eq!(planned.plan.bid_checks[1].probability, 0.0);
        assert!(
            planned.plan.warnings.iter().any(|w| w.contains("e$5.00")),
            "{:?}",
            planned.plan.warnings
        );

        let text = render(&planned);
        assert!(text.contains("YOUR BID  CHANCE"), "{text}");
        assert!(
            text.lines().any(|l| l.starts_with("e$5.00") && l.ends_with("0.0%")),
            "{text}"
        );
        assert!(
            text.lines().any(|l| l.starts_with("e$25.00") && l.ends_with('%')),
            "{text}"
        );
    }

    #[test]
    fn a_window_without_history_says_so_and_keeps_the_warnings() {
        let mut a = args("TEST101");
        a.window = "R2W1".to_string();
        a.bid = vec![20.0];
        let planned = build(&ctx(), &a).expect("the course exists; the window has no rows");
        assert!(planned.plan.recommended.is_empty());
        assert!(planned.plan.bid_checks.is_empty());
        let text = render(&planned);
        assert!(text.contains("No bid can be recommended"), "{text}");
        assert!(
            planned.plan.warnings.iter().any(|w| w.contains("has no results for Round 2 Window 1")),
            "{:?}",
            planned.plan.warnings
        );
        assert!(!text.contains("Warnings:"), "warnings go to stderr, not the table: {text}");
        assert!(
            text.lines().any(|l| l.starts_with("e$20.00") && l.ends_with('-')),
            "{text}"
        );
        assert!(!text.contains("Confidence"), "{text}");
    }

    #[test]
    fn a_past_target_is_planned_from_the_terms_before_it() {
        let mut a = args("TEST101");
        a.target = Some("2025-26 T1".to_string());
        let planned = build(&ctx(), &a).expect("a past target is allowed");
        assert_eq!(planned.plan.target, "2025-26 T1");
        // Only 2022 T1 to 2024 T2 are before the target: 6 terms, 4 window-1 sections each.
        assert_eq!((planned.plan.n_terms, planned.plan.n_observations), (6, 24));
        let note = "2025-26 T1 is not after the latest term in the data";
        assert!(
            planned.plan.warnings.iter().any(|w| w.starts_with(note)),
            "{:?}",
            planned.plan.warnings
        );
        // The JSON view carries the same warning, so a script sees it without the text view.
        let value = serde_json::to_value(&planned.plan).expect("serialises");
        assert!(value["warnings"][0].as_str().is_some_and(|w| w.starts_with(note)), "{value}");
    }

    #[test]
    fn a_data_file_that_could_not_be_read_is_the_first_warning() {
        let mut data = sample_data();
        data.report.files.push(FileReport {
            name: "2024-25_T1.xlsx".to_string(),
            term: None,
            rows_kept: 0,
            rows_special_rounds: 0,
            rows_invalid: 0,
            duplicates: 0,
            error: Some("bad zip".to_string()),
        });
        let ctx = Ctx {
            data,
            filters: GlobalFilters::default(),
            params: ModelParams::default(),
            json: false,
        };
        let planned = build(&ctx, &args("TEST101")).expect("valid");
        assert!(
            planned.plan.warnings[0].starts_with("2024-25_T1.xlsx could not be read"),
            "{:?}",
            planned.plan.warnings
        );
    }

    #[test]
    fn observations_are_listed_only_when_asked() {
        let mut a = args("TEST101");
        a.observations = true;
        let planned = build(&ctx(), &a).expect("valid");
        assert_eq!(planned.plan.observations.len(), 28);
        let text = render(&planned);
        assert!(text.is_ascii(), "{text}");
        assert!(text.contains("Past sections used:"), "{text}");
        assert!(
            text.lines()
                .any(|l| l.starts_with("TERM") && l.contains("SECTION") && l.contains("WEIGHT")),
            "{text}"
        );
    }

    #[test]
    fn instructor_names_resolve_and_bad_inputs_have_the_right_kind() {
        let mut a = args("TEST101");
        a.instructor = Some("al".to_string());
        let planned = build(&ctx(), &a).expect("one match");
        assert_eq!(planned.plan.instructor.as_deref(), Some("ALPHA"));
        assert!(render(&planned).contains("instructor: ALPHA"));

        // ALPHA, BETA and GAMMA all contain an "a".
        a.instructor = Some("a".to_string());
        assert_eq!(rejected(&a).kind, ErrorKind::NotFound);
        a.instructor = Some("zed".to_string());
        assert_eq!(rejected(&a).kind, ErrorKind::NotFound);
        a.instructor = None;

        assert_eq!(rejected(&args("NOPE")).kind, ErrorKind::NotFound);
        let mut bad_window = args("TEST101");
        bad_window.window = "R9W9".to_string();
        assert_eq!(rejected(&bad_window).kind, ErrorKind::Usage);
        let mut bad_target = args("TEST101");
        bad_target.target = Some("next year".to_string());
        assert_eq!(rejected(&bad_target).kind, ErrorKind::Usage);
        let mut bad_bid = args("TEST101");
        bad_bid.bid = vec![-5.0];
        assert_eq!(rejected(&bad_bid).kind, ErrorKind::Usage);
    }

    #[test]
    fn all_windows_gets_the_warning_to_pick_one() {
        let mut a = args("TEST101");
        a.window = "all".to_string();
        let planned = build(&ctx(), &a).expect("valid");
        let text = render(&planned);
        assert!(text.contains("All rounds | bidding for"), "{text}");
        assert!(
            planned.plan.warnings.iter().any(|w| w.contains("pick a specific round and window")),
            "{:?}",
            planned.plan.warnings
        );
    }

    #[test]
    fn json_is_the_plan_with_an_empty_observations_list_by_default() {
        let planned = build(&ctx(), &args("TEST101")).expect("valid");
        let value = serde_json::to_value(&planned.plan).expect("serialises");
        for key in [
            "course",
            "target",
            "recommended",
            "bid_checks",
            "fill_probability",
            "confidence",
            "observations",
            "warnings",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["observations"], serde_json::json!([]));
        assert_eq!(value["recommended"][2]["probability"], 0.8);
    }

    #[test]
    fn run_prints_both_views() {
        run(&ctx(), &args("TEST101")).expect("text view");
        let mut json = ctx();
        json.json = true;
        run(&json, &args("TEST101")).expect("json view");
    }
}
