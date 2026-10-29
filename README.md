# BossBidHelper
Vibe-coded repo to help with SMU BOSS bidding, in Rust.

It has a desktop app (the default) and a command line for scripts and LLM agents. Both read the same folder of BOSS results.

## Data
Download the BOSS **Overall Results** export for each term and put the files, unchanged, in one folder
(e.g. `2025-26_T1.xlsx`, `2022-23_T3A.xls`). `.xls` and `.xlsx` are read directly (`.xlsm`, `.xlsb` and `.ods` too).
There is no Python/parquet conversion step any more.

The folder is the first of these that applies:
1. `--data-dir <path>` (before or after the command). A bare path as the first argument also works, for older scripts.
2. the `BOSS_DATA_DIR` environment variable (an empty value counts as unset)
3. the folder last chosen in the app (**⚙ Data folder** button), saved in `%APPDATA%\smu_bid_analyser\config.json`, as long as it still exists
4. `~/OneDrive - Singapore Management University/School Documents/Bidding and School Guides/Bidding/Boss Overall Resultsa`, if it exists
5. `./data`, relative to where you run it

Files are cleaned on load: header rows repeated mid-sheet, pagination debris, blank rows, duplicate
rows and the Incoming Freshmen / Incoming Exchange rounds are dropped, and the short column names used
by `Bid History/scrape_bids.py` ("Sect", "Bef Proc", ...) are understood. The Data folder window shows
what was kept or dropped per file.

Instructor names are tidied on load too:
- a name repeated in one entry collapses: "HAN JIN KYUNG, HAN JIN KYUNG" becomes "HAN JIN KYUNG";
- spellings that differ only by case, hyphens or spaces unify to the most common one;
- a two-part "SURNAME, GIVEN" entry becomes the standalone spelling the data already has, in either order: "MOK, HENG NGEE" becomes "MOK HENG NGEE";
- co-teaching lists are kept: "SAY GUI DENG, ARNOLD WEE" is a different instructor from "SAY GUI DENG". Lists of three or more keep their order, so one team can still show up twice.

## Running
The desktop app:
```
cargo run --release                                       # the app
cargo run --release -- gui --course IS216 --tab planner   # open on a course; --tab is overview, planner, instructors, terms or saved
```

## Command line
The same data, for scripts and LLM agents. Build it once, then run `smu_bid_analyser`:
```
cargo build --release                # then run target\release\smu_bid_analyser.exe (Git Bash: ./target/release/smu_bid_analyser.exe)
cargo install --path .               # or install it, so smu_bid_analyser is on your PATH (in ~/.cargo/bin)
cargo run --release -q -- <command>  # or run it from the source; -q hides cargo's build messages
```

| Command | What it does |
| --- | --- |
| `info` | What was loaded: files, rows, courses, terms, and the term you are bidding for |
| `search` | Find courses by code or title, to get the exact code the other commands take |
| `course` | One course's history by term: fill rate and clearing prices (`--sections` for every section) |
| `plan` | The main one: the bid for each chance of a seat, and the chance of any bids you name |
| `instructors` | Instructors ranked by how much harder their sections were to get than their peers' |
| `terms` | How big each term was: courses, sections, instructors, class seats, seats won and left unclaimed, fill rate |
| `hot` | Courses with the highest median clearing price in one term |
| `backtest` | How well the model's bids held up on past terms |
| `guide` | The usage guide for LLMs and scripts (Markdown, or `--json`) |
| `gui` | Opens the desktop app; this is what runs with no command |

Examples, from the data as of 2026-10-11 (output trimmed with `...`):
```
$ smu_bid_analyser search "data science" --limit 5
CODE     TITLE                                               SCHOOL  LAST OFFERED
-------  --------------------------------------------------  ------  ------------
CS105    Statistical Thinking for Data Science               SIS     2025-26 T2
DSA201   Statistical Inference for Data Science              SOE     2026-27 T1
DSA312   Data Science with Python                            SOE     2026-27 T1
...
```
```
$ smu_bid_analyser plan IS216 --window R1W1
IS216  Web Application Development II
Round 1 Window 1 (R1W1) | bidding for 2026-27 T2 | instructor: any section

CHANCE  BID
50%     e$10.00  (minimum bid)
70%     e$30.94
80%     e$38.78  <- suggested
90%     e$47.70
95%     e$54.10
Confidence: High - based on 47 past section-results over 6 terms
Section fills: 44% of the time (if it doesn't, any bid >= e$10 wins)
...
```
```
$ smu_bid_analyser plan IS216 --window R1W1 --instructor shim --bid 30
IS216  Web Application Development II
Round 1 Window 1 (R1W1) | bidding for 2026-27 T2 | instructor: KYONG JIN SHIM

CHANCE  BID
50%     e$10.00  (minimum bid)
80%     e$42.44  <- suggested
...
Confidence: High - based on 47 past section-results over 6 terms (11 with this instructor)
...

YOUR BID  CHANCE
e$30.00   64.6%
```
```
$ smu_bid_analyser course IS216 --terms 3
IS216 - Web Application Development II
...
TERM        SECTIONS  OPEN WINDOWS  FILLED WINDOWS   FILL%  SEATS OPEN  SEATS LEFT  CLEARING MEDIAN      LOW     HIGH  MEDIAN BID
----------  --------  ------------  --------------  ------  ----------  ----------  ---------------  -------  -------  ----------
2024-25 T1        10             9               7   77.8%          11           2          e$39.97  e$25.50  e$54.00     e$40.37
2025-26 T1        11             7               7  100.0%          12           0          e$42.02  e$27.00  e$51.21     e$42.02
2026-27 T1        13            10               2   20.0%          42          19          e$43.01  e$32.99  e$53.03     e$44.31
...
```
```
$ smu_bid_analyser hot --limit 5
#  COURSE   TITLE                                               MEDIAN CLEARING  FILLED
-  -------  --------------------------------------------------  ---------------  ------
1  LAW4039  Community Based Projects in the Real World(SMU-X)           e$68.25     2/2
2  IS469    Generative AI with LLMs: From Development to Ap...          e$60.28     2/2
3  IS216    Web Application Development II                              e$43.01    2/10
4  ACCT336  Valuation                                                   e$41.69     3/3
5  ACCT331  Audit and Assurance                                         e$40.80     2/4
```
`hot` ranks by price, not by how often sections fill, so check `FILLED` before calling a course hot.

Add `--json` (before or after the command) for scripts and agents. Stdout is then one JSON document, errors included, and stderr stays empty. Exit codes: `0` ok (an empty result is still 0); `1` the data folder could not be read; `2` invalid usage; `3` a course, instructor, session, school or term was not found, or was ambiguous. A JSON error looks like `{"error": {"kind": ..., "message": ..., "suggestions": [...]}}`; use one of the suggestions to retry.

**Using it from an LLM agent** (Claude Code etc.): run `smu_bid_analyser guide` (it needs no data), or point the agent at `LLM_GUIDE.md`. Something like "Use the BOSS bid CLI in this repo; read LLM_GUIDE.md first" is enough to start.

The **Terms** tab in the app (and `smu_bid_analyser terms`, with `--school` and `--window`) sizes up each term
across every course: how many courses, sections and instructors there were, the class seats, the seats open in
each section's first bidding window, the seats won through bidding and the seats still unclaimed after the last
window, plus the share of sections that filled and the median clearing price. A section counts once however
many windows it was in, and seats are not counted again in each window they roll over to.

The BOSS Overall Results export has no class days, times or venues, so the app cannot show the timetable.

## How the bid prediction works
For a section that **filled**, `Min Bid` is the clearing price: bids at or above it got a seat. For a section that **did not fill**, any bid of at least e$10 got a seat. So

`P(seat | bid) = P(section doesn't fill) + P(fills) x P(clearing price <= bid)`

Both parts are estimated from the course's past sections in the same round and window, weighting recent terms and the same term type (T1/T2) more, and the chosen instructor's sections more than other sections of the course. The fill chance is shrunk toward that window's fill rate across all courses, and the clearing price distribution is a kernel density whose spread grows when there is little data. The Bid Planner then shows the bid needed for a 50/70/80/90/95% chance and a curve you can try bids against. It gives the same numbers as `plan`.

`smu_bid_analyser backtest` checks the model against past terms. Each regular term after the first three (six at the time of writing, 2023-24 T2 to 2026-27 T1) is predicted using only the terms before it, fill-rate prior included, and every section open in that term is scored against what happened. At the time of writing, in Round 1 Window 1 (4,941 sections) the "80%" bid won a seat 78.7% of the time (average e$23.45) and the "90%" bid 89.1% (average e$29.92). Across all windows (`backtest --all-windows`, 33,160 sections) they won 84.7% (average e$22.33) and 91.1% (average e$32.19). The old linear regression's bands were not calibrated: its "50%" bid won 87.6% of the time in Round 1 Window 1, at an average of e$28.36. The new model has lower quantile loss at every level in both views. Its "50%" bid is often just the e$10 minimum, which wins whenever a section does not fill, so it wins more than half the time (66.7% in Round 1 Window 1).

The Instructors tab ranks instructors by how much more their sections cost than the other sections of the same course, term and window, rather than by raw bids (which mostly reflect how popular the course is). It only ranks instructors with at least 10 comparisons by default (the **Min. comparisons** box), because a few sections can top a short list. The CLI has the same ranking: `smu_bid_analyser instructors --course IS216 --window R1W1`, with `--min-compared` (default 10), `--sort easiest` and `--search` options.

## Development
- `cargo test --release` runs the unit tests and the black-box CLI tests in `tests/cli.rs`. The CLI tests write small synthetic `.xlsx` folders to a temp directory, so no real data is needed.
- `cargo clippy --release --all-targets` is clean.

## Notes
`data_prep/convert_to_parquet.py` and the parquet files in `data/` are from v0.1 and are no longer used.
`Bid History/scrape_bids.py` clicks through and scrapes BOSS tables if they are not released onto OASIS yet.

Feedback always welcome.
Use at your own risk.
