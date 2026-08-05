---
layout: doc
outline: deep
lang: en-US
---

# Data Tables

**Data tables** are Excel's what-if analysis feature. They evaluate one or more formulas repeatedly while substituting different values into one or two input cells. LibreOffice calls the same feature **Multiple Operations**.

IronCalc imports, calculates, creates, and exports Excel data tables, including workbooks that combine data tables with iterative calculation (deliberate circular references).

## Supported Behavior

IronCalc supports:

- One-variable data tables with input values down a column.
- One-variable data tables with input values across a row.
- Two-variable data tables with one row input and one column input.
- Cached XLSX data-table cells imported as normal cell values, so files display correctly even without recalculation.
- Recalculation of data-table outputs during `Model::evaluate()`.
- Iterative calculation (`<calcPr iterate="1">`), so workbooks whose data tables sit on top of intentional circular references (for example interest expense ↔ debt balance) converge as they do in Excel.
- Creating, replacing, and deleting data tables through the `UserModel` API, with undo/redo, in Rust and in the wasm, Python, and Node bindings.
- XLSX round-trip of `<f t="dataTable">` metadata on the table anchor cell and of the workbook's `<calcPr>` settings.

IronCalc does not currently include a UI dialog for creating or editing data tables; the feature is engine and API level.

## Spreadsheet Layout

Excel stores the output cells as a range, but the input headers and governing formula cells sit just outside that range.

### One-Variable Column Table

For a table with output range `C3:D5`:

- `B3:B5` contains input values.
- `C2:D2` contains the formula cells whose results are collected.
- The table substitutes each value from `B3:B5` into the input cell.
- Each row of `C3:D5` receives the results for that substituted value.

### One-Variable Row Table

For a table with output range `C3:E4`:

- `C2:E2` contains input values.
- `B3:B4` contains the formula cells whose results are collected.
- The table substitutes each value from `C2:E2` into the input cell.
- Each column of `C3:E4` receives the results for that substituted value.

### Two-Variable Table

For a table with output range `D3:E4`:

- `D2:E2` contains row-input values.
- `C3:C4` contains column-input values.
- `C2` contains the formula cell whose result is collected.
- The table substitutes each row-input value into the row input cell and each column-input value into the column input cell.
- Each output cell receives the formula result for that pair of substitutions.

## XLSX Mapping

In OOXML, the top-left output cell contains an empty formula element with `t="dataTable"`:

```xml
<f t="dataTable" ref="D3:E4" dt2D="1" dtr="1" r1="A1" r2="B1" ca="1"/>
```

IronCalc stores this as worksheet-level `DataTable` metadata:

- `range`: the output range, for example `D3:E4`.
- `two_dimensional`: Excel's `dt2D` flag.
- `row_oriented`: Excel's `dtr` flag for one-variable row tables.
- `r1`: the row input cell for two-variable tables, or the sole input cell for one-variable tables.
- `r2`: the column input cell for two-variable tables.
- `calculate_always`: Excel's `ca` flag.

The output cells themselves remain ordinary cached values in the worksheet grid. During export, IronCalc writes the data-table formula element back onto the top-left output cell.

Input cell references may be sheet-qualified, including quoted sheet names (`'P&L'!B7`).

## Calculation Model

Data tables run as part of `Model::evaluate()`, which proceeds in three phases: the normal formula sweep, data tables, then conditional formatting.

For each scenario, IronCalc does **not** write the scenario value into the input cell. Instead it uses read-time *reference redirection*: while a scenario is being computed, any read of the input cell — a direct reference, a cell inside a range, a spill — transparently sees the scenario value. The grid is never mutated, so there is nothing to restore afterwards and a scenario can never leak into the workbook. Concretely:

1. IronCalc evaluates normal workbook formulas.
2. It resolves each worksheet data table, skipping tables with invalid ranges or input references (a malformed table in an imported file never poisons the whole evaluation).
3. It reads the input-header values from the evaluated workbook.
4. For each scenario, it installs the scenario value(s) as read-time overrides on the input cell(s) and recomputes just the governing formula cells on demand.
5. It writes the collected scenario results into the output range, preserving each output cell's existing style.
6. It re-runs the formula sweep (a *settle* pass) so intermediate cells return to their true values and formulas that depend on the table's outputs see them.

A governing formula that evaluates to a range or an array has no single value for an output cell and produces `#VALUE!`.

`Model::evaluate_formulas_only()` and `Model::evaluate_data_tables_only()` expose the phases separately, so an application can implement Excel's `calcMode="autoNoTable"` policy (automatic recalculation that deliberately skips data tables because of their cost). `Model::evaluate_with_data_tables()` is an explicit alias of `evaluate()` for callers that want the data-table cost visible at the call site.

## Iterative Calculation

Workbooks that rely on deliberate circular references enable iterative calculation in Excel (`<calcPr iterate="1" iterateCount="100" iterateDelta="0.001"/>`). IronCalc honors these settings:

- With iteration **off** (the default), a circular reference produces `#CIRC!`, as before.
- With iteration **on**, circular cells seed at `0` (matching Excel) and the workbook sweep repeats until no numeric cell changes by more than `iterate_delta`, or `iterate_count` passes have run. Reaching the pass limit keeps the last values; like Excel, it is not an error.

Data tables compose with iterative calculation: when a table's governing formulas sit inside a circular region, each scenario is itself iterated to convergence, and the settle pass uses the iterative sweep.

The settings are available programmatically via `set_iterative_calculation(iterate, iterate_count, iterate_delta)` / `get_iterative_calculation()` on both `Model` and `UserModel` (undoable), and in all bindings.

## Creating Data Tables

`UserModel::set_data_table(sheet, range, row_input_cell, column_input_cell)` creates or replaces a table. The kind is inferred from which input cells are supplied, mirroring Excel's dialog:

- both → two-variable table,
- row input only → one-variable row-oriented table,
- column input only → one-variable column-oriented table.

Setting a table replaces any table anchored at the same top-left cell. `delete_data_table(sheet, row, column)` removes the table containing that cell and clears the orphaned output values (styles are kept). `get_data_table(sheet, row, column)` returns the table containing a cell, which a UI can use to detect that the cursor is inside a table body (Excel blocks partial edits of a data table's body). All operations participate in undo/redo.

The same API is exposed in the bindings as `setDataTable` / `deleteDataTable` / `getDataTable` (wasm and Node) and `set_data_table` / `delete_data_table` / `get_data_table` (Python).

## Current Limitations

- Data-table calculation is inherently expensive: each table costs one demand-driven recompute per scenario plus a settle sweep. This is the same cost shape as Excel, which is why Excel offers `calcMode="autoNoTable"`; use the phase-split API if you need that policy. A dependency-graph-scoped evaluation that avoids the settle pass is possible future work.
- There is no UI dialog for creating or editing data tables.
- Non-scalar governing-formula results are reported as `#VALUE!` rather than reduced by implicit intersection.

## Tests

The implementation includes:

- Engine tests for one-variable column, one-variable row, and two-variable data tables, and for formulas that consume data-table outputs (`base/src/test/test_data_table.rs`).
- Iterative-calculation tests: convergence, pass limits, seeding, `#CIRC!` when disabled, and circular data tables converging per scenario (`base/src/test/test_iterative_calculation.rs`).
- UserModel tests: creation, replacement at the same anchor, deletion clearing the body, and undo/redo round-trips (`base/src/test/user_model/test_data_table.rs`).
- XLSX import and export tests for `<f t="dataTable">` and `<calcPr>` round-trip (`xlsx/tests/test_data_table.rs`, `xlsx/tests/test_iterative_calculation.rs`).
