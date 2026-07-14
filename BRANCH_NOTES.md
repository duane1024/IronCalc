# perf-scoped-recompute — branch notes

Written 2026-07-13. This branch carries the engine-side changes behind the
workbook-recompute performance work in baba
(`docs/prototypes/workbook-recompute-harness/PERF.md` there): a driver-edit
refresh of an analyst workbook's mapped display range drops from ~30 s (full
`evaluate()`) to 1–55 ms, bit-exact, across all five firm workbooks tested.

## Branch topology

```
ironcalc/IronCalc main  (upstream; origin/main tracks it)
  └── data-tables-impl  (custom: +1,379 lines / 19 files over main)
        └── perf-scoped-recompute  (this branch: 2 commits, +240/−15, 6 files)
```

`data-tables-impl` contributes **all** of: Excel Data Tables (the `DataTable`
type, `compute_data_tables`, `recompute_with_overrides`), **iterative
calculation** (`<calcPr iterate>` import, `CalcProperties.iterate/
iterate_count/iterate_delta`, `evaluate_workbook_cells_iterative`,
`numeric_snapshot`, `calc_results_converged`, `recompute_cells`,
`recompute_cells_iterative`), and the `!CAP` import-side `parse_reference`
fix (f56665e3). None of those symbols exist on main. Upstream main *does*
already have `mock_time`, `quote_name`, `fn_yearfrac`, and
`evaluate_conditional_formatting`.

## Commit 54cdfa9c — "Perf + correctness for analyst-workbook live edits"

### 1. `Model::recompute_target_cells(&[(sheet, row, col)])` — the headline API (`base/src/model.rs`)

Public wrapper that converts plain tuples to `CellReferenceIndex` and
dispatches to the branch's existing `recompute_cells` (demand-driven cone
evaluation: memo cleared, targets and their transitive precedents evaluated,
results persisted to the grid) or, when the workbook has iterative calc
enabled, `recompute_cells_iterative` (repeats the cone recompute until the
target values converge, warm-starting from current grid values). This is
what turns a driver edit into a 1–55 ms refresh instead of a full evaluate.
It is ~10 lines of new code; the machinery already existed on
`data-tables-impl`, built for data-table scenario sweeps — this change makes
it reachable by an external caller (the baba excel service).

Contract note for callers: cells OUTSIDE the targets' precedent cone keep
their previous values — the caller owns that staleness (include every mapped
display range in the target set; never serve non-target cells as fresh).

**Dependency: entirely `data-tables-impl`.** `recompute_cells`/
`recompute_cells_iterative` don't exist on main, so this cannot be
upstreamed independently — it goes wherever the data-tables/iterative work
goes.

### 2. Phase split: `evaluate_formulas_only()` / `evaluate_data_tables_only()` (`base/src/model.rs`)

`evaluate()` on this branch is three phases (iterative cell sweep → data
tables → conditional formatting). These two public methods expose phases 1
and 2 individually so the caller owns the data-table policy: the analyst
books are saved `calcMode="autoNoTable"`, meaning Excel itself skips table
recomputation on ordinary edits, and a faithful host must reproduce exactly
that (data tables in these books cost seconds to tens of seconds; see the
baba PERF.md attribution tables).

**Dependency: `data-tables-impl`.** On main, `evaluate()` has no iterate
loop and no data-table phase, so the split is meaningless there. If upstream
ever takes the data-tables work, these should ride along.

### 3. Convergence fix in `calc_results_converged` (`base/src/model.rs`) — a behavior fix, not just plumbing

The branch's target-convergence test treated any non-numeric result as "not
yet converged", so a target set containing labels/blanks/errors (any real
display range) always ran to `iterate_count` = 100 passes. Now stable
non-numeric pairs (equal strings, equal booleans, both-empty, same error
kind) count as converged; a pair that changes kind still blocks convergence.
Measured effect on the Core & Main book: scoped edit 167 ms → 27 ms — and,
more importantly, it stops masking genuine non-convergence.

**Dependency: `data-tables-impl`** (the function is branch-only). Note this
also changes behavior for the branch's *data-table* iterative solves: a
scenario whose output cell is text/error now converges early instead of
burning 100 passes. Strictly better, but it is a semantic change to branch
code; the five-book certification in baba re-ran green after it.

### 4. YEARFRAC reversed-dates fix (`base/src/functions/date_and_time.rs`) — genuine upstream-main bug

`fn_yearfrac` computed the day count in argument order and took `|result|`
at the end. Excel instead **swaps start/end before** the 30/360
day-of-month adjustments, which are asymmetric (only the start day collapses
31→30 unconditionally). So `YEARFRAC(TODAY(), DATE(2025,12,31))` with today
past that date returned 189/360 vs Excel's 190/360 — the entire residual
6-cell certification gap on the Core & Main workbook. Fixed with a
`std::mem::swap` on the serials right after parsing; the swap also protects
basis 1's year-range arithmetic, which assumes ordered dates (reversed
multi-year inputs could hit an empty year range). Regression test in
`base/src/test/test_yearfrac_basis.rs` asserts both argument orders give
190/360.

**Dependency: none — `fn_yearfrac` exists on upstream main and the region is
untouched by `data-tables-impl`; cherry-picks cleanly. Strongest
upstream-PR candidate.**

### 5. Instrumentation (env-gated, zero overhead when off) (`base/src/model.rs`, `base/src/data_table.rs`)

- `IRONCALC_TRACE=1` tracing: per iterate pass (sweep time, snapshot time,
  numeric-cell count, changed-cell count and max |Δ| via a new
  `snapshot_drift` helper), per data table (range, output count, scenario
  time, settle time), and per scoped recompute (target count, passes to
  convergence).
- Per-sheet formula-evaluation counters: a thread-local
  `Option<HashMap<u32, u64>>` bumped at the memo-miss point in
  `evaluate_cell` (actual evaluations, not memoized reads), exposed as
  `Model::eval_counting_start()` / `eval_counting_take()`. When counting is
  off, the cost is one thread-local `Option` check.

**Dependency: mixed.** The counters and the one-line `evaluate_cell` hook
would apply to main, but everything they measure (iterate passes, tables) is
branch-only; treat all instrumentation as branch-resident debug tooling.

## Commit c5823a94 — "Quote sheet names by allow-list, not block-list"

### 6. `name_needs_quoting` (`base/src/expressions/utils/mod.rs`) — genuine upstream-main bug, viewer-relevant

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

**Dependency: none — `quote_name`/`name_needs_quoting` exist on upstream
main unmodified by `data-tables-impl`; cherry-picks cleanly. Second
upstream-PR candidate.** It pairs conceptually with `f56665e3` on
`data-tables-impl` (`parse_reference` last-`!` split): that fixed the
*import/parse* side, this fixes the *stringify* side. Upstream would ideally
get both together, but f56665e3 touches `xlsx/src/import/worksheets.rs`,
which `data-tables-impl` also modified for data tables, so extracting it
needs a small manual cherry-pick.

## Dependency matrix

| Change | Needs `data-tables-impl`? | Cherry-picks to upstream main? |
|---|---|---|
| `recompute_target_cells` | **Yes** (recompute_cells machinery) | No — travels with the data-tables/iterative work |
| `evaluate_formulas_only` / `evaluate_data_tables_only` | **Yes** (phased `evaluate()`) | No — same |
| `calc_results_converged` non-numeric fix | **Yes** (function is branch-only) | No — same |
| iterate/data-table/recompute tracing + `snapshot_drift` | **Yes** | No |
| Per-sheet eval counters (`evaluate_cell` hook) | No, but pointless alone | Possible, low value standalone |
| **YEARFRAC swap fix + test** | **No** | **Yes — clean; send as its own PR** |
| **`quote_name` allow-list + tests** | **No** | **Yes — clean; ideally alongside f56665e3's parse-side fix** |

## Practical implications

- **Rebase risk:** on the next merge of upstream main into
  `data-tables-impl`, the two correctness fixes (`date_and_time.rs`,
  `utils/mod.rs`) are the only hunks living in files upstream actively
  evolves independent of this work; upstreaming them first means they return
  via the merge and the branch copies collapse to no-ops.
- **Baba coupling:** baba's `services/excel` and the recompute harness build
  against `../IronCalc` by path, so baba pins whatever branch is checked
  out. All changes here are additive (no signatures changed, no behavior
  change with tracing off; the one semantic change — convergence — was
  re-certified on all five books), so `services/excel` builds unmodified.
- **Test status:** full `ironcalc_base` suite green in debug
  (2205 passed / 0 failed). One pre-existing release-profile failure,
  `test_to_from_bytes::errors` (bitcode error-string drift), fails on clean
  `data-tables-impl` too.
- **Suggested next step:** open two small PRs against `ironcalc/IronCalc`
  main (YEARFRAC swap; sheet-name quoting, optionally bundling f56665e3),
  keep the rest on the fork lineage, and fold this branch into
  `data-tables-impl` when ready — it is a strict superset.
