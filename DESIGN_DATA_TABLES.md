# RFC: Data Tables (What-If Analysis) and Iterative Calculation for IronCalc 1.0

*Author: Duane Moore — draft for discussion with IronCalc maintainers*

## Summary

This proposal adds support for Excel **Data Tables** (the "What-If Analysis"
feature; LibreOffice calls it "Multiple Operations") and **iterative
calculation** (`<calcPr iterate="1">`) to IronCalc. Today, importing any
workbook that contains a data table fails hard: the importer returns
`NotImplemented("data table formulas")`, so the entire file is rejected. Data
tables are common in exactly the kind of workbook IronCalc's 1.0 audience
cares about — financial models, sensitivity analyses, loan/pricing sheets —
so a single sensitivity grid currently makes a whole workbook unopenable.

The work is complete and tested on a branch, and I'd like to discuss how to
get it (or an adjusted version of it) merged. I'm happy to split it into
small reviewable PRs (a suggested split is at the end), to rework any of the
design decisions below, and to maintain the feature afterwards.

## What a data table is (30-second refresher)

A data table re-evaluates one or more formulas repeatedly, substituting each
of a list of values into one input cell (one-variable tables) or each pair
from a row and a column of values into two input cells (two-variable tables).
The results are written into an output range. In the file format, the whole
construct is stored as a single element on the top-left output cell:

```xml
<c r="D3"><f t="dataTable" ref="D3:E4" dt2D="1" dtr="0" r1="B1" r2="B2" ca="1"/><v>42</v></c>
```

All other output cells are plain cached values. The input value headers and
the formula being evaluated live *outside* the range (row above, column to
the left).

## Scope

Included:

- One-variable column-oriented and row-oriented tables, and two-variable
  tables — calculated during `Model::evaluate()`, matching Excel results.
- XLSX import and export round-trip of the data-table metadata (cached
  output values import as ordinary cells even if a consumer never recalculates).
- Iterative calculation (`<calcPr iterate="1" iterateCount iterateDelta>`),
  which is a hard prerequisite: real workbooks that use data tables very
  often also contain deliberate circular references (e.g. interest expense ↔
  debt balance), and without iteration those workbooks evaluate to `#CIRC!`.
- A `UserModel` authoring API (`set_data_table` / `delete_data_table` /
  `get_data_table`, plus iterative-calc settings) with undo/redo support.
- Bindings for wasm, Python, and Node exposing the same API.

Not included (deliberately):

- Any UI for creating/editing data tables. The webapp is untouched; the
  feature is engine + API only.
- Excel's `calcMode="autoNoTable"` policy decisions — but the evaluation
  phases are exposed separately (`evaluate_formulas_only` /
  `evaluate_data_tables_only`) so an application *can* implement that policy.

## Design

### Data model

Two small additions to `types.rs`, both mirroring OOXML directly:

- `DataTable { range, two_dimensional, row_oriented, r1, r2, calculate_always }`
  stored as `Vec<DataTable>` on `Worksheet`. This mirrors
  `<f t="dataTable" ref dt2D dtr r1 r2 ca>` one-to-one, so import/export is
  a straight mapping and nothing is lost in round-trip. The output cells
  themselves stay ordinary cells in the grid.
- `CalcProperties { iterate, iterate_count, iterate_delta }` on
  `WorkbookSettings`, mirroring `<calcPr>` with Excel's defaults (100 /
  0.001).

### Calculating a table: read-time input redirection

The naive implementation (and my first one) substitutes scenario values by
*writing* them into the input cells, recomputing, and restoring the original
values afterwards. That works, but it mutates the grid on every scenario:
an error path or early return can leave a scenario value behind, the
mutation is observable to anything that reads the model mid-evaluation, and
it interacts badly with undo history.

The implementation instead uses **read-time reference redirection**. The
model carries an optional override map:

```rust
pub(crate) data_table_overrides: Option<HashMap<(u32, i32, i32), CalcResult>>,
```

and `evaluate_cell` — the single choke point through which *every* cell read
funnels (direct references, cells read as part of a range, spill anchors) —
checks it as its first statement. While a scenario is being computed, reads
of the input cell(s) see the scenario value; the grid is never touched, so
there is nothing to restore and nothing can leak. On the normal recalc path
the field is `None` and the cost is one branch.

Per scenario, the engine recomputes only the governing formula cells
(the row above / column left of the range) on demand via
`recompute_with_overrides`, which installs the overrides, clears the memo
(`cells`/`support`), evaluates the targets, and always clears the overrides
before returning. Scenario results are collected and written into the output
range with the existing cell styles preserved.

Because the demand-driven recompute persists intermediate formula values
into the grid (that is how the current evaluator works), a **settle pass**
re-runs the normal workbook evaluation after each table, so intermediate
cells return to their true values and downstream formulas that consume the
table's outputs see them. This is correct but costs an extra sweep per
table — see "Performance and open questions".

### Evaluation order

`Model::evaluate()` becomes three phases:

1. normal formula sweep (iterative when enabled),
2. data tables,
3. conditional formatting.

Phases 1 and 2 are also exposed individually because Excel itself has a
calculation mode (`autoNoTable`) where automatic recalculation deliberately
skips data tables for cost reasons; embedders should be able to honor that.

### Iterative calculation

When `calc_properties.iterate` is set, the circular-reference arm of
`evaluate_cell` (the `CellState::Evaluating` case that today returns
`#CIRC!`) instead returns the cell's value from the previous pass — or 0.0
if there is none, which is Excel's seed. The whole-workbook sweep then runs
as a fixed-point loop: evaluate, snapshot all numeric cells, repeat until no
cell moves by more than `iterate_delta` or `iterate_count` passes elapse.
Matching Excel, hitting the pass limit keeps the last values and is not an
error. Convergence requires at least two passes so a pass-0 coincidence
can't end the loop early.

Data tables compose with this: when the governing formulas sit inside a
circular region, each scenario's recompute is itself iterated to convergence
(with a slightly different convergence test that treats unchanged
non-numeric results — labels, blanks, booleans, same-kind errors — as
stable, since they can't iterate toward a fixpoint), and the settle pass
uses the iterative sweep.

### Authoring API and undo/redo

`UserModel::set_data_table(sheet, range, row_input_cell, column_input_cell)`
infers the table kind from which inputs are given (both → two-variable, row
only → row-oriented, column only → column-oriented). Setting a table
replaces any table anchored at the same top-left cell; deleting one clears
the orphaned output cells (keeping styles) so stale values don't linger.
Both operations record `Diff` entries, so they undo/redo cleanly. New `Diff`
variants are appended at the end of the enum because it is bitcode-encoded
by positional discriminant.

Ranges and input-cell references are validated at set time (the range needs
a row above and a column to its left; input cells may be sheet-qualified,
including quoted sheet names with escaped quotes).

## Related fixes found along the way

Testing against real-world workbooks surfaced three pre-existing bugs that
are independent of data tables; I can send each as its own small PR:

1. **`parse_reference` splits at the first `!`** — Excel allows `!` in sheet
   names (only `\ / ? * [ ] :` are forbidden), so a sheet named `!CAP`
   produced shredded references on import. Fixed to split at the *last* `!`.
2. **Sheet-name quoting uses a block-list** — when the engine re-stringifies
   formulas, names like `SUMM_P&L` or `!CAP` weren't quoted and produced
   unparseable or `#NAME?` formulas (this silently corrupted 122 cells of a
   real workbook on recompute). Fixed by inverting to an allow-list: only
   names matching the unquoted-identifier grammar skip quotes.
3. **`YEARFRAC` with reversed dates** — Excel swaps start/end *before* the
   asymmetric 30/360 day-of-month adjustments; IronCalc took the absolute
   value at the end, giving off-by-one day counts.

## Performance and open questions for maintainers

I want to be upfront about the costs and the parts I'd most like feedback on:

1. **Cost model.** A table with N scenarios costs N demand-driven recomputes
   of the governing cone plus one settle sweep. Because `recompute_cells`
   clears the whole memo and the evaluator persists intermediates, the
   worst case approaches O(scenarios × affected formulas), the same shape as
   Excel's own cost (Excel added `autoNoTable` for exactly this reason). The
   right long-term fix is a non-persisting, input-cone-scoped evaluation,
   which needs a reverse-dependency structure the engine doesn't maintain
   yet — I've left a `TODO(perf)` marking the seam. Is there appetite for
   that structure post-1.0, and does this interim shape seem acceptable?
2. **Where should data-table evaluation live in `evaluate()`?** I've put it
   between formulas and conditional formatting, always on. Excel's
   `calcMode` policy could alternatively be honored inside the engine rather
   than left to embedders.
3. **Non-scalar results.** A governing formula that evaluates to a range or
   array writes `#VALUE!` into the output cell (Excel coerces via implicit
   intersection in legacy mode). Good enough, or should this follow the
   engine's implicit-intersection behavior?
4. **`evaluate_with_data_tables()`** exists as an explicit alias of
   `evaluate()` for call-site clarity. Keep or drop?
5. **Naming/API bikeshedding** on `DataTable` fields (they currently mirror
   OOXML: `r1`/`r2`) is very welcome.

## Testing

- Engine tests: one-variable column, one-variable row, and two-variable
  tables; formulas downstream of table outputs seeing recalculated values;
  input redirection not leaking into subsequent evaluations
  (`base/src/test/test_data_table.rs`).
- Iterative calculation: convergence, pass limits, seed behavior, circular
  data tables per scenario (`base/src/test/test_iterative_calculation.rs`).
- UserModel: authoring, replacement at the same anchor, delete clearing the
  body, undo/redo round-trips (`base/src/test/user_model/test_data_table.rs`).
- XLSX: import of `<f t="dataTable">`, export round-trip byte-compat with
  the metadata, `<calcPr>` round-trip (`xlsx/tests/test_data_table.rs`,
  `xlsx/tests/test_iterative_calculation.rs`).
- Validated against real-world financial workbooks containing multiple
  interacting data tables and deliberate circular references.

## Suggested PR split

Each step compiles and passes tests on its own:

1. **xlsx: import data-table metadata instead of failing** (+ export
   round-trip). Even without calculation this is a big win: files open, and
   cached values display.
2. **engine: calculate data tables during evaluate()** (the redirection
   mechanism + `data_table.rs`).
3. **engine: iterative calculation** (`CalcProperties`, `<calcPr>` round-trip,
   fixpoint sweep) — independently useful, even without data tables.
4. **engine: converge circular data tables per scenario** (small, composes
   2 + 3).
5. **UserModel authoring API + undo/redo.**
6. **Bindings** (wasm / Python / Node).
7. The three independent bug fixes, as separate PRs, in parallel.

Feedback on any and all of this is very welcome — including "we'd rather
this waited until after 1.0" or "we'd want the calculation model done
differently". The branch is at `<link to branch/compare>`, and the docs page
draft is at `docs/src/features/data-tables.md`.
