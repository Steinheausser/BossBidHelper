# BOSS Bid Helper: guide for LLM agents

For an agent (Claude Code, or any model with a shell) helping a student decide what to bid in SMU's
BOSS course-bidding system. Money is in e-dollars (e$). The tool turns past BOSS results into
estimates: how much a bid must be to reach a chosen chance of a seat, how likely a given bid is to
win, and how instructors and prices compare. These are statistical estimates from past terms, not
guarantees. The tool has no live BOSS data.

## 1. What it is and how to run it

Use it for bidding questions: how much to bid for a course, the chance of a given bid, which
instructors or sections are hard to get into, whether prices are rising, and how well the model has
done. Each command reads a folder of BOSS exports, then prints a table, or JSON with `--json`.

The examples use `smu_bid_analyser`. If that is not on PATH, use one of these instead:
- `target/release/smu_bid_analyser.exe` in the repo (Git Bash: `./target/release/smu_bid_analyser.exe`), after `cargo build --release`
- `cargo run --release -q -- --json info` (the `--` separates cargo's options from the tool's; `-q` keeps cargo's messages off the output)
- or install it once with `cargo install --path .`, which puts `smu_bid_analyser` on PATH

Typical first run:

```
smu_bid_analyser --json info
smu_bid_analyser --json search is216
smu_bid_analyser --json plan IS216 --window R1W1
```

The data folder is the first of these that applies:
1. `--data-dir <PATH>` (before or after the command)
2. the `BOSS_DATA_DIR` environment variable
3. the folder saved by the desktop app, in `config.json` under `%APPDATA%\smu_bid_analyser\`
4. `%USERPROFILE%\OneDrive - Singapore Management University\School Documents\Bidding and School Guides\Bidding\Boss Overall Resultsa`
5. `./data`

It holds one BOSS "Overall Results" export per term, unchanged, named like `2025-26_T2.xlsx`. `.xls`,
`.xlsx`, `.xlsm`, `.xlsb` and `.ods` are read; `~$` lock files are skipped. The repo's `data/` folder
holds only old parquet files, which are ignored, so pointing at it exits 1. Run `info` first to see
what was loaded. Commands take about half a second.

## 2. Ground rules

1. Always name a subcommand and pass `--json`, then parse stdout only. Do not parse the text tables;
   their columns and wording may change. With `--json`, stdout is one JSON document on success and on error.
2. Never run the tool without a subcommand. `smu_bid_analyser`, a folder path, `--data-dir` alone and the
   legacy `--course`/`--tab` flags all open a desktop window and block, and so does `gui`. With no subcommand,
   `--json` or `--exclude-session` is refused with exit 2 instead, so a missing subcommand fails at once.
3. Exit codes: `0` success (an empty result is still 0); `1` the data folder could not be loaded;
   `2` invalid usage (bad flag or value, window, term or sort; a chance not between 0 and 1; a negative bid;
   `--json` or `--exclude-session` with no subcommand); `3` not found or ambiguous (course, instructor, session,
   school or term).
4. Errors: with `--json`, stdout gets `{"error": {"kind": "usage" | "not_found" | "data", "message": "...", "suggestions": [...]}}`.
   In text mode, stderr gets `error: ...` and `did you mean: ...`. Use `suggestions` to correct the input
   and retry. If a code or name is not found, ask the student or use a listed suggestion. Never map it to a similar one silently.
5. Text mode: stdout is the table. `Loading ...` and `warning: ...` lines go to stderr. With `--json`, stderr
   stays empty and everything is in the document: read the `warnings` array (empty means none). Every command
   that can warn has one: `info`, `search`, `course`, `plan`, `instructors`, `hot` and `backtest`.
6. Course codes ignore case, spaces and hyphens in every command that takes one, `search` included: "is 216"
   and "cor comm1304" find IS216 and COR-COMM1304. If several codes collapse to the same text, the command exits
   3 and lists them in `suggestions`. If a code is unclear, `search` it first.
7. `--instructor` takes an exact name (any case) or a part of a name that matches exactly one
   instructor of that course. Several matches give exit 3 with the choices in `suggestions`. Names are
   as the data stores them after cleaning (see section 7).
8. Terms are written `2026-27 T2` (T1, T2, T3A, T3B). `plan --target` accepts any term, including
   future ones. `hot --term` must be a term in the data.
9. Windows: `R1W1` (Round 1, Window 1), `R1AW2`, `1:1`, `"Round 1 Window 1"`, `R2` (every window of
   Round 2) and `all` (every round and window together). Use one specific window for any price answer (section 4).
10. Chances are fractions in JSON and in `--levels`: `0.8` means 80%. `--levels 80` exits 2. Text
    tables print percentages.
11. Money is in e$, and JSON numbers are in e$. `null` means "no value" (for example, no filled
    section), not zero. A `min_bid` or `median_bid` of 0.0 means BOSS reported none.
12. Every figure you give a student must come from a command you ran. Say which command it came from.

## 3. Command reference

Global flags work before or after the command: `--json`, `--data-dir PATH`, and `--exclude-session NAME`
(repeatable and case-insensitive; it removes that session's rows from every result).

### info

What was loaded: files, rows, courses, terms, the latest term, the term `plan` bids for by default,
sessions and schools.

    smu_bid_analyser --json info

JSON: `data_dir`, `files` (`name`, `term`, `rows_kept`, `rows_special_rounds`, `rows_invalid`,
`duplicates`, `error`), `total_rows`, `courses` (a count), `terms` (oldest first), `latest_term`,
`default_target_term`, `sessions`, `schools`, `load_millis`, `excluded_sessions`, `warnings`.

Gotchas: `rows_special_rounds` counts the Incoming Freshmen and Incoming Exchange rows, which are
dropped on load. `total_rows` ignores `--exclude-session`. `warnings` names any file that could not be
read; its rows are left out.

### search

Finds the exact course code, title, school and last term offered.

    smu_bid_analyser --json search "data science" --limit 5
    smu_bid_analyser --json search is216 --school SIS

Flags: `--school NAME` (case-insensitive), `--limit N` (default 20). With no words, it lists every course.

JSON: `query`, `school`, `limit` (as given), `total_matches` (a fuzzy query stops counting at 200), `shown`,
`results` (each with `code`, `description`, `school`, `last_term`), and `warnings`.

Gotchas: matching is fuzzy and ranked, with exact code prefixes first. Hyphens and spaces are ignored, so
`search "cor comm1304"` finds COR-COMM1304. Titles are complete in JSON but cut to 50 characters in text. `last_term` is the last term the course appeared in. If it is before
`info`'s `latest_term`, the course was not offered in the latest term.

### course

A course's history in one window: each term's fill rate and clearing prices. Add `--sections` for
every section.

    smu_bid_analyser --json course IS216 --window R1W1
    smu_bid_analyser --json course IS216 --window R1W1 --sections --terms 3

Flags: `--window W` (default R1W1), `--sections`, `--terms N` (the N most recent terms).

JSON: `code`, `description`, `school`, `last_term`, `window`, `window_code`, `windows_offered` (every
window the course has been in, in bidding order), `instructors` (the full list, most recent first),
`excluded_sessions`, `terms_limit` (the N given to `--terms`, or null), `terms` (oldest first: `term`, `sections`,
`section_windows`, `available`, `filled`, `fill_rate`, `seats_open`, `seats_left`, `clearing_median`, `clearing_low`,
`clearing_high`, `median_bid`), `sections` (null unless `--sections`; otherwise per section: `term`, `session`,
`window_code`, `section`, `instructor`, `vacancy`, `before_proc`, `after_proc`, `enrolled`, `filled`, `available`,
`min_bid`, `median_bid`, newest term first), and `warnings`.

Gotchas: in `terms`, `sections` counts each section once per term. `section_windows` counts every
section-window in the term, open or not. `available` counts the section-windows with seats open (text:
OPEN WINDOWS), and `filled` counts those that filled (text: FILLED WINDOWS). `fill_rate` is filled over
available. `seats_open` sums the seats offered in the open section-windows, and `seats_left` the seats left
after bidding. A section open in two windows counts twice in the last three. The text view puts a note
under the table explaining these units when `--window` is not one round and window. `clearing_*` use
filled sections only. A window with no rows is exit 0 with `terms: []` and a warning (text prints it on stderr).
`--window all`, or a round alone such as `R2`, pools clearing prices from different windows into one median,
and prices differ between windows. The `warnings` array says so; use one specific window for prices.

### plan

The main command. For one course, window and target term: the bid for each chance of a seat, the
chance of any bids you name, and the evidence behind them.

    smu_bid_analyser --json plan IS216 --window R1W1
    smu_bid_analyser --json plan IS216 --window R1W1 --instructor "KYONG JIN SHIM" --bid 30 --bid 40
    smu_bid_analyser --json plan IS216 --window R1W1 --levels 0.8,0.9 --target "2026-27 T2" --observations

Flags: `--window W` (default R1W1); `--instructor NAME`; `--target TERM` (default: the term after the
latest regular term in the data, which `info` reports as `default_target_term`); `--bid E$`
(repeatable, zero or more); `--levels P,P,...` (fractions between 0 and 1; default
0.5,0.7,0.8,0.9,0.95); `--observations` (fills the JSON `observations` list and adds a table in text).

JSON (`BidPlan`): `course`, `description`, `window`, `window_code`, `instructor` (the resolved name, or
null), `target`, `recommended` (each with `probability`, `bid`, `is_minimum_bid`, in `--levels` order),
`bid_checks` (each with `bid`, `probability`, in `--bid` order), `fill_probability`, `availability`,
`trend_per_year`, `trend_applied`, `median_clearing`, `clearing_quantiles` (`p10`, `p25`, `p50`, `p75`, `p90`:
the clearing price's percentiles if the section fills; null when no past section filled), `confidence` ("Low", "Medium" or "High"; always "Low" when no past section filled),
`n_observations`, `n_terms`, `n_instructor_obs`, `effective_n`, `observations` (`term`, `section`,
`instructor`, `filled`, `min_bid`, `median_bid`, `weight_share`), and `warnings`.

Example (IS216, R1W1, data as of 2026-10-11, shortened):

```
{"course": "IS216", "window_code": "R1W1", "target": "2026-27 T2",
 "recommended": [{"probability": 0.5, "bid": 10.0, "is_minimum_bid": true},
                 {"probability": 0.8, "bid": 38.78, "is_minimum_bid": false}],
 "fill_probability": 0.4376, "availability": 0.8545, "trend_per_year": 0.1176,
 "trend_applied": false, "median_clearing": 37.27, "confidence": "High",
 "n_observations": 47, "n_terms": 6, "n_instructor_obs": 47, "effective_n": 18.55,
 "observations": [], "warnings": []}
```

Gotchas:
- `bid` is the lowest bid, rounded up to the cent, that reaches the chance. `is_minimum_bid: true`
  means the e$10 BOSS minimum already reaches it.
- A bid under e$10 has a 0% chance and gets a warning.
- Chances apply to one section where seats are offered in that window. `availability` is how often they were offered.
- With no history in the window, `recommended` and `bid_checks` are empty and `warnings` says why.
  A window that is not specific (`R2`, `all`) gets a warning, because prices differ between windows.
- `median_clearing` is `null` when no past section filled.
- A `--target` at or before the latest term plans from earlier terms only, and `warnings` says so.
- `warnings` also names a course that was last offered before the latest regular term and before the target:
  "X was last offered in T; it may not be offered in target". The tool cannot see future offerings, so check it.
- When no past section in the window filled, `median_clearing` is null, `confidence` is Low, and the price falls
  back to the reported Min Bids, a loose upper bound. The real risk is mostly whether the section fills.
- With `--exclude-session`, `warnings` says how many of the course's results in the window were left out, and in
  which sessions. When all of them were, there is no history, and `recommended` is empty.

### instructors

Ranks instructors by how much more it cost to get into their sections than into the other sections of
the same course, term and window ("VS PEERS"). Raw bids mostly measure how popular a course is, so use
this instead of raw bids.

    smu_bid_analyser --json instructors --course IS216 --window R1W1
    smu_bid_analyser --json instructors --window R1W1 --min-compared 5 --sort easiest --limit 20
    smu_bid_analyser --json instructors --search shim --window all

Flags: `--course CODE` (one course; `--min-compared` then does not apply); `--window W` (default R1W1);
`--school NAME`; `--search TEXT`; `--min-compared N` (default 10);
`--sort demand|easiest|clearing|sections` (demand puts the hardest to get into first); `--limit N`
(default 30).

JSON: `window`, `window_code`, `course`, `school`, `search`, `sort`, `min_compared` (null with
`--course`), `limit` (as given), `total` (before `--limit`), `shown`, `warnings` (for example, the
mixed-window caveat when the window is not specific), and `instructors` (each with `name`, `school`,
`courses`, `sections`, `fill_rate`, `median_clearing`, `premium`, `compared`, `last_term`).

Gotchas: `premium` is a fraction. 0.4 means their sections cost about 40% more than their peers'. It is
shrunk toward 0 when there are few comparisons, and it is `null` when `compared` is 0 (no peer to
compare with). `sections` and `compared` count section-windows. With a low minimum, a small sample can top
the list: `--min-compared 3` put an instructor with 3 comparisons first at +170% (at the time of writing).
Keep the default minimum, or raise it, for a broad ranking.

### hot

Courses with the highest median clearing price in one term and window.

    smu_bid_analyser --json hot --window R1W1 --limit 25
    smu_bid_analyser --json hot --term "2025-26 T2" --window R1W1
    smu_bid_analyser --json hot --min-filled 1 --limit 50

Flags: `--term TERM` (must be in the data; default: the latest non-summer term); `--window W` (default
R1W1); `--min-filled N` (default 2, at least 1: rank only courses with at least N filled sections);
`--limit N` (default 25).

JSON: `term`, `window`, `window_code`, `min_filled` (as given), `limit` (as given), `total` (courses with at
least `min_filled` filled sections, before `--limit`), `shown`, `warnings`, and `courses` (each with `code`,
`description`, `median_clearing`, `sections_filled`, `sections_available`).

Gotchas: a median from one filled section is one price, so by default a course needs two filled sections
to be ranked. With `--min-filled 1`, a course with `sections_filled: 1` can top the list, so check the counts
before calling a course hot. It ranks by price, not by fill rate.

### backtest

How well the model's bids held up on past terms. Quote it whenever you state a probability.

    smu_bid_analyser --json backtest
    smu_bid_analyser --json backtest --window R1AW2
    smu_bid_analyser --json backtest --all-windows

Flags: `--window W` or `--all-windows` (default R1W1); `--tune` is a grid search over 243 parameter
sets (about 2.5 minutes at the time of writing), text only, for maintainers.

JSON: `scope`, `window_code`, `targets` (the terms checked), `sections` (the sections checked), `warnings`,
and `methods`, each with `name` and `levels` (`level`, `n`, `hit_rate`, `mean_bid`, `mean_pinball`). Method
0 is the current model, method 1 the old v0.1 regression, and method 2 "Last term's highest clearing
price" (its `level` and `mean_pinball` are null).

Meaning: `hit_rate` is the share of sections where a bid at that level would have won a seat. A
calibrated 80% bid wins about 80% of the time. `mean_bid` is the average bid in e$, and `mean_pinball`
is a quantile loss (lower is better).

Gotchas: each target term is predicted from earlier terms only, and the first three regular terms are
history only. Results cover every course in the scope, so they say nothing about one course's own prices.

### guide

Prints this guide: `smu_bid_analyser guide` (Markdown), or `smu_bid_analyser --json guide`
(`{"guide": "..."}`). It needs no data.

## 4. Domain primer

- **Rounds and windows.** BOSS bidding runs in rounds (1, 1A, 1B, 2 and 2A appear in the data), each
  with one or more windows. `windows_offered` in `course` lists the windows a course has been in. The
  tool treats each (round, window) pair as its own market, and prices differ between them. For IS216 at
  the time of writing, the 80% bid was e$10.00 in R2W1, e$27.64 in R1W2, e$38.78 in R1W1 and e$63.52 in R1AW2.
- **Outcome of a section in a window.** Filled: every seat offered in that window was taken
  (`before_proc` above 0 and `after_proc` 0). Seats left: some seats remained (`after_proc` above 0).
  No seats: none were offered (`before_proc` 0).
- **Clearing price.** For a filled section, `Min Bid` is the clearing price, the lowest bid that got a
  seat. Bids at or above it won.
- **Unfilled section.** Seats remained, so any bid of at least e$10 got a seat. Its `Min Bid`, if it has
  one, is only the lowest bid placed, so it says little about price.
- **e$10 minimum.** BOSS accepts no bid below e$10. A lower bid has a 0% chance.
- **Chance of a seat.** For a bid b, the chance is (1 - fill probability) + fill probability x P(clearing
  price <= b). That is why the 50% bid is often exactly e$10: if the section often does not fill, the
  minimum bid already wins about half the time.
- **fill_probability.** The estimated chance that this section fills in this window. It is pulled toward
  the window's fill rate across all courses, so thin data does not give 0% or 100%.
- **availability.** The share of the course's past sections that had any seats in this window.
- **median_clearing.** The typical clearing price if the section fills. `null` when no past section filled.
- **trend_per_year.** The direction of clearing prices per year, fitted to per-term medians and clamped
  to about -26% to +35%. It is shown for information and not applied to bids (`trend_applied` is false).
  It needs several terms with filled sections.
- **confidence.** High: at least 3 terms with a filled section, effective_n of at least 4, and at least
  2 observations from the chosen instructor (or from any instructor when none is chosen). Medium: at
  least 2 terms and effective_n of at least 2. Otherwise Low. Low is also reported whenever no past section
  in the window filled, whatever the counts are.
- **n_observations, n_terms, n_instructor_obs.** The past section-results used, the terms they came
  from, and how many of them the chosen instructor taught.
- **effective_n.** How many equally weighted results the estimate is worth after weighting. Recent terms
  count more (half-life 1.5 years), the same term type (T1 vs T2) counts more than other types, summer
  terms count much less for a regular term, and other instructors' sections count less when one is chosen.
- **weight_share.** Each past result's share of the forecast, in `observations`.
- **Instructor premium.** For each section, its log price relative to the average of the other sections
  of the same course, term and window (an unfilled section counts as e$10). These are averaged per
  instructor and shrunk toward 0.
- **Data shape.** Regular terms have about 12,500 to 14,100 rows each. Summer files have 14 to 135.

## 5. Recipes

A. **"How much should I bid for IS216 with Prof X in Round 1 Window 1?"**
1. Confirm the code: `--json search is216`.
2. Read the history and the instructor list: `--json course IS216 --window R1W1`. Check `last_term`.
3. Price the instructor: `--json plan IS216 --window R1W1 --instructor "X"`. A part of the name is enough.
   If several instructors match, the error lists them. If the name may be stored in more than one form (section 7),
   check `instructors --search` first and run the plan for each form.
4. Report the bid for the chance the student wants. 80% is the suggested default; say so if you chose it.
   Include `confidence`, `n_instructor_obs` and `warnings`.
5. For context, run it again without `--instructor`, because the instructor effect can be large or small.

B. **"I have e$X. What is my chance in each of these three courses?"**
Run `--json plan CODE --window W --bid X` for each course and read `bid_checks[0].probability`. Pass
several `--bid` values to show alternatives. Each chance is for one section. The tool does not know the
student's total budget or how many courses they need.

C. **"Which section or instructor of COURSE is cheapest to get into?"**
1. `--json instructors --course COURSE --window W --sort easiest` lists the lowest premium first. Ignore
   anyone with `compared` of 2 or fewer.
2. For each candidate, run `--json plan COURSE --window W --instructor "NAME" --levels 0.8`. Compare
   `recommended[0].bid` with `n_instructor_obs`.
3. `--json course COURSE --sections --terms 3` shows which sections filled and at what minimum bids.

D. **"Is COURSE getting more expensive?"**
Run `--json course COURSE --window W`. Compare `terms[].clearing_median` from oldest to newest, and read
`filled` alongside it. A median from one or two filled sections is noise, so say so. The `trend_per_year`
from `plan` is only a rough summary.

E. **"Which courses are the most contested right now?"**
`--json hot --window R1W1 --limit 50` ranks the latest regular term that has data (2026-27 T1 at the time
of writing; the next term has no data yet), counting only courses with at least two filled sections (the
default `--min-filled 2`). Compare `sections_filled` with `sections_available`. The tool has no ranking by
fill rate, so say that.

F. **"Plan my whole bid budget across courses."**
Run `plan` for each course with `--levels 0.5,0.8,0.9 --bid <intended bid>`. Then tabulate the course,
window, bid for the chosen chance, the chance at the intended bid, the confidence and the warnings.
Reason about the totals with the student. The tool does not know the total budget, the number of
courses or sections needed, timetable clashes, or which options matter most to the student.

G. **"Are these probabilities any good?"**
Run `--json backtest --window W` for the window the student is bidding in, and quote the row for the chance you used.

H. **Unknown window.** Ask which round and window the student is bidding in. If they do not know, use
R1W1, say so, and show `windows_offered` from `course` so they can check.

## 6. How to present results to a student

- Lead with the figure asked for, and quote the tool's exact value: "IS216 in Round 1 Window 1, bidding
  for 2026-27 T2: about e$39 (planner: e$38.78) for an estimated 80% chance of a seat."
- Give the evidence in one line: "based on 47 past section results over 6 terms; confidence High."
- Give calibration context from a `backtest` you ran, and say it came from `backtest`. At the time of
  writing, the 80% bid in Round 1 Window 1 won a seat in 78.7% of sections (average e$23.45). Across all
  windows it won 84.7%. These are averages across courses, not this course.
- Say what makes the answer rough: Low or Medium confidence, a `warnings` entry, a bid of exactly e$10
  with "no past section filled", few filled sections, or an instructor with a small `compared`.
- Say what is not modelled: the price trend (shown, not applied), the student's budget, timetable clashes,
  whether the course will be offered in the target term, and changes to the course or instructor.
- Avoid false precision. Round when speaking, do not rank instructors on premiums from fewer than 5
  comparisons, and never describe a bid as a guarantee.
- For several courses, use one small table. Keep the answer short: the recommendation, the evidence, and the caveats.

## 7. Limitations and pitfalls

Data
- No live data. The tool knows only the exported terms (`info` shows the latest). It cannot see the
  seats in the current window or the bidding that is happening now.
- Instructor names need care. The loader merges spelling variants it can recognise: "MOK, HENG NGEE" becomes
  "MOK HENG NGEE", and "HAN JIN KYUNG, HAN JIN KYUNG" becomes "HAN JIN KYUNG". It does not merge co-taught
  lists: "SAY GUI DENG, ARNOLD WEE" is a separate instructor from "SAY GUI DENG". Names contain commas, so do
  not split them, and text tables cut long names at 30 characters. To check for leftover variants, run
  `instructors --search SURNAME --window all --min-compared 0 --json` and combine any spellings yourself.
- Summer terms (T3A, T3B) are tiny, so forecasts for them are weak. `hot` and the default target skip them.
- The tool cannot see future offerings. `plan` warns when a course was last offered before the latest regular
  term and before the target. `last_term` from `search` or `course` shows the same thing, against `info`'s
  `latest_term`.
- `--exclude-session` removes history from every result. For example, IS216's R1W1 history is all in the
  Regular Academic Session, so excluding that session leaves no history and `plan` says so.

Model
- It assumes the recent past resembles the future. The trend is reported but not applied, because the
  backtests did not improve when it was applied.
- Thin data gives wide, honest ranges. One to three filled sections is still a guess.
- If no past section in the window filled, the model falls back to the reported Min Bids of unfilled
  sections. Those are the lowest bids placed, not clearing prices. `plan` then reports Low confidence and a
  warning (at the time of writing, LAW4019 in R1W1: an 80% bid of e$10.00, with `median_clearing` null).
  Treat such results as very rough.
- A chance applies to one section where seats are offered. Check `availability`.

Known rough edges (true at the time of writing; check against current output)
- The desktop app's Overview ranks courses with one filled section, where the CLI's `hot` needs two by default.
  Its Bid Planner badge shows the forecast's own confidence, so for a window with no filled section it can show
  Medium where `plan` says Low. Say which one you are quoting.
