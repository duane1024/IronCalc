# Upstream candidates — fixes to send to `ironcalc/IronCalc` main

Written 2026-07-13, updated after PR #1 (`perf-scoped-recompute` → `data-tables-impl`).
Everything else this branch carried (the scoped-recompute API, phased
`evaluate()`, the iterative-convergence fix, tracing/counters) is
`data-tables-impl`-specific plumbing and is not relevant to plain upstream
`main` — it's covered by that PR, not here. The fixes below are
independent, exist on unmodified upstream `main`, and cherry-pick cleanly.

## 1. YEARFRAC reversed-dates fix (`base/src/functions/date_and_time.rs`)

`fn_yearfrac` computed the day count in argument order and took `|result|`
at the end. Excel instead **swaps start/end before** the 30/360
day-of-month adjustments, which are asymmetric (only the start day collapses
31→30 unconditionally). So `YEARFRAC(TODAY(), DATE(2025,12,31))` with today
past that date returned 189/360 vs Excel's 190/360. Fixed with a
`std::mem::swap` on the serials right after parsing; the swap also protects
basis 1's year-range arithmetic, which assumes ordered dates (reversed
multi-year inputs could hit an empty year range). Regression test in
`base/src/test/test_yearfrac_basis.rs` asserts both argument orders give
190/360.

Cherry-picks cleanly: `fn_yearfrac` exists on upstream main and the region
is untouched by `data-tables-impl`.

## 2. `name_needs_quoting` allow-list (`base/src/expressions/utils/mod.rs`)

The engine re-stringifies parsed formulas (imported formulas are stored via
this path), and sheet names were quoted per a **block-list** of bad
characters (space, `()'$,;-+{}`) that misses `!`, `&`, `=`, `%`, and most
other characters Excel legally allows in sheet names (Excel forbids only
`\ / ? * [ ] :`). A real workbook (AppLovin) has sheets named `!CAP` and
`SUMM_P&L`; their references came out unquoted (`='!CAP'!E5` became the
unparseable `=!CAP!R[1]C[0]`; `'SUMM_P&L'!H28` became a `#NAME?`),
corrupting 122 of the book's display cells on any recompute — invisible in
viewer mode because cached values render fine.

Fix inverts to an **allow-list**: only names matching the
unquoted-identifier grammar (letter/`_` first, then letters/digits/`_`/`.`)
may skip quotes; everything else gets quoted, which is always legal. The
existing A1/R1C1-lookalike checks are retained. Regression tests: new
`quote_name` cases (`!CAP`, `SUMM_P&L`, `A=B`, `P%L`, and `SUMM_P.L`
staying unquoted) plus an end-to-end evaluate through both sheet names in
`base/src/test/test_general.rs`.

Cherry-picks cleanly: `quote_name`/`name_needs_quoting` exist on upstream
main unmodified by `data-tables-impl`. It pairs conceptually with
`f56665e3` on `data-tables-impl` (`parse_reference` last-`!` split): that
fixed the *import/parse* side, this fixes the *stringify* side. Upstream
would ideally get both together, but f56665e3 touches
`xlsx/src/import/worksheets.rs`, which `data-tables-impl` also modified for
data tables, so extracting it needs a small manual cherry-pick.

## 3. Clip whole-column/row CF ranges to the used dimension (`base/src/conditional_formatting.rs`)

`evaluate_conditional_formatting` scanned each rule's range **literally as
declared in the file** — `collect_numeric_values` and every `apply_cf_*`
loop `for row in r1..=r2 { for col in c1..=c2 { … } }` over it. Conditional
formatting applied to a whole column (Excel's "apply to whole column" UI
action emits e.g. `D173:M1048576`) is a completely ordinary, common
declaration, but it makes that loop enumerate all ~1,048,576 rows regardless
of how much data the sheet actually has. One real analyst workbook declared
two such whole-column rules across 11 total CF rules — 29,360,257 covered
cells, ~59M `get_cell_value_by_index` calls and a `cf_cache` entry per
matching cell per evaluation pass: 2.53 GB / 26 s to open (vs. 0.02–0.07 GB
/ 2 s for comparable books without whole-column rules); in production it
was 7.26 GB / 344 s and OOM-killed the host.

Fix: a new pure helper, `clip_ranges_to_dimension(ranges, dimension)`, clips
a range's trailing edge (`row2`/`col2`) down to `Worksheet::dimension()`'s
used extent, but **only on an axis whose declared bound already reaches the
sheet's absolute ceiling** (`LAST_ROW`/`LAST_COLUMN`) — i.e. only a genuine
whole-column/row declaration, not any ordinary bounded range that happens to
extend past the currently-used cells. `evaluate_conditional_formatting`
calls it once per sheet, right after `parse_sqref`, so every rule type
inherits it with no changes to the individual `apply_cf_*` functions. A
sheet with zero cells anywhere is skipped outright (rather than evaluated
against `dimension()`'s `(1,1,1,1)` empty-sheet placeholder), so a rule
declared before any data exists produces no results, same as before.

The obvious broader version — clip every declared range to the used
dimension unconditionally — is unsound and was caught by the existing test
suite while building this fix: `Blanks`/`NoErrors`/content-independent
`Formula` rules key off the *absence* of content, so they legitimately match
blank cells a user deliberately left inside an ordinary bounded range past
the last populated row (breaks `test_blanks_applies_to_empty_cells`); and a
multi-area formula rule's relative-formula anchor is the bounding box of
*all* its areas, so dropping one area because it doesn't overlap the used
dimension shifts that anchor and corrupts every other area's evaluation
(breaks `multi_area_formula_anchor_is_min_row_min_col`). Scoping the clip to
axes that reach the sheet's absolute ceiling sidesteps both: an ordinary
range is never touched, and a whole-column/row area is always the only area
in its range (`parse_sqref` keeps space-separated areas distinct), so there
is no anchor to shift.

Pure performance/memory fix, no API change, no behavior change for any rule
whose result depends on the cell's own value (the overwhelming majority of
rule types, and the shape of the reported production regression) — clean
PR to offer upstream. Regression tests: unit tests on
`clip_ranges_to_dimension` directly (`base/src/conditional_formatting.rs`'s
`mod tests`), plus engine-level tests in
`base/src/test/conditional_formatting/range_clipping.rs` covering the
visible-result-unchanged + bounded-cache-size cases, the empty-sheet guard,
and the ordinary-bounded-Blanks-rule scope guard.

## Suggested next step

Open three small PRs against `ironcalc/IronCalc` main: the YEARFRAC swap,
the sheet-name quoting fix (optionally bundled with f56665e3's parse-side
fix), and the CF whole-column/row range clipping. Rebase risk note: on the
next merge of upstream main into `data-tables-impl`, these are the only
hunks living in files upstream actively evolves independent of the
data-tables work — upstreaming them first means they return via the merge
and the branch copies collapse to no-ops.
