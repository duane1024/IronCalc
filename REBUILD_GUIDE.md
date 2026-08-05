# Rebuilding the Data Tables implementation by hand

A stage-by-stage guide for re-implementing everything on `data-tables-impl`
yourself, in an order where every stage compiles, passes tests, and is small
enough to hold in your head. Each stage lists **what** to build, **why** it's
shaped that way (the design decisions you'll need to defend in review), and
the **traps** — the non-obvious things the current implementation had to get
right, which are exactly what a reviewer will probe.

Work from a fresh branch off upstream `main`. Don't look at the diff while
writing a stage; implement from this guide, *then* diff against
`data-tables-impl` to see what you missed. The misses are where your
understanding has gaps — chase each one down before moving on.

Suggested rhythm per stage: write the failing test first, implement, run
`cargo test -p ironcalc_base` (or `-p ironcalc` for xlsx), diff, reconcile.

---

## Stage 0 — Read the engine until you can answer these

Before writing anything, read these and be able to answer the questions from
memory. Review pushback will land here, not in your new code.

- `base/src/model.rs` — `evaluate_cell`, `CellState`, the `cells` memo map,
  `evaluate()` (the two-phase spill design), `get_cell_value`.
- `base/src/calc_result.rs` — the `CalcResult` variants.
- `base/src/expressions/parser/mod.rs` — `parse_range`;
  `base/src/expressions/utils` — `parse_reference_a1`.
- `base/src/user_model/` — `common.rs` (`push_diff_list`,
  `evaluate_if_not_paused`), `history.rs` (`Diff`), `undo_redo.rs`.
- `xlsx/src/import/worksheets.rs` — `load_sheet`, how `<f>` nodes are
  dispatched by their `t` attribute; `xlsx/src/export/worksheets.rs` —
  `get_worksheet_xml`.

Questions you must be able to answer:

1. Why does *every* read of a cell — direct reference, inside a range,
   spill — end up in `evaluate_cell`? (This is the entire justification for
   the redirection design.)
2. What do `CellState::Evaluating` / `Evaluated` mean, and how does the
   engine currently detect a circular reference?
3. What does the evaluator do with an evaluated formula value — where does
   it get persisted, and why does that matter for anything that recomputes
   "just a few cells"?
4. In OOXML, what exactly is in the file for a data table? Open a real one:
   make a tiny table in Excel/LibreOffice, save, `unzip -p file.xlsx
   xl/worksheets/sheet1.xml`, and stare at the `<f t="dataTable">` element
   and the plain `<v>` cells around it. Do the same for a workbook with
   iterative calc enabled and look at `<calcPr>` in `xl/workbook.xml`.

---

## Stage 1 — Data model (`types.rs`, `new_empty.rs`)

**What.** Add to `base/src/types.rs`:

- `struct DataTable { range: String, two_dimensional: bool, row_oriented:
  bool, r1: String, r2: Option<String>, calculate_always: bool }` with the
  same derive set as its neighbors (`Serialize, Deserialize, Encode, Decode,
  Debug, PartialEq, Eq, Clone`).
- `pub data_tables: Vec<DataTable>` on `Worksheet`.
- `struct CalcProperties { iterate: bool, iterate_count: u32, iterate_delta:
  f64 }` with `Default` = `{ false, 100, 0.001 }`, and a
  `calc_properties: CalcProperties` field on `WorkbookSettings`.

Fix every construction site the compiler flags (`new_empty.rs` has two: the
empty worksheet and the empty workbook settings).

**Why this shape.**
- Fields mirror OOXML attributes (`ref`→`range`, `dt2D`, `dtr`, `r1`, `r2`,
  `ca`) one-to-one so the xlsx layer is a dumb mapping and round-trip is
  lossless. Keeping `range`/`r1`/`r2` as *strings* (not parsed indices)
  means import can never fail on a weird-but-valid reference — validation
  happens at evaluation/authoring time instead.
- Table metadata lives on the `Worksheet`, not on a cell, because in the
  file it's attached to the anchor cell but semantically it *describes a
  range*; storing it per-cell would make range edits and export lookups
  awkward. The output cells stay ordinary cells — that's also how the file
  format sees them.
- `CalcProperties` defaults are Excel's documented defaults (iterateCount
  100, iterateDelta 0.001).

**Traps.**
- `WorkbookSettings` currently derives `Eq`. `f64` isn't `Eq`, so adding
  `calc_properties` forces you to drop `Eq` from `WorkbookSettings` (keep
  `PartialEq`). Leave a comment saying why, or a reviewer will "helpfully"
  re-add it.
- These structs are `bitcode`-encoded (`Encode, Decode`). Adding *fields* to
  structs used in persisted history has compatibility implications — here
  it's fine because `Worksheet`/settings snapshots are whole-value, but keep
  it in mind for Stage 5.

**Check.** `cargo check -p ironcalc_base` compiles; no behavior change.

---## Stage 2 — XLSX import & export round-trip

**What (import, `xlsx/src/import/worksheets.rs`).**
- In `load_sheet`'s formula dispatch, replace the
  `"dataTable" => return Err(NotImplemented(...))` arm with code that reads
  `ref` (required), `dt2D`, `dtr`, `r1` (required), `r2` (optional), `ca`,
  pushes a `DataTable` onto a local `Vec`, and stores it on the returned
  `Worksheet`.
- Guard the "cached `ca` formula with no text" special case (which imports a
  formula-less recalc marker) with `formula_node.attribute("t").is_none()` —
  otherwise a `<f t="dataTable" ca="1"/>` node (empty, no text!) is swallowed
  by that earlier branch and never reaches your new arm. **This is the
  subtlest line of the import change; make sure you can explain it.**
- Filter `sheet_data_nodes.children()` to `has_tag_name("row")` and row
  children to `has_tag_name("c")`. Why: once you re-serialize with nested
  elements you'll hit whitespace/text nodes between elements; the old code
  iterated *all* children and only worked by accident on Excel's compact
  output.

**What (workbook-level, `xlsx/src/import/workbook.rs` + export twin).**
- Parse `<calcPr>`: `iterate` (accept `"1"` and `"true"`), `iterateCount`,
  `iterateDelta`, defaulting each missing attribute to the Excel default.
  Thread it through `WorkbookXML` into `settings.calc_properties`.
- Export: emit `<calcPr iterate="1" iterateCount=".." iterateDelta=".."/>`
  when `iterate` is true, else the existing bare `<calcPr/>`.

**What (export, `xlsx/src/export/worksheets.rs`).**
- Build a `HashMap<(row, col), &DataTable>` of anchors (top-left cell of
  each table's `range`) once per sheet.
- When emitting any cell that is an anchor, inject
  `<f t="dataTable" ref=".." dt2D=".." dtr=".." r1=".."[ r2=".."][ ca="1"]/>`
  *before* the `<v>` element. This must be handled in **every** cell-kind arm
  — empty, boolean, number, error, shared string, inline string — because
  after import the anchor cell is whatever plain value was cached. XML-escape
  `range`/`r1`/`r2`.

**Why this shape.** Import-then-export must reproduce the metadata even if
the consumer never recalculates — that's the "files open and display" value
of this stage standing alone. The anchor map is per-sheet O(tables) and
avoids a per-cell scan.

**Traps.**
- Emit `r2` and `ca` only when present/true; unconditional attributes break
  byte-for-byte round-trip tests and diverge from Excel output.
- The empty-cell arm changes shape: `<c r=".."/>` becomes
  `<c r="..">…formula…</c>` only when an anchor lands on a styled-but-empty
  cell.

**Tests.** Port/re-derive `xlsx/tests/test_data_table.rs` (build a workbook
XML in-test containing a data table, import, assert the `DataTable` fields,
export, assert the `<f t="dataTable">` string round-trips) and
`xlsx/tests/test_iterative_calculation.rs` for `<calcPr>`.

---

## Stage 3 — Engine evaluation (`base/src/data_table.rs` + `model.rs`)

This is the heart. Build it in three sub-steps.

### 3a. The redirection mechanism (`model.rs`)

- Add `pub(crate) data_table_overrides:
  Option<HashMap<(u32, i32, i32), CalcResult>>` to `Model` (init to `None`
  in both constructors — `from_bytes` path in `model.rs` and `new_empty.rs`).
- At the **very top** of `evaluate_cell`, before fetching the cell: if
  overrides are installed and contain `(sheet, row, column)`, return the
  override clone.
- Add:
  - `recompute_cells(&mut self, targets) -> Vec<CalcResult>`: clear `cells`,
    `support`, variable stack, lambdas; then `evaluate_cell` each target.
    (Demand-driven: only the targets' precedent cones get evaluated.)
  - `recompute_with_overrides(&mut self, targets, overrides)`: install
    overrides, run `recompute_cells` (later: or its iterative twin), then
    **unconditionally** set overrides back to `None`. The install/clear
    pairing lives in exactly one function so it can't leak.

**Why redirection instead of write-and-restore.** First implementation wrote
scenario values into the input cells and restored after. Problems you should
be able to recite: (1) any error path between write and restore leaves the
workbook corrupted; (2) the mutation is observable (e.g. to `get_cell_value`
calls during evaluation, to anything hashing the workbook); (3) input cells
can be *formulas* in weird workbooks — restoring a formula cell from a
captured value is lossy; (4) it churns undo/history machinery. Redirection
touches nothing: reads are intercepted at the single choke point every read
funnels through — plain refs, range iteration, and spill reads all call
`evaluate_cell`. On the normal path the field is `None`: one branch, ~zero
cost.

**Why clear the whole memo per scenario.** The memo (`cells`) caches values
computed under *previous* overrides; keeping it would serve stale scenario
values. Clearing everything is the blunt-but-correct option. The precise
option — invalidate only the input cells' dependent cone — needs a
reverse-dependency index the engine doesn't have. Leave a `TODO(perf)`
noting exactly that; it shows reviewers you know the cost and the fix.

### 3b. Table evaluation (`data_table.rs`, new file)

Helpers first:

- `split_sheet_reference("'P&L'!B7") -> (Some("P&L"), "B7")`: handle quoted
  sheet names with `''` escapes and the unquoted `Sheet!A1` form (split on
  the *last* `!` — see Stage 7). Then `parse_cell_reference` resolves the
  sheet name via the model (falling back to the table's own sheet) and
  parses the A1 part (uppercased — the file can contain `b7`).
- `data_table_index_at(worksheet, row, col)`: position of the table whose
  parsed `range` contains the cell.

Then a `ResolvedDataTable` (numeric bounds + resolved input refs) and three
methods on `Model`:

- `resolve_data_tables()`: walk all sheets, parse each table's range and
  input refs; **skip silently** anything invalid (`top <= 1 || left <= 1` —
  a table needs a header row above and a column to its left — or unparsable
  refs). Skipping, not erroring, because a bad table in an imported file
  must not poison `evaluate()` for the whole workbook.
- `compute_one_data_table(table, outputs)`: three cases, all following the
  same pattern — *read the input header values first* (they may themselves
  be formulas already evaluated in phase 1), then per scenario install
  overrides and recompute the governing cells:
  - **column-oriented 1-var** (`dtr=0`): inputs down the column left of the
    range (`left-1`), governing formulas across the row above (`top-1`).
    One recompute per *row*, overriding `r1` with that row's input; results
    fan across the columns.
  - **row-oriented 1-var** (`dtr=1`): mirror image — inputs across `top-1`,
    governing formulas down `left-1`, one recompute per *column*.
  - **two-variable** (`dt2D=1`): single governing formula at
    `(top-1, left-1)`; row inputs across `top-1`, column inputs down
    `left-1`; one recompute per output cell with *both* `r1` and `r2`
    overridden. Requires `r2` — bail if absent.
- `compute_data_tables()`: for each resolved table, run the scenarios into
  an `outputs: Vec<(CellReferenceIndex, CalcResult)>` buffer, then write
  every output through `write_value` **preserving the cell's existing
  style**, then run a **settle pass** (full workbook evaluation) before the
  next table.

`write_value` maps `CalcResult` → concrete cell (`Number`, `Boolean`,
string, error, empty clears contents keeping style); `Range`/`Array`/
`Lambda` results become `#VALUE!` — a governing formula returning a
non-scalar has no single value to put in one output cell.

**Why the settle pass.** `recompute_cells` *persists* every intermediate
formula value it evaluates (that's how the evaluator works — you answered
this in Stage 0 Q3). After N scenarios, the governing/intermediate cells
hold values computed under the *last* scenario's overrides. The settle
sweep restores them to their true values and also lets formulas elsewhere
that reference the table's output range see fresh outputs before the next
table runs. Note what does *not* need restoring: the input cells — they
were never written.

**Why buffer outputs instead of writing during the sweep.** Writing an
output cell mid-sweep could perturb later scenarios of the *same* table
(e.g. if the governing formula ranges over the table body — pathological
but legal). Collect, then write.

### 3c. Wire into `evaluate()`

Split the existing `evaluate()` body into `evaluate_workbook_cells()` (the
existing two-phase spill sweep, minus conditional formatting) and make
`evaluate()` = formula sweep → `compute_data_tables()` → conditional
formatting. Add the thin public phase wrappers (`evaluate_formulas_only`,
`evaluate_data_tables_only`, `evaluate_with_data_tables`) — justified by
Excel's `calcMode="autoNoTable"`, where automatic recalc deliberately skips
tables.

**Tests** (`base/src/test/test_data_table.rs`): 1-var column, 1-var row,
2-var; a formula *outside* the table that references table outputs sees
post-table values; inputs unchanged after evaluation; invalid table is
skipped without panicking. Assert against values you computed by hand.

---

## Stage 4 — Iterative calculation (`model.rs`)

**What.**
- In `evaluate_cell`'s `CellState::Evaluating` arm (the circular-reference
  detection): if `calc_properties.iterate`, return `iteration_seed(cell)` —
  the cell's previous numeric value, else `0.0` (Excel's seed) — instead of
  `#CIRC!`.
- `evaluate_workbook_cells_iterative()`: if `iterate` is off, delegate to
  the plain sweep. Otherwise loop up to `iterate_count.max(1)` passes:
  sweep, take a `numeric_snapshot()` (map of every numeric cell), and stop
  when `snapshots_converged(prev, cur, delta)` — same key set, no cell moved
  more than `iterate_delta`. **Require `iteration >= 1` before declaring
  convergence** (a pass-0 seed can coincidentally match). Hitting the pass
  limit keeps the last values — Excel does not error on non-convergence.
  Use this everywhere the full sweep ran (in `evaluate()` and the settle
  pass).
- `recompute_cells_iterative(targets)`: per-scenario twin — repeat
  `recompute_cells(targets)` until `calc_results_converged` or the pass
  limit. Its convergence test differs from the snapshot one: it compares the
  *target result vectors* pairwise — numbers within delta, and **unchanged
  non-numerics (strings/booleans/blanks/same-kind errors) count as
  converged**. Why: non-numeric values can't iterate toward a fixpoint; if a
  target set includes a label, requiring numeric convergence of it would
  force every scenario to burn all `iterate_count` passes. A pair that
  changes *kind* between passes is not converged.
- Route `recompute_with_overrides` and `recompute_target_cells` through the
  iterative variant when `iterate` is on.
- `set_iterative_calculation(iterate, count, delta)` /
  `get_iterative_calculation()` accessors on `Model`.

**Why fixpoint-by-sweep, not something smarter.** Excel's own semantics are
"recalculate everything up to N times until quiet"; matching it exactly is
the compatibility goal, and per-pass seeding falls out of the existing
persistence behavior for free (each pass reads the previous pass's
persisted values through the `Evaluating` arm).

**Traps.**
- `iterate_count.max(1)` — a file can say `iterateCount="0"`.
- The snapshot only tracks *numeric* cells; a cell appearing or disappearing
  between passes counts as non-convergence (key-set equality).

**Tests** (`base/src/test/test_iterative_calculation.rs`): the classic
`A1=B1+1, B1=A1` non-convergent pair stops at the pass limit without error;
a convergent geometric relaxation (e.g. `A1 = 0.5*A1 + 1` → 2.0) lands
within delta; iteration off still yields `#CIRC!`; a data table over a
circular model converges per scenario (this is the Stage 3+4 composition
test — a workbook with interest↔balance style circularity feeding a
sensitivity grid).

---

## Stage 5 — UserModel authoring API + undo/redo

**What.**
- `Model::set_data_table(sheet, data_table) -> Result<Option<DataTable>>`
  (in `data_table.rs`): validate range ("needs a row above and a column to
  its left") and input refs; **replace** any table anchored at the same
  top-left cell, returning the replaced one (that's the undo payload).
- `Model::delete_data_table(sheet, row, col) -> Result<DataTable>`: remove
  the table containing that cell and **clear the orphaned output cells,
  keeping styles** (otherwise stale computed values masquerade as data);
  return the removed table.
- `Model::get_data_table(sheet, row, col) -> Result<Option<DataTable>>` —
  lookup by containment, so a UI can detect "cursor is inside a table body"
  (Excel blocks partial edits of a table body; embedders need this hook).
- New file `base/src/user_model/data_table.rs`: `build_data_table` infers
  the kind from which input cells the caller supplies (both → 2-var, row
  only → row-oriented 1-var, column only → column-oriented 1-var, neither →
  error). `UserModel::set_data_table/delete_data_table` wrap the model
  calls, push a `Diff`, and `evaluate_if_not_paused()`.
  `set_iterative_calculation` does the same with a before/after
  `CalcProperties` pair.
- `Diff` variants: `SetDataTable { sheet, old_value: Box<Option<DataTable>>,
  new_value: Box<DataTable> }`, `DeleteDataTable { sheet, old_value:
  Box<DataTable> }`, `SetCalcProperties { old_value, new_value }`.
  **Append them at the END of the enum** — `Diff` is bitcode-encoded by
  positional discriminant; inserting mid-enum silently corrupts previously
  serialized undo history. Put that in a comment. Boxed because the enum's
  size is its largest variant.
- Undo/redo arms: undo of `SetDataTable` restores `old_value` (or removes
  the table if it was `None`); redo re-applies `new_value`; symmetric for
  delete and calc properties.

**Why the inference API** (`row_input_cell: Option<&str>, column_input_cell:
Option<&str>`) instead of exposing `dt2D`/`dtr` flags: callers can't express
an inconsistent state (e.g. `two_dimensional=true` with no `r2`), and it
matches how Excel's dialog works — you fill in one or both boxes.

**Tests** (`base/src/test/user_model/test_data_table.rs`): create → values
correct; replace at same anchor returns/undoes to the old table; delete
clears body; full undo/redo round-trip re-evaluates to identical values.

---

## Stage 6 — Bindings (wasm / Python / Node)

Mechanical exposure of the five UserModel calls, matching each binding's
local conventions: `getDataTable`/`setDataTable`/`deleteDataTable` +
`getIterativeCalculation`/`setIterativeCalculation` (camelCase via
`wasm_bindgen(js_name)` / `napi(js_name)`; snake_case for Python).
`DataTable` and `CalcProperties` need serializable mirror types
(`bindings/python/src/types.rs`, `bindings/wasm/types.ts`). Copy the pattern
of a recently added API (e.g. conditional formatting) rather than inventing
one.

---

## Stage 7 — The independent fixes (each its own commit/PR)

These came out of testing real workbooks; they stand alone on upstream main.

1. **`parse_reference` last-`!` split** (`xlsx/src/import/worksheets.rs`):
   the shared-formula/context parser split `Sheet!A1` at the *first* `!`,
   but Excel forbids only `\ / ? * [ ] :` in sheet names — `!CAP` is legal
   and produced context `!CAP!B7`, which shredded. Rewrite with
   `s.rfind('!')`, then the simple col/row scan. Tests: `!CAP!B7`,
   `A!B!C!AS23`.
2. **Sheet-name quoting allow-list**
   (`base/src/expressions/utils/mod.rs`, `name_needs_quoting`): stringifying
   used a block-list of "bad" characters that missed `&`, `=`, `%`, … so
   `'SUMM_P&L'!H28` re-stringified unquoted → `#NAME?`. Invert to an
   allow-list: skip quotes only for names matching the identifier grammar
   (letter/`_`, then letters/digits/`_`/`.`), keep the existing A1/R1C1
   lookalike checks (a sheet named `A1` must be quoted). This is the
   *stringify-side* twin of fix 1's *parse-side*.
3. **`YEARFRAC` reversed dates**
   (`base/src/functions/date_and_time.rs`): Excel swaps start/end *before*
   the asymmetric 30/360 day-of-month adjustments (only the start day
   collapses 31→30 unconditionally); taking `abs()` at the end gives
   189/360 where Excel gives 190/360. Fix: `std::mem::swap` the serials
   right after parsing (also protects basis 1's year-range arithmetic from
   reversed multi-year inputs). Test both argument orders.

See `BRANCH_NOTES.md` for the upstreaming notes on these.

---

## Stage 8 (optional, branch-specific) — Perf plumbing

Only rebuild if you want the live-edit performance work too; it's not part
of the upstream proposal:

- `recompute_target_cells(&[(sheet,row,col)])`: public demand-driven
  recompute of a target list + its precedent cone, warm-starting from the
  grid, iterative-aware. The caller owns the staleness contract (cells
  outside the cone keep old values) — that's what makes it fast for
  formula-bar edits on huge books.
- Perf instrumentation: thread-local per-sheet eval counters
  (`eval_counting_start`/`eval_counting_take`, bumped on every memo-miss in
  `evaluate_cell`) and `IRONCALC_TRACE=1` timing/convergence traces in the
  iterative sweep and `compute_data_tables`. Thread-local so `&self` methods
  can bump without borrow gymnastics and it never appears in the public API.

---

## Verification checklist (run after every stage)

```
cargo test -p ironcalc_base
cargo test -p ironcalc              # xlsx crate
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

End-to-end sanity: import a real Excel file containing a 2-var table +
iterative calc, `evaluate()`, and diff every cached value in the file
against IronCalc's computed values (write a small example under
`base/examples/` or a throwaway test). This catches semantic drift no unit
test will.

## Review-defense drill

You should be able to answer these cold, without the code open:

1. Why does the override check live in `evaluate_cell` and not in
   `get_cell_value` or the range-reading code? *(Choke point: all three read
   paths funnel through it; `get_cell_value` is also used by the writer to
   read true values.)*
2. What happens if a scenario recompute panics — can an override leak?
   *(Install/clear live in one function; no grid state to restore either
   way. A panic aborts the evaluate anyway; the grid holds no scenario
   writes.)*
3. Why is there a settle pass, and what exactly does it fix? *(Persisted
   intermediates from the last scenario; downstream consumers of outputs.)*
4. Why is the per-scenario convergence test different from the sweep one?
   *(Mixed-kind targets; non-numerics can't converge numerically.)*
5. Why must new `Diff` variants go at the end? *(bitcode positional
   discriminant; serialized histories.)*
6. Why did import need the `t().is_none()` guard on the cached-`ca` branch?
   *(`<f t="dataTable" ca="1"/>` is empty and text-less too.)*
7. What's the complexity of evaluating a table, and what's the known-better
   design? *(O(scenarios × cone) + settle sweep; cone-scoped non-persisting
   eval needs reverse deps — the `TODO(perf)`.)*
8. Why are `#CIRC!` semantics untouched when `iterate` is off? *(Excel
   parity; iteration is opt-in via `<calcPr>`.)*
