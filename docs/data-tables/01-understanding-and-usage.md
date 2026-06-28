# Understanding Excel Data Tables (What-If Analysis) and the VEEV Model

**Audience.** An engineer who knows spreadsheets and formulas but has never used — or had to implement — Excel's *Data Table* feature. This document explains what a data table is, how a user creates and interacts with one, walks the real VEEV investment model end-to-end, states precisely what "calculable in the IronCalc engine" requires, and gives a terminology cross-walk so the same concept is recognizable across Excel, LibreOffice, and the OOXML file format.

This is Part 1 of a two-part deliverable. Part 1 is conceptual and example-driven; the companion document covers the engine implementation plan.

---

## 1. What a data table / what-if / multiple-operations IS

A **data table** is a built-in Excel sensitivity tool. You have one formula (or a small model behind one output cell) that depends on one or two **input cells**. You want to see what that output becomes for a *range* of input values — not by re-typing each value by hand 25 times, but in one grid that recomputes the whole model for every combination automatically.

Mechanically, a data table does exactly this: for each cell in a result grid, it temporarily substitutes a header value into a designated input cell, recalculates the model, reads the governing formula's result, writes that result into the grid cell, and restores the input cell. It is a *batch of "what if the input were X?" experiments*, laid out as a table.

The feature has three variants, distinguished by how many inputs vary and (for the one-input case) which way the input values run.

### Variant A — One-variable, column input

Input values run **down a column**. One or more governing formulas sit in the row directly above the body, each offset one column to the right of the input column. Each input value is poked into a single input cell; the formulas recompute.

```
                 f1        f2        <- governing formula(s), row ABOVE body
               +---------+---------+
   input_0     |  out    |  out    |   left neighbor = input value, fed into the input cell
   input_1     |  out    |  out    |
   input_2     |  out    |  out    |
               +---------+---------+
   ^               ^
   |               +-- result body (the data-table array)
   +-- column of input values, substituted one at a time into the input cell
```

### Variant B — One-variable, row input

The mirror image of A. Input values run **across a row**; governing formulas sit in the column directly to the left of the body, one per output row.

```
   (gov) | in_0    in_1    in_2    in_3     <- input values run across, row ABOVE body
  -------+-------------------------------
  (gov)  | out     out     out     out      <- one gov formula per output row, col to LEFT
  (gov)  | out     out     out     out
```

### Variant C — Two-variable

Now **two** inputs vary at once. A single governing formula sits in the **top-left corner**, one cell up and one cell left of the body. The **top header row** holds the values for the first (row) input cell; the **left header column** holds the values for the second (column) input cell. The body is the full cross-product: every (row value, column value) pair.

```
  (corner gov) | r_0    r_1    r_2    r_3    r_4    <- row-input header  -> input cell 1
  -------------+----------------------------------
        c_0    | out    out    out    out    out
        c_1    | out    out    out    out    out
        c_2    | out    out    out    out    out    <- body = result array
        c_3    | out    out    out    out    out
        c_4    | out    out    out    out    out
                 ^
   left col c_* -> input cell 2 (column input)
```

A key, often-surprising property of all three variants: **only the top-left cell of the result body actually carries the data-table definition.** Every other body cell is just a *cached value* — Excel does not repeat the metadata per cell, and the body behaves as one locked array (see §2).

---

## 2. How Excel users create and interact with data tables

A user builds a data table by hand-laying-out the geometry, then invoking one menu command.

1. **Lay out the block.** Put the governing formula (the output you want to sensitize) in the **top-left corner**. Put the row-input values across the **top row** to its right; put the column-input values down the **left column** below it. The empty rectangle to the lower-right is the result body.

2. **Select the entire block** including the headers and the corner formula.

3. **Data ▸ What-If Analysis ▸ Data Table…**

4. **Fill the dialog.** It has two fields:
   - **Row input cell** — the model cell each *top-row* value is plugged into.
   - **Column input cell** — the model cell each *left-column* value is plugged into.

   For a two-variable table, fill both. For a one-variable table, fill only one box (whichever orientation your values run).

5. **Excel array-enters the table.** Across the whole body it writes `{=TABLE(rowInput, colInput)}`. The body becomes a **locked array**: you cannot edit, move, or clear a single interior cell. Attempting it raises *"You cannot change part of a data table."* You may only edit the whole table as a unit, the input cells, or the headers. (`TABLE()` is not a real worksheet function — it is purely Excel's formula-bar display string for the data-table formula type. You cannot type it.)

6. **Recalc.** Data tables honor the **Calculation Options** setting. Because each table re-runs the whole model once per body cell, "Automatic Except for Data Tables" is a common choice — it keeps normal edits instant while deferring the expensive table refresh to an explicit F9.

**Array-locking is the most important interaction rule to internalize.** The result body is one indivisible object. It is created, cleared, and recomputed as a unit; single-cell edits are forbidden. An implementation must enforce the same protection.

---

## 3. The VEEV example, fully worked

The VEEV template (`VEEV_Template_v9.xlsx`) is a forward DCF / exit-multiple **equity valuation of Veeva Systems**. Its sheet **`Model_SENS`** (workbook sheet3, `rId3` → `xl/worksheets/sheet3.xml`) is a near-identical copy of the base `Model` sheet (rows 1–70) with three sensitivity tables bolted on below (rows 72–95). Crucially, the data-table input cells and the formula precedents all live **on `Model_SENS` itself**, pointing at that sheet's local copy of the model, so each table is self-contained on one sheet.

> **Correction to a prior brief.** An earlier statement of "known facts" said the workbook contains *exactly one* data table. That is wrong: `Model_SENS` contains **three** `t="dataTable"` formulas. The primary example below (Q75) is the one that brief described; the other two are real and any implementation must handle them. There is no legacy `TABLE()` syntax anywhere — all three are modern `t="dataTable"` array formulas.

### 3.1 The model being sensitized (finance terms)

`Model_SENS` projects 2023→2029, applies an exit EV/EBITDA multiple to derive an enterprise value, bridges to equity, divides by shares, and reports an **implied share price** (price target) and an **implied IRR** versus today's price. The cells that matter for the tables:

| Cell | Label | Role | Live value |
|------|-------|------|-----------|
| **B4** | Current Price | IRR denominator / upside base | 159.71 |
| **F8** | Revenue Growth % | **Input 1** (row input for Tables 1 & 3) | 0.14 |
| **G10** | Exit EV / EBITDA | **Input 2** (column input for Table 1) | 12.5 |
| **H9** | EBITDA Margin % (terminal) | **Input** (column input for Table 3) | 0.46 |
| **G58** | Implied Share Price | **Output 1** (price target) | ≈ 249.67 |
| **G61** | Implied IRR (NTM) | **Output 2** (annualized return) | ≈ 0.1943 |

- **G58 "Implied Share Price"** = `IFERROR(G56/Consensus!K15,"")` — implied equity value ÷ fully-diluted shares ⇒ a 2028 year-end price target.
- **G61 "Implied IRR (NTM)"** = `+(G58/$B$4)^(1/G62)-1` — annualized return from today's price (`$B$4`) to that target over the term `G62` (= `YEARFRAC(TODAY(), DATE(2028,12,31))`).

So the tables answer: **"If I change Revenue Growth and the Exit EV/EBITDA multiple (or the terminal EBITDA margin), what price target and IRR do I get?"** This is a textbook investment-research **sensitivity grid** — a "price-target / IRR football field."

### 3.2 The three tables

| # | Anchor | `<f>` attributes | Shape | Inputs |
|---|--------|------------------|-------|--------|
| 1 | **Q75** | `t="dataTable" ref="Q75:U79" dt2D="1" dtr="1" r1="F8" r2="G10" ca="1"` | 2-variable, 5×5 | row→F8, col→G10 |
| 2 | **W75** | `t="dataTable" ref="W75:X79" dt2D="0" dtr="0" r1="G10" ca="1"` | 1-variable, column input, 5×2 | col→G10 |
| 3 | **Q86** | `t="dataTable" ref="Q86:U90" dt2D="1" dtr="1" r1="F8" r2="H9" ca="1"` | 2-variable, 5×5 | row→F8, col→H9 |

### 3.3 Table 1 (Q75:U79) — the primary PT/IRR grid, fully dissected

This is the table from the known facts. Its on-sheet geometry:

```
            Q74        R74        S74        T74        U74    <- ROW header -> r1 = F8 (Rev Growth)
P74   =>   0.10       0.125      0.15       0.175      0.20
P75  10 |  Q75*       R75        S75        T75        U75
P76 12.5|  Q76        R76        S76        T76        U76
P77  15 |  Q77        R77        S77        T77        U77
P78 17.5|  Q78        R78        S78        T78        U78
P79  20 |  Q79        R79        S79        T79        U79
 ^ COL header -> r2 = G10 (Exit EV/EBITDA)
```

- **Corner / governing formula — P74:** `+TEXT(G58,"$0")&" / "&TEXT(G61,"0%")`, cached value `"$250 / 19%"`. It produces a single **"price / return%"** string. This is the one output the entire grid recomputes. (It is `t="str"` and `ca="1"`.)
- **Row-input header Q74:U74** = `0.10, 0.125, 0.15, 0.175, 0.20` — each substituted into **F8** (Revenue Growth). Q74 is a literal `0.1`; R74 = `+Q74+2.5%`; S74:U74 are a shared formula `+R74+2.5%`.
- **Column-input header P75:P79** = `10, 12.5, 15, 17.5, 20` — each substituted into **G10** (Exit EV/EBITDA), all literals.
- **Result body Q75:U79** (5×5 = 25 cells): **only Q75** carries the `<f t="dataTable" …/>`. The other 24 cells are cached `t="str"` `<v>` strings with **no `<f>`**. The cached grid:

| G10 \ F8 | 0.10 | 0.125 | 0.15 | 0.175 | 0.20 |
|---|---|---|---|---|---|
| **10**   | $197 / 9%  | $206 / 11% | $216 / 13% | $227 / 15% | $238 / 17% |
| **12.5** | $230 / 16% | $242 / 18% | $255 / 20% | $268 / 23% | $281 / 25% |
| **15**   | $264 / 22% | $278 / 25% | $293 / 27% | $309 / 30% | $325 / 33% |
| **17.5** | $298 / 28% | $314 / 31% | $331 / 34% | $349 / 37% | $368 / 39% |
| **20**   | $331 / 34% | $350 / 37% | $370 / 40% | $390 / 43% | $412 / 46% |

Read it as a financier would: at 15% revenue growth (column header) and a 15× exit multiple (row header), the model implies a **$293 price target and a 27% IRR** — versus the $159.71 current price, a clearly attractive setup. The grid lays bare how the thesis depends jointly on growth durability and exit multiple.

**Mechanically**, the cell at row-header `cv` and column-header `rv` is computed by: substitute `rv` into F8 and `cv` into G10 *without permanently changing them*; recalculate everything feeding P74 (→ G58, G61 → … → F8/G10); read P74's resulting string; write it into the body cell; restore F8/G10. Twenty-five such substitute-recompute-restore cycles fill the grid.

(Helper column **V75:V79** = `+P75…` just mirrors the column header beside the block for charting; it is not part of the array.)

### 3.4 Table 2 (W75:X79) — one-variable, two outputs

A column-input 1-variable table sharing the same Exit-multiple ladder (10…20) as Table 1's left column. Its two governing formulas sit in the row above the body:

```
        W                 X
74    +G59 (gov)       +G60 (gov)     <- two governing formulas, row above body
      ------------------------------
75   <DT> 0.0809        0.0623         (only W75 carries <f t="dataTable">)
76    0.0688           0.0530
77    0.0598           0.0461
78    0.0529           0.0408
79    0.0475           0.0365
```

`dt2D="0"` ⇒ one-variable; `dtr="0"` ⇒ the single input is the **column** input, so `r1="G10"` is fed from the left header column (here, V75:V79 = the multiple ladder). The two output columns come from two governing formulas: `+G59` (LFCF yield) and `+G60` (LFCF-SBC yield) at exit price. So one variable, two simultaneously-tabulated outputs.

### 3.5 Table 3 (Q86:U90) — growth × terminal margin

Same 2-variable shape as Table 1, but the column input is **H9** (terminal EBITDA Margin) instead of G10:

- **Corner P85** — identical formula to P74: `+TEXT(G58,"$0")&" / "&TEXT(G61,"0%")`.
- **Row header Q85:U85** = `0.10…0.20` → **F8** (Revenue Growth).
- **Column header P86:P90** = `0.42, 0.46, 0.50, 0.54, 0.58` → **H9** (terminal margin; P87 = `+H9`).
- **Body Q86:U90**: master at **Q86**, the other 24 cells cached strings.

### 3.6 Why these tables are volatile and the model is iterative

- **`ca="1"`** on every data-table master marks it **always-recalc / volatile**. This is standard for data tables: the result depends on re-running the whole model under substituted inputs, which dependency analysis cannot fully capture, so it can never be safely skipped on a partial recalc. (The corner formula also transitively depends on `TODAY()` via the IRR term, which is inherently volatile.)
- **`<calcPr calcId="191029" iterate="1"/>`** — **iterative calculation is ENABLED workbook-wide.** The model has genuine circular references *by design*: a debt-paydown / interest / cash / FCF loop (rows 31, 38, 55–56, 66–70 feed net interest → FCF → cumulative cash → back into financing), plus mutually-circular margin smoothers (`F9 = AVERAGE(E9,G9)`, `G9 = AVERAGE(F9,H9)`). Without iteration, Excel would throw a circular-reference error. The three sensitivity tables all sit downstream of this circular subgraph.

This last point is the crux for any reimplementation: **the grid is meaningless unless the engine can re-solve the circular model iteratively for each of the 25 substitutions.** A plain non-iterative re-evaluation errors on the circular references and the entire body becomes `#CIRC!`.

### 3.7 How the file stores it (round-trip facts)

- **Array-locked cells (carry `<f t="dataTable">`):** exactly Q75, W75, Q86 — the top-left cell of each block.
- **Cached-only cells (`<v>`, no `<f>`):** every other body cell.
- **Header / corner cells** are ordinary formulas/literals outside the `ref` range and round-trip like any other cell.
- The data-table masters are **absent from `calcChain.xml`** — Excel does not put `t="dataTable"` cells in the calc chain; they are recomputed by a dedicated post-pass *after* normal dependency-ordered calculation completes. Defined names exist but **none reference the table ranges** — the tables are geometry-driven, not name-driven.

---

## 4. Why this matters for Baba, and what "calculable" concretely requires

**Why Baba cares.** Baba builds investment-research models on IronCalc and consumes the engine as a local path dependency. VEEV-style sensitivity grids (price-target / IRR football fields) are a standard deliverable. Two needs follow: (1) *open* such workbooks without corruption and show the analyst's last-saved grid; and (2) eventually *author and recompute* these grids natively, so the tool can emit and refresh sensitivity blocks the way Excel does. The same construct also underpins a second consumer (l123 / Lotus `/Data Table`).

"Calculable in the IronCalc engine" decomposes into three capability tiers of increasing difficulty.

**(a) Read / round-trip — the minimum viable bar.**
Parse `<f t="dataTable" ref dt2D dtr r1 r2 ca>` on import and store it as a worksheet-level descriptor (anchor, body range, `dt2D`, `dtr`, `r1`, `r2`, volatility); keep the cached `<v>` body cells. On export, re-emit the master's `<f t="dataTable" …/>` plus the cached body values. This alone lets VEEV open and re-save without destroying the grid, and must cover all three observed shapes (`dt2D=1`, and `dt2D=0`/`dtr=0`).

**(b) Actually COMPUTE the body from the live model — the hard, valuable part.**
For each body cell the engine must, *without permanently mutating the input cells*:
1. Substitute the row/column header value(s) into `r1` (and `r2`).
2. **Recalculate the dependency cone** feeding the governing formula — and because this model is iterative, run the **iterative solver to convergence** (the interest/cash loop) for *each* substituted pair.
3. Read the governing formula's result and write it into the body cell.
4. Restore the inputs.

Concretely this requires: a re-entrant evaluator that can run with temporary input overrides; **iterative-calc support** (a fixed-point loop honoring the workbook's iteration count and epsilon); a data-table post-pass that runs *after* the main calc; and tolerance for **non-numeric results** — VEEV's corner output is a string (`TEXT(...)&"/"&TEXT(...)`), so the body holds strings, not just numbers. For VEEV specifically, tier (b) is gated on iterative calc: 25 full iterative re-solves per 2-variable table.

**(c) Author / edit new tables — UX parity.**
A "Data ▸ What-If ▸ Data Table" command with row/column input-cell fields; writing the master `<f t="dataTable">` into the top-left body cell; **enforcing array semantics** (no single-interior-cell edits; clearing/deleting affects the whole block); and recomputing when inputs or headers change. This is the inverse of import and is what lets Baba emit sensitivity blocks natively.

**The decisive engineering fact:** for the real VEEV file, a correct grid is impossible without iterative calculation. Tier (a) makes the file *survive*; tier (b) — and only with iterative calc — makes the numbers *true*.

---

## 5. Terminology / cross-walk

The same feature wears three names. When reading code or specs across ecosystems, these are the same thing:

| Concept | Excel (UI) | OOXML (file format) | LibreOffice Calc |
|---|---|---|---|
| Feature name | **Data Table** (under *What-If Analysis*) | `<f t="dataTable">` | **Multiple Operations** |
| Formula-bar display | `{=TABLE(rowInput, colInput)}` | (attributes only; empty body) | `=MULTIPLE.OPERATIONS(...)` (token `ocTableOp`) |
| Two-variable flag | two input cells filled | `dt2D="1"` | mode `Both` (5-arg form) |
| One-variable, row input | row input cell only | `dt2D="0" dtr="1"` | mode `Row` (3-arg form) |
| One-variable, column input | column input cell only | `dt2D="0" dtr="0"` | mode `Column` (3-arg form) |
| First input cell | Row input cell (2-var) | `r1` | input cell ref |
| Second input cell | Column input cell (2-var) | `r2` (only when `dt2D=1`) | second input cell ref |
| Always-recalc | volatile | `ca="1"` | `EXC_TABLEOP_RECALC_ALWAYS` |
| Defer table recalc | "Automatic Except for Data Tables" | `calcMode="autoNoTable"` | (calc-mode setting) |

**OOXML attribute reference** (the on-disk truth, validated against VEEV):

- `t="dataTable"` — selects the data-table formula kind; the `<f>` body is **empty** (no formula text).
- `ref` — the **result body** rectangle only (e.g. `Q75:U79`); excludes headers and the governing cell. The top-left of `ref` is the anchor, the sole cell carrying the `<f>`.
- `dt2D` — `1` ⇒ two-variable; `0` ⇒ one-variable.
- `dtr` — one-variable only: `1` ⇒ row-input, `0` ⇒ column-input. **Ignored when `dt2D=1`.**
- `r1` — two-variable: the **row** input cell; one-variable: the sole input cell.
- `r2` — two-variable only: the **column** input cell.
- `del1` / `del2` — set when the corresponding input cell was deleted (dangling ref); absent in VEEV.
- `ca` — always-recalculate / volatile.

**The subtle trap** to remember: the role of `r1` swaps between modes, and Excel always writes `dt2D=1 dtr=1 r1=<rowInput> r2=<colInput>`. A clean-room engine should **treat `r1` as row-input and `r2` as column-input whenever `dt2D=1`, and not branch on `dtr` in that case.**

A **structural difference** worth flagging for implementers: OOXML and Excel model the table as one object with a single anchor formula; **LibreOffice instead explodes it into N identical `ocTableOp` formula cells** and reconstructs the rectangle on export — which is why LibreOffice's `.xlsx` export of data tables is lossy (it writes them back as plain array formulas). IronCalc should keep the OOXML shape — a first-class descriptor on the worksheet — making import/export a near-1:1 field copy and avoiding that lossy reconstruction entirely.
