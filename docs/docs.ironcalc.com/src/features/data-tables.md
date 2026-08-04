---
layout: doc
outline: deep
lang: en-US
---

# Data Tables

**Data tables** are Excel's what-if analysis feature. They evaluate one or more formulas repeatedly while substituting different values into one or two input cells. LibreOffice calls the same feature **Multiple Operations**.

IronCalc supports calculating imported Excel data tables and preserving them when saving back to XLSX.

## Supported Behavior

IronCalc supports:

- One-variable data tables with input values down a column.
- One-variable data tables with input values across a row.
- Two-variable data tables with one row input and one column input.
- Cached XLSX data-table cells imported as normal cell values.
- Recalculation of data-table outputs during `Model::evaluate()`.
- XLSX round-trip of `<f t="dataTable">` metadata on the table anchor cell.

IronCalc does not currently include a UI dialog for creating or editing data tables. Applications can create them by adding `DataTable` metadata to a worksheet, or by importing an XLSX file that already contains them.

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

## Calculation Model

Data tables run as part of `Model::evaluate()`:

1. IronCalc evaluates normal workbook formulas.
2. It resolves each worksheet data table.
3. It reads input-header values from the evaluated workbook.
4. For each scenario, it temporarily writes the scenario value or values into the input cells.
5. It recomputes the governing formula cell or cells.
6. It restores the original input cells.
7. It writes the scenario results into the data-table output range.
8. It recalculates normal formulas so formulas that depend on data-table outputs see the updated results.

`Model::evaluate_with_data_tables()` is also available as an explicit alias for callers that want to make the data-table cost visible in their code.

## Current Limitations

Data-table calculation can be expensive because it evaluates formulas repeatedly.

IronCalc does not yet support Excel iterative calculation. Workbooks that rely on `<calcPr iterate="1">` may still differ from Excel even though their data-table metadata imports and recalculates. For example, a workbook with circular formulas feeding a sensitivity table will still need iterative-calculation support for exact parity.

## Tests

The implementation includes:

- Engine tests for one-variable column, one-variable row, and two-variable data tables.
- A test that formulas depending on data-table outputs see recalculated table values.
- XLSX import and export tests using a generated workbook with `<f t="dataTable">`.

