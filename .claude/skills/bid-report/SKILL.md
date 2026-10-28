---
name: bid-report
description: Look up SMU BOSS bid statistics for one or more courses with the smu_bid_analyser CLI - the median, 10th and 90th percentile clearing prices, the bid needed for a given chance of a seat, fill rate, confidence and per-term history. Use when the user types /bid-report or asks what a course "goes for", how much to bid, or for bid percentiles/ranges for a course, instructor or window.
argument-hint: "<course or search words> [window, e.g. R1W1] [instructor]"
---

# BOSS bid report

Answer with figures from the `smu_bid_analyser` CLI in this repo. The full reference is
`LLM_GUIDE.md` (or `smu_bid_analyser guide`); read the relevant section there if anything below is unclear.

## Running the tool

From the repo root, use the first that works:
1. `./target/release/smu_bid_analyser.exe` (build once with `cargo build --release -q` if missing or stale)
2. `cargo run --release -q -- <args>`

Rules:
- **Always pass a subcommand and `--json`.** Without a subcommand the desktop GUI opens and blocks.
- Parse stdout only. Exit codes: 0 ok, 1 data folder failed to load, 2 bad usage, 3 not found/ambiguous.
  On error, read `error.suggestions`; if a course or instructor is not found, show the suggestions and ask -
  never silently swap in a similar code.
- Pass `--data-dir PATH` only if the user names a folder; otherwise the tool finds the data itself.
- Run several courses as separate commands in parallel.

## Steps

1. **Parse the request** (`$ARGUMENTS`): course(s), window, instructor, chances or bids of interest.
   - No window given: use `R1W1` and say so. Always use one specific window (`R1W1`, `R1AW2`, `R2W1`...),
     never `R2` or `all`, for prices.
2. **Resolve the course** if the code is not exact: `--json search "<words>" --limit 5`.
   Check `last_term` against `info`'s `latest_term`; flag a course not offered recently.
3. **Get the report**:
   ```
   smu_bid_analyser --json plan CODE --window W --levels 0.5,0.8,0.9 [--instructor "NAME"] [--bid X ...]
   ```
   Key JSON fields:
   - `clearing_quantiles` `{p10, p25, p50, p75, p90}` - percentiles of the clearing price (lowest winning bid)
     **given the section fills**, weighted to the target term. `null` if no past section filled.
   - `median_clearing` - same as `p50`.
   - `recommended[]` `{probability, bid, is_minimum_bid}` - lowest bid for that chance of a seat, which already
     accounts for sections that don't fill (so the 50% bid is often the e$10 minimum).
   - `bid_checks[]` - chance of a seat for each `--bid`.
   - `fill_probability`, `availability`, `confidence`, `n_observations`, `n_terms`, `n_instructor_obs`, `warnings`.
4. **Per-term history** (include when the user asks for history/trend, or for context on a thin estimate):
   ```
   smu_bid_analyser --json course CODE --window W --terms 4
   ```
   `terms[]` gives each term's `clearing_low`, `clearing_median`, `clearing_high` (over filled sections only),
   `filled`/`available` and `fill_rate`. Add `--sections` for each section's `min_bid` and instructor.
5. **Instructor** given: run `plan` with and without `--instructor` so the user sees the instructor effect. If the
   name is ambiguous (exit 3), list the choices and ask.

## Reporting

Keep it short. For one course:

> **IS216 - Round 1 Window 1, bidding for 2026-27 T2** (`plan`)
> Clearing price when the section fills: 10th e$23.39 · median e$37.27 · 90th e$55.17
> Bid for a seat: 50% e$10 (minimum) · 80% e$38.78 · 90% e$47.70
> Section fills 44% of the time; confidence High (47 results over 6 terms).

For several courses, one table: course, window, p10 / median / p90 clearing, 80% bid, fill %, confidence.

Always:
- Explain the difference once if both appear: percentiles describe the clearing price *if the section fills*;
  the "bid for X% chance" also counts the times it doesn't fill.
- Quote `warnings` and flag Low/Medium confidence, `clearing_quantiles: null` (no past section filled, so the
  bid falls back to reported Min Bids and is very rough), and small `n_instructor_obs`.
- Say these are estimates from past terms, not live BOSS data and not guarantees. The price trend is shown,
  not applied.
- Round when speaking (about e$39), but give the tool's exact value alongside.
- Every number must come from a command you ran.
