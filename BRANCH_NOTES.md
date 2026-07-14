# Upstream candidates — fixes to send to `ironcalc/IronCalc` main

Written 2026-07-13, updated after PR #1 (`perf-scoped-recompute` → `data-tables-impl`).
Everything else this branch carried (the scoped-recompute API, phased
`evaluate()`, the iterative-convergence fix, tracing/counters) is
`data-tables-impl`-specific plumbing and is not relevant to plain upstream
`main` — it's covered by that PR, not here. The two fixes below are
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

## Suggested next step

Open two small PRs against `ironcalc/IronCalc` main: the YEARFRAC swap, and
the sheet-name quoting fix (optionally bundled with f56665e3's parse-side
fix). Rebase risk note: on the next merge of upstream main into
`data-tables-impl`, these are the only hunks living in files upstream
actively evolves independent of the data-tables work — upstreaming them
first means they return via the merge and the branch copies collapse to
no-ops.
