# Data Tables (What-If Analysis) — Analysis and Implementation Plan

Status: research / planning. Evidence gathered from the VEEV template
(`VEEV_Template_v9.xlsx`), the `data_table_first_attempt` branch, LibreOffice
26.2.3.2 (`sc/source/...`), and the Baba (`/Users/ddmoore/dev/baba`) and l123
(`/Users/ddmoore/dev/l123`) consumers.

## TL;DR

- The VEEV template has **three** what-if "data tables", all on the `Model_SENS`
  sheet. They are presentation/leaf cells — nothing downstream consumes them.
- Excel's OOXML stores a data table as one anchor cell with
  `<f t="dataTable" ref=… dt2D=… dtr=… r1=… r2=…/>`; interior cells are plain
  cached values.
- The `data_table_first_attempt` branch gets **import** right but its
  **evaluation is broken on real models** (returns `#VALUE!`/empty instead of
  the cached strings/numbers) and **export is missing** (tables are dropped on
  save).
- The deeper blocker is **not** data tables: the VEEV workbook sets
  `<calcPr iterate="1"/>` and `Model_SENS` row 9 is an inherently circular
  moving-average chain (`F9=AVG(E9,G9)`, `G9=AVG(F9,H9)`). IronCalc has no
  iterative-calc support, so a plain `evaluate()` of the *unmodified* file
  produces `#CIRC!` in **32 cells, all on `Model_SENS`**, which cascades into
  `#VALUE!` in the data-table master formula. Excel converges the cycle by
  iteration.
- **Plan**: Phase A — iterative calculation (prerequisite). Phase B — data
  tables done properly (fix evaluation, add export/round-trip, add authoring
  API, add the VEEV file as a regression fixture).

---

## 1. How data tables are used in the VEEV template

All three data tables live on `Model_SENS` and are sensitivity grids. The
anchor cell carries the `<f t="dataTable">`; every other cell in the range is
a plain cached `<v>`.

| # | Range      | Type          | r1 (row input)     | r2 (col input)  | Master formula                                  | Output        |
|---|------------|---------------|--------------------|-----------------|-------------------------------------------------|---------------|
| 1 | `Q75:U79`  | 2D (`dt2D=1`) | `F8` (growth rate) | `G10` (multiple) | `P74` = `+TEXT(G58,"$0")&" / "&TEXT(G61,"0%")`   | `"$197 / 9%"` |
| 2 | `W75:X79`  | 1D col        | `G10` (multiple)  | —               | per-column `W74`=+G59, `X74`=+G60               | numbers       |
| 3 | `Q86:U90`  | 2D            | `F8` (growth)     | `H9` (margin)   | `P85` (same TEXT master)                        | `"$216 / 13%"`|

Verified facts:

- **Headers come from cells around the range**, not the input cells. For table
  1 the top row `Q74:U74` (0.1…0.2) feeds the row input; the left column
  `P75:P79` (10…20) feeds the column input. `F8`/`G10` are *substitution
  targets*: Excel overwrites them per output cell, recomputes the master's
  dependency cone, and writes back the master's result.
- **OOXML semantics** (cross-checked against LibreOffice
  `sc/source/filter/oox/sheetdatabuffer.cxx` `finalizeTableOperation`):
  - 2D: master at corner `(top-1, left-1)`; `r1` = row input cell (top-row
    headers), `r2` = column input cell (left-column headers).
  - 1D column (`dtr=0`): one master per column in the row above the range;
    input values down the left column; substitute `r1 := leftHeader[row]`.
  - 1D row (`dtr=1`): one master per row in the column left of the range; input
    values across the top row; substitute `r1 := topHeader[col]`.
- **Outputs are leaves.** `grep` across all eight sheets: no cell references
  `Model_SENS!Q75…`, `W75…`, or `Q86…`. They are analyst-facing presentation
  cells. This is why Excel's `autoNoTable` mode (don't recalc data tables except
  on explicit F9) costs nothing here.
- The master is a **TEXT formula producing a string** (`"$250 / 19%"`). This
  matters for the evaluator: the `#VALUE!` the first attempt produces is the
  `TEXT("","$0")` failure that follows from an unsolved circular chain, not a
  data-table bug per se.

## 2. State of `data_table_first_attempt`

Built and tested against current `main`:

- **Import: correct.** Parses `<f t="dataTable">` into a `DataTable` struct on
  the worksheet (`base/src/types.rs`) and imports the anchor as its cached
  value instead of aborting with `NotImplemented`. The data model and the
  dt2D/dtr/r1/r2 semantics are right.
- **Evaluation: broken on real models.** After `evaluate_with_data_tables()`
  the VEEV tables return `#VALUE!` (2D) and `""` (1D). The two synthetic
  fixtures pass only because they substitute a single directly-referenced cell
  (`B1*2`); real models have a deep dependency cone that the
  `recompute_cells` primitive mis-handles. Root cause traced to the circular
  chain in §3, not the data-table machinery itself.
- **Export: missing entirely.** `xlsx/src/export/` has no `dataTable` handling,
  so saving drops the `<f t="dataTable">` element — the feature is not
  round-trippable.
- **Staleness.** The branch no longer compiles against current `main` (missing
  `Color` import in `xlsx/src/import/worksheets.rs` after the theme-color
  refactor). Its evaluation approach should be replaced, not patched.

## 3. The real blocker: iterative calculation

`xl/workbook.xml`:

```xml
<calcPr calcId="191029" iterate="1"/>
```

`Model_SENS` row 9 is an inherently circular moving-average chain:

```
E9 = 0.4524…            (plain value)
F9 = AVERAGE(E9, G9)    ──┐
G9 = AVERAGE(F9, H9)    ──┘  (F9 and G9 reference each other)
H9 = 0.46
```

Excel converges this by iteration (because `iterate="1"`). IronCalc has no
iterative-calc support, so a plain `evaluate()` of the *unmodified* workbook
returns `#CIRC!` for `F9`/`G9`, which cascades to **32 cells, all on
`Model_SENS`** (forward-year columns F/G/H; rows 9, 19, 21, 27, 31, 32, 38,
52, 55, 56, 66–70). Downstream: `G58` → `""` (IFERROR swallows `#CIRC`),
`G61` → `#VALUE!` (`TEXT("","$0")`), `P74` → `#VALUE!`. The model the data
tables sit on **cannot be computed by IronCalc today**, independent of data
tables. The cached `"$197 / 9%"` values only look correct because the
data-table anchor is imported as a frozen cached value that `evaluate()`
never recomputes.

LibreOffice computes the same file correctly: it both supports iterative calc
*and* evaluates the `=TABLE()` pseudo-formula by re-running the substitution
with the iterative solver (`ScInterpreter::ScTableOp` in
`sc/source/core/tool/interpr4.cxx`; `setTableOpCells` in
`sc/source/core/data/documentimport.cxx`).

## 4. Implementation plan

### Phase A — Iterative calculation (prerequisite, unblocks VEEV by itself)

1. **Persist settings**: parse and export `calcPr` — `iterate`, `iterateCount`
   (Excel default 100), `iterateDelta` (0.001), and `calcMode` (including
   `autoNoTable`). Files: `xlsx/src/import/workbook.rs`,
   `xlsx/src/export/workbook.rs`, plus storage on `Workbook`/`CalcProperties`
   in `base/src/types.rs`.
2. **Evaluation fixpoint**: in `Model::evaluate`, when `iterate` is on, after
   the topological pass, take cells that resolved to `#CIRC!`; seed them with
   their last value (or 0), then re-evaluate repeatedly until no value changes
   by more than `iterateDelta` or `iterateCount` is hit (remaining cells stay
   `#CIRC!`, matching Excel). The existing cycle detection in `evaluate_cell`
   (it returns `#CIRC` on the second visit to an in-progress cell) is the hook.
3. **Blast radius**: constrain the fixpoint to the detected cycle's SCC plus
   its direct consumers, not a full re-evaluation of the workbook each pass.
   IronCalc's `cells`/`support` cache is built for a single acyclic pass, so
   the fixpoint must re-evaluate the SCC without re-clearing global state each
   iteration.

**Correctness test**: the 32 `#CIRC` cells converge to Excel's cached values;
the VEEV model's `Model_SENS!G21`, `G58`, `G61`, `P74` compute without error.

### Phase B — Data tables, done properly

Build on the first attempt's *import* and *data model* (which are correct);
replace evaluation; add export and authoring.

1. **Fix evaluation** (replace `recompute_cells`): for each output cell,
   (a) substitute the input cell(s) with the header value, (b) re-run the
   **full fixed-point evaluate** so iterative cells converge per substitution
   (depends on Phase A), (c) read the master cell's value, (d) restore inputs.
   Keep the `autoNoTable` opt-in via `evaluate_with_data_tables()` — faithful
   to Excel and free for leaf tables like VEEV's. The string-result case
   (TEXT master) is already handled by the existing `write_value` `String` arm.
2. **Add export/round-trip** (`xlsx/src/export/worksheets.rs`): write the
   anchor's `<f t="dataTable" ref=… dt2D=… dtr=… r1=… r2=… ca=…/>` plus cached
   `<v>` values for interior cells. Without this the feature can't survive a
   save — critical for Baba.
3. **Authoring API + UI** (for Baba and l123): add a `Model` method to create a
   data table on a range from (row input cell, col input cell, master cell) —
   the inverse of import. Baba's `services/excel` calls this to emit a
   VEEV-style sensitivity block instead of hand-rolling it.
4. **Tests**: keep the two synthetic fixtures; add the **VEEV file as a
   regression fixture** asserting the three tables compute to Excel's cached
   values (e.g. `Q75="$197 / 9%"`, `W75=0.08088…`). This test would have caught
   the first attempt's breakage.

## 5. Recommendation

Start with **Phase A (iterative calc)** on a fresh branch off `main`:
- it unblocks the VEEV model on its own, independent of data tables;
- it is a well-bounded change with a clear correctness test (the 32 `#CIRC`
  cells should converge to Excel's values);
- it is a prerequisite for Phase B to produce correct results on this file.

Phase B then layers on the existing (correct) import and the new iterative
evaluator.

## 6. Consumers

- **Baba** (`services/excel`): uses IronCalc as a local path dep
  (`../../../IronCalc/xlsx`), builds workbooks with formulas, calls
  `evaluate()`, exports xlsx. Data tables + iterative calc let it author and
  render VEEV-style sensitivity blocks natively.
- **l123** (`/Users/ddmoore/dev/l123`): uses published `ironcalc` 0.7 plus an
  `ironcalc_lotus` crate. Lotus 1-2-3 had `/Data Table`; the feature benefits
  l123 too.
