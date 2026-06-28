# PART 2 — IronCalc Data Tables (What-If / Multiple Operations): Implementation Design & Phased Plan

Status: FINAL (review-incorporated). Synthesizes R1–R7 and three adversarial critiques (calc-correctness, OOXML-roundtrip, architecture-fit). Decisions are flagged **DECISION**; alternatives are noted briefly. Where a critic was wrong, the rejection is noted inline and in §13.

Target file paths are given against the worktree checkout `base/src/…`, `xlsx/src/…`, with prior-art citations to the `data_table_first_attempt` branch and the `codex_data_table_research` / `claude_data_table_research` worktrees in `/Users/ddmoore/dev/IronCalc`. All engine line numbers were re-verified against `data_table_first_attempt` (which already contains `evaluate_with_data_tables`, `recompute_cells`, and the two-phase spill loop inside `evaluate_workbook_cells`) and against `main`.

> **Reading note:** §13 ("Resolved review issues") is the change-log against the draft. §14 ("Recommended path") and §15 ("Open questions for the maintainer") are the decision-ready summary. If you read nothing else, read §0, §13, §14, §15.

---

## 0. Scope, target, and the headline tension

Excel data tables ("What-If Analysis ▸ Data Table", LibreOffice "Multiple Operations") are a first-class array region whose body is computed by repeatedly substituting header values into one or two *input cells* and re-evaluating a *governing formula*. We want IronCalc to **read, round-trip, compute, and author** them — reusable by all IronCalc consumers and by Baba (which wants to *emit* VEEV-style sensitivity blocks) and l123 (Lotus `/Data Table`).

The motivating fixture, VEEV `Model_SENS` (sheet3), contains **THREE** data tables (R1/R7 corrected the prior "exactly one" fact):

| Anchor | `ref`     | `dt2D` | `dtr` | `r1` | `r2` | `ca` | Kind |
|--------|-----------|--------|-------|------|------|------|------|
| Q75    | Q75:U79   | 1      | 1     | F8   | G10  | 1    | 2-variable (Growth × Exit multiple) |
| W75    | W75:X79   | 0      | 0     | G10  | —    | 1    | 1-variable, column input, **two** governing formulas (PT yield, FCF-SBC yield) |
| Q86    | Q86:U90   | 1      | 1     | F8   | H9   | 1    | 2-variable (Growth × terminal margin) |

The headline tension: VEEV's workbook is `<calcPr calcId="191029" iterate="1"/>`. The governing formulas (P74 = `=TEXT(G58,"$0")&" / "&TEXT(G61,"0%")`) sit **downstream of an intentional circular ramp** (`F9=AVERAGE(E9,G9)`, `G9=AVERAGE(F9,H9)`, plus a debt/cash/interest loop). glm's doc quantified the blast radius: **32 `#CIRC!` cells** on Model_SENS cascade into `G58→""`, `G61→#VALUE!`, `P74→#VALUE!`. IronCalc today has **no iterative calc** (R2 §8): circular refs unconditionally yield `Error::CIRC` at the `CellState::Evaluating` guard in `evaluate_cell`. **Therefore data-table values for VEEV are gated on iterative calc.** We resolve this tension by *decoupling*: the data-table feature ships and is acceptance-gated on a synthetic non-circular fixture; VEEV-correct values are a joint milestone with iterative calc (P3). This is R5's framing and it is correct.

**Two scope clarifications forced by review (architecture-fit C4, ooxml C1/C3):**
- "Round-trip" means **value-faithful + stable-output + `<f>`-substring-equal**, *not* byte-faithful file equality. IronCalc re-serializes the whole sheet (string promotion, style renumbering, omitted defaults), so a VEEV file will differ in thousands of bytes outside the `<f>`. We never claim byte-faithful file equality (§6, §13/R-OX3).
- **Export of a stale cached body is a silent-wrong-answer hazard once cells are editable.** P0 is therefore **import + descriptor-preserving export, with export of the `<f>` re-emitted but the body left exactly as imported** — and we add a "dirty-table" guard so a header/input edit either recomputes (when compute exists) or the file is flagged. See §6 and §13/R-AF4.

---

## 1. Data model

**DECISION:** Store data tables as **worksheet-level first-class metadata** — `Worksheet.data_tables: Vec<DataTable>` — exactly mirroring the OOXML on-disk form (one descriptor per `<f t="dataTable">` anchor). This is the first-attempt/codex shape, validated by R2/R3/R4, and is strictly better than LibreOffice's "explode into N `ocTableOp` cells" (R6 §5), which is the root cause of LO's lossy XLSX export.

### 1.1 The struct (`base/src/types.rs`, after `conditional_formatting` at ~`:202`)

```rust
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct DataTable {
    /// Output body range in A1, unqualified (same sheet), e.g. "Q75:U79".
    /// Normalized to canonical `start:end` form at import (even for a 1×1 ref).
    pub range: String,
    /// dt2D — true ⇒ two-variable.
    pub two_dimensional: bool,
    /// dtr — one-variable orientation: true ⇒ row-input. Ignored when two_dimensional.
    pub row_oriented: bool,
    /// r1 — 2D: row input cell; 1D: the sole input cell. A1, may be sheet-qualified.
    pub r1: String,
    /// r2 — 2D only: column input cell. INVARIANT: r2.is_some() == two_dimensional.
    pub r2: Option<String>,
    /// del1 — r1 deleted/dangling. Round-tripped verbatim; suppresses recompute of that axis.
    pub input1_deleted: bool,
    /// del2 — r2 deleted/dangling. INVARIANT: false when !two_dimensional.
    pub input2_deleted: bool,
    /// ca — always-recalculate / volatile.
    pub calculate_always: bool,
}
```

**Field decisions vs prior art:**
- Both prior structs (R3/R4) had six fields and **omitted `del1`/`del2`**. R7 §1.1 documents them; LibreOffice (R6) bails the whole table on `del1`/`del2`. We **add `input1_deleted`/`input2_deleted`** so we can round-trip them verbatim. **Correctness behaviour changed by review (ooxml C1):** a deleted axis must **NOT** stamp `#REF!` over the cached body. Instead we *preserve the cached `<v>`* and *refuse to recompute* that table. See §3.4 and §9.
- **Refs stored as A1 strings, sheet-qualified-capable** (codex's win over first-attempt). Parsing deferred to compute time. Rationale (R2 §4): avoids re-indexing on structural edits; the cost is that refs don't auto-shift on row/col insert in v1 (handled in P4 — see §7).
- **`r2`/`del2` invariants enforced at the import boundary** (ooxml M9/O6): on import, force `r2 = None` and `input2_deleted = false` whenever `two_dimensional` is false, regardless of what attributes were present. This makes the export rule "emit `r2` iff `two_dimensional`" and codex's "emit `r2` iff `r2.is_some()`" provably equivalent.
- `range` is **normalized to canonical `start:end`** at import (ooxml H5): a degenerate single-cell `ref="Q75"` becomes `"Q75:Q75"` so `parse_range` (compute) and `range.split(':')` (export-anchor) never disagree.
- `Eq` is derivable (all fields are `Eq`); derive it (codex) so `DataTable` can be compared in tests.

**Alternative considered & rejected:** storing resolved integer `CellReferenceIndex` instead of strings. Rejected because indices must be re-mapped on every structural edit and on sheet rename; strings defer that and round-trip verbatim. We resolve to indices lazily in a `ResolvedDataTable` (§3.4) at compute time.

### 1.2 Serialization

`Worksheet` is `bitcode` `Encode/Decode` (R2 §4). `DataTable` derives both, so it round-trips through the native `.ic` format for free. **Add `#[serde(default)]`** on the `Worksheet.data_tables` field (R5 KTD3) so pre-feature `.ic`/JSON files deserialize to an empty vec — forward/backward compatible.

**Bitcode-enum positional hazard (architecture-fit M6 — valid and important):** the `Diff` enum (`base/src/user_model/history.rs:23`) is `#[derive(Encode, Decode)]` bitcode, whose enum discriminants are **positional**. The new `Diff::SetDataTable` variant introduced in §7 **MUST be appended at the very end of the enum** (after `SwapConditionalFormattingPriority`, currently the last variant at `history.rs:266`). Inserting it anywhere else shifts discriminants and breaks (a) `flush_send_queue`/`apply_external_diffs` wire-compat between mixed-version replicas (`common.rs:366,379`) and (b) any persisted undo history. Add a `flush_send_queue → apply_external_diffs` round-trip test carrying a data-table diff.

### 1.3 `new_empty` and all `Worksheet { … }` literals

Add `data_tables: vec![]` to `new_empty_worksheet` (`new_empty.rs:59-76`) and **every** `Worksheet { … }` literal. The checklist is `grep -rn "Worksheet {"` (R2 §6): known sites are `new_empty.rs`, `xlsx/src/import/worksheets.rs`, and test fixtures.

---

## 2. Representation decision: worksheet metadata vs parsed-formula node

**DECISION: worksheet-level descriptor (§1), NOT an AST `Node`/`ocTableOp` token.** Justification (R2 §9):

1. The `<f t="dataTable">` is a *pseudo-formula*: only the anchor carries it; the 24 (or 9) non-anchor body cells are **plain cached `<v>`** with no formula at all (R1 §2, R7 §3). Forcing this into the `Node` enum (`expressions/parser/mod.rs:143-259`, which has **no** `DataTable` variant) would pollute the grammar, the static analyzer, RC stringify, and `move_formula`, while still needing a side mechanism for the value-only cells. Net negative.
2. LibreOffice's `ocTableOp`-per-cell model (R6) buys partial-recalc via broadcast plumbing we don't have and don't need, and directly causes its lossy XLSX export (R6 §4). We keep a first-class object precisely to make export a near-1:1 field copy.
3. The first attempt already proved the descriptor + post-pass seam in ~303 lines (R3).

**Consistency between the anchor formula, the governing formula, and the cached body:**
- The **governing formula is NOT stored in the descriptor**; it is *located by geometry* (R5 KTD3, R7 §6 "derived"): 2-D ⇒ corner at `(ref.start.row-1, ref.start.col-1)`; 1-var column ⇒ one formula per output column in the row directly above `ref`; 1-var row ⇒ one per output row in the column directly left of `ref`. Never scan neighbours for "a formula" — derive the exact cell(s) from `ref`+`dt2D`+`dtr`.
- The **anchor body cell** (Q75) imports/holds its cached `<v>` like any value cell (`formula_index = -1`, R3). **The `dataTable` import arm only records the descriptor and MUST fall through to the normal cached-value path** (ooxml H4): it must not early-return, or the anchor's `<v>` (`"$197 / 9%"`) is dropped and Q75 imports empty. Test gate: `Q75 == "$197 / 9%"` after import (string).
- **Body cells are stored as plain values, never formulas** — both on import (already true) and after compute (`write_value` writes number/bool/string/error). They carry **zero** formula metadata (no `t="array"`, no `si`, no shared index — see §6/ooxml H7). This matches Excel and avoids the body re-triggering circular deps through itself (R3 `compute_data_tables` sequencing).

---

## 3. Calc engine integration — the core

This is the heart, and it is where the critiques landed hardest. The draft proposed (a) injecting the override in the single-reference read arm *and* the top of `evaluate_cell`, and (b) a "snapshot/restore of `self.cells`+`self.support` per body cell that doesn't pollute the memo." Both were shown to be wrong or self-contradictory (calc-correctness C1/C2; architecture-fit C3). The design below **collapses to a single override choke point + the proven `recompute_cells` clear**, and defines one solve primitive that can host iteration.

### 3.1 Where it hooks into `evaluate`

`Model::evaluate()` is a full recompute. The split below **already exists** in `data_table_first_attempt` (verified: `evaluate` at `model.rs:3034`, `evaluate_with_data_tables` at `:3046`, `evaluate_workbook_cells` at `:3075` containing the full two-phase spill loop). We adopt it as-is:

```rust
pub fn evaluate(&mut self) {                  // UNCHANGED default — no data tables
    self.evaluate_workbook_cells();           // FULL two-phase spill loop, minus CF
    self.evaluate_conditional_formatting();
}
pub fn evaluate_with_data_tables(&mut self) { // opt-in / explicit recalc
    self.evaluate_workbook_cells();
    self.compute_data_tables();
    self.evaluate_conditional_formatting();
}
```

**`evaluate_workbook_cells` already contains the ENTIRE spill two-phase loop** (calc-correctness C5 was a correct concern about a hypothetical split; the prior-art split honours it). Two consequences carried forward from C5:
1. **CF audit:** confirm no internal caller of `evaluate_workbook_cells` relied on CF running (audit `evaluate_if_not_paused` and tests asserting CF after `evaluate`). Verified: only `evaluate`/`evaluate_with_data_tables` call CF.
2. **`recompute_cells` does NOT re-run spill Phase 1.** It maps `evaluate_cell` over targets after a cache clear (verified `model.rs:3057`). Therefore **the governing cone must not contain a dynamic-array/spill dependency** in v1. **DECISION:** detect a spill cell in the governing cone and yield `#VALUE!` (documented limitation), rather than silently reading an un-repopulated spill. Re-running spill Phase 1 per substitution is deferred to a later perf/correctness phase.

**Rationale for keeping tables off the default path** (R3, R4 §3, R7 §4): Excel's `calcMode="autoNoTable"` excludes data tables from automatic recalc because each table is O(rows×cols × cone). **Reject codex's wiring** of `compute_data_tables` into the *default* `evaluate()`. R5/glm independently confirmed VEEV's table outputs are **leaves** (no cell references the table ranges), so deferring their recompute is free for consumers.

**Trigger policy (revised per calc-correctness C4 + architecture-fit M4/L5):**
- **v1 (P2):** `compute_data_tables` runs on **explicit recalc only** (`evaluate_with_data_tables`) and on authoring edits to a table's input/header cells (§7). Bindings do **NOT** auto-wire `evaluate_with_data_tables` into the keystroke recalc path (rejecting draft §8's back-door re-enable of the cost codex's default-wiring was rejected for).
- **When an auto-on-edit trigger is eventually added (post-P5, gated on cone-amortization §3.5):** a `calculate_always` (`ca="1"`) table MUST recompute on **every** recalc pass unconditionally — the "cone-changed?" optimization is skipped for `ca` tables, because `ca="1"` is precisely Excel's signal that the table cannot be dependency-tracked. The "cone-changed?" gate is reserved for **non-`ca`** tables and even then computed from **un-overridden (real) reads**, with branchy-governing-formula under-trigger documented. `calcMode` precedence: `calcMode=autoNoTable` suppresses auto recompute entirely; otherwise `ca="1"` forces recompute. The simplest safe default at the perf phase is "recompute all tables on the sheet" (cheap at VEEV scale, and `ca` makes "always" correct).

This satisfies `ca="1"` volatility *without building any volatile infrastructure* (R5): under explicit-recalc, every table recomputes whole, so `ca` is a no-op there.

### 3.2 The compute pass — substitution via read-time redirection, single choke point

**DECISION:** substitute inputs by **read-time address redirection** (LibreOffice `ReplaceCell`, R6 §2; R5 KTD1), **not** by mutating then restoring the input cell. Mechanism, corrected per calc-correctness C1/C6:

- Add a transient field on `Model`: `data_table_overrides: Option<HashMap<(u32,i32,i32), CalcResult>>`. `None` in the common case ⇒ **zero overhead** on the hot path.
- **Consult it at exactly ONE place: the first statement of `evaluate_cell`, immediately after `fetch_cell` and BEFORE the `SpillCell` branch** (verified: `SpillCell` match is the first branch at `model.rs:1421`, and a non-formula value cell falls straight through `get_formula()==None` to `get_cell_value`):

  ```rust
  pub(crate) fn evaluate_cell(&mut self, cell_reference: CellReferenceIndex) -> CalcResult {
      let original_cell = match self.fetch_cell(cell_reference) { … };
      if let Some(ov) = &self.data_table_overrides {        // FIRST — before SpillCell
          let key = (cell_reference.sheet, cell_reference.row, cell_reference.column);
          if let Some(v) = ov.get(&key) { return v.clone(); }
      }
      if let Cell::SpillCell { … } = original_cell { … }
      match original_cell.get_formula() { … }
  }
  ```

  **Remove the draft's second injection point in the `ReferenceKind` arm (`model.rs:563-588`) entirely.** That arm is *not* the universal read choke point: `evaluate_range`/`get_range` calls `evaluate_cell` directly per cell (verified `model.rs:1373`), and spill reads re-enter `evaluate_cell` for the anchor — both bypass the `ReferenceKind` arm. Placing the override only in `evaluate_cell` covers single refs, **ranges that straddle the input cell**, value-typed input cells, and spill anchors uniformly. Having it in two places invites divergence and double-counts `support`. (This was the single most load-bearing draft error; calc-correctness C1 is accepted in full.)

- A range that *straddles* an input cell is now substituted **correctly** at the per-cell level (each cell in the range funnels through `evaluate_cell`). Excel/LO nonetheless treat a range straddling the input as an error region; we **replicate LibreOffice's `IsTableOpInRange` guard** (R6 §2) and raise `#VALUE!` for the *whole table* when the governing cone reads the input cell **through a range** rather than as a single reference, matching Excel. Single-cell refs are fine. (Detecting straddle is a static scan of the governing AST for ranges containing `r1`/`r2`.)
- The input cell's stored content is **never written**. No save/restore of cell content, no broadcast storms, re-entrant-safe at the field level.

**Memo correctness (calc-correctness C1 second half — accepted):** `evaluate_cell` memoizes via `self.cells`. If we override F8=0.10, evaluate the cone, then override F8=0.125 without clearing, the memo returns 0.10-derived values for every body cell. **DECISION:** the per-substitution solve **clears the memo via the proven `recompute_cells` primitive** (clear `self.cells`/`self.support`/var-stack/lambdas, then evaluate the governing target). We **drop the draft's "snapshot/restore that preserves the memo across substitutions" framing entirely** — it is either stale (if it preserves) or equivalent to a clear (if it doesn't), as both C1 and architecture-fit C3 noted. The override map is the **only** carried state between substitutions.

### 3.3 The single solve primitive (hosts iteration)

To satisfy calc-correctness C3 (the seam must be able to host a fixpoint *before* P2 hardcodes a single pass), define ONE function that every body-cell solve goes through:

```rust
/// Solve the governing formula(s) at `targets` with `overrides` active.
/// Iteration-aware: the same primitive serves P2 (no iteration) and P3 (fixpoint).
fn solve_governing(
    &mut self,
    targets: &[CellReferenceIndex],
    overrides: HashMap<(u32,i32,i32), CalcResult>,
) -> Vec<CalcResult> {
    self.data_table_overrides = Some(overrides);
    let result = if self.iteration_enabled() {
        // P3: fixpoint. Seed each formula cell's FormulaValue from the prior pass,
        // run up to iterateCount passes, stop at max|Δ| < iterateDelta.
        // Overrides stay active across ALL passes of this one solve.
        // The seed lives in the Cell/memo; we clear the memo ONCE before the loop,
        // then RETAIN it across passes (the iterative loop owns the seed).
        self.solve_governing_iterative(targets)
    } else {
        // P2: today's behavior — clear caches once, single demand-driven pass.
        self.recompute_cells(targets)
    };
    self.data_table_overrides = None;
    result
}
```

Key invariants forced by the critique (C3):
- **The data-table loop clears the memo BETWEEN body cells** (via `recompute_cells` / once-before-loop), never *between iteration passes*. The iterative loop retains and seeds the memo across its own passes; the data-table loop owns the clear across substitutions. These two cache regimes were previously "fighting over the same cache" — this function fixes the ownership.
- **`support` entries created while an override is active are never trusted.** They are cleared with the memo. No incremental path (P4/P5) may rely on them. (calc-correctness C2(b) — accepted; stated as an explicit invariant.)
- **Header-value reads happen on the converged true model** (calc-correctness C7 — accepted): the data-table pass runs *after* `evaluate_workbook_cells` has converged, so header cells like P87=`=+H9` and the corner P74 (both possibly inside the circular cone) are read post-convergence. Pre-P3, `#CIRC!` headers → error substitution → error body is **expected**.

**P1 spike acceptance is strengthened** (calc-correctness O6): the spike must prove correctness on (a) a governing formula that reads the input cell **through a `SUM(range)`**, (b) a **second-level dependency** (governing reads X, X reads the input cell), and (c) prove the `solve_governing` signature can host a **circular** governing cone (fixpoint seam), not just `=row*col`.

### 3.4 Per-table dispatch (three layouts) and write-back

`resolve_data_tables()` parses `range` via `parse_range` (**verified** at `expressions/parser/mod.rs:76` returns `(left.column, left.row, right.column, right.row)` — col-first; destructure as `(left, top, right, bottom)`) and `r1`/`r2` via codex's `parse_reference_a1` + `split_sheet_reference` (sheet-qualified). Guards (R4): skip if `top<=1 || left<=1` (no header room), inverted range, unparseable ref. Malformed ⇒ skip (non-fatal), not abort.

**Deleted-axis handling (revised — ooxml C1):** if `input1_deleted` (or `input2_deleted`), **do NOT compute and do NOT overwrite the body** — leave the cached `<v>` intact and refuse to recompute the table. (The draft's "yield `#REF!` in the body" is **rejected**: it destroys the preserved cache and is value-lossy vs Excel, which keeps the last cached body even with `del1=1`.)

- **2-var** (`two_dimensional`): governing = corner `(top-1, left-1)`. Row header along `(top-1, c)` → `r1`; column header along `(r, left-1)` → `r2`. For each `(r,c)`: `solve_governing(&[corner], {r1: rowHeader[c], r2: colHeader[r]})`, write body. (VEEV Q75: r1=F8 row, r2=G10 col — R7 §1.2: **r1=row, r2=col when dt2D=1; do not branch on dtr**.)
- **1-var row** (`dt2D=0, dtr=1`): headers across `(top-1, c)` → `r1`; one governing per output row at `(r, left-1)`. Loop columns; one `solve_governing` of all row-governing targets per column. **Geometry is unvalidated against a real Excel file** (R7 §2 Layout 2, ooxml H6): mark as "geometry unverified" until a 1-var-row fixture is built *in Excel*. Do not advertise 1-var-row as "Excel-compatible" on symmetry alone.
- **1-var column** (`dt2D=0, dtr=0`): headers down `(r, left-1)` → `r1`; one governing per output column at `(top-1, c)`. Loop rows; one `solve_governing` per row. (VEEV W75 — note **two** governing formulas W74=`+G59`, X74=`+G60` feeding the two output columns.)

**Header values are computed values** (R7 §2 note on P87=`=+H9`): read the *evaluated* header cell, not its formula. A formula-valued header that is read as text would corrupt the next save's cache (ooxml M11) — covered by a dedicated test.

**Empty-header override value (calc-correctness O1 — accepted):** an empty header cell substitutes as **`CalcResult::Number(0.0)`** (Excel substitutes empty as 0). This differs observably in `=ISBLANK(F8)` and in `TEXT(...)` concatenation; VEEV's corner uses `TEXT(...)`, so the choice is load-bearing. Specify 0, add a test.

**Result write-back:** `write_value()` maps scalar `CalcResult` → value cell (`set_cell_with_number/boolean/error/string`). Non-scalar (Range/Array/Lambda) → `#VALUE!` (Excel disallows). **String results are first-class** (VEEV body is `"$197 / 9%"`). 
- **Style/number-format (R3 bug #10; ooxml O5; architecture-fit M5 — resolved):** **preserve each body cell's existing imported style/number-format; do NOT re-derive format from the `CalcResult` type.** Excel stores per-cell formats and the body cells were authored per-cell, so the imported cache style is authoritative. This resolves the draft's §3.4↔§9 contradiction in favour of "preserve". (The anchor's string style is correct for VEEV's string result; a synthetic numeric table authored with numeric body styles likewise carries the right format.)
- **Error origin (calc-correctness O4 — accepted):** when a header value is an error written into the override and it propagates to the body, **re-origin the error to the body cell** in write-back, so the body's `CalcResult::Error.origin` is not a misleading header address.

### 3.5 Ordering, downstream consistency, and tables-feeding-tables

With redirection there are no inputs to restore. Sequence (calc-correctness C2(a) — accepted; sequenced explicitly):
1. For each body cell: clear memo, set overrides, `solve_governing`, **collect** the scalar into a `Vec` (do NOT write into cells yet), clear overrides.
2. After the whole grid: write **all** body values as plain literals.
3. Clear memo.
4. One `evaluate_workbook_cells()` to re-propagate to any consumer of the body range.

Because step 1 only *collects* (never writes into cells), there is no snapshot/restore of the live memo and no race between body-write and re-propagate. For VEEV the tables are leaves so steps 3–4 are moot, but this closes the general stale-consumer hole (R3 bug #3) without the draft's unsequenced snapshot dance.

**Tables-feeding-tables (calc-correctness C8 + architecture-fit M3 — both accepted; draft was wrong):** the draft's "order by `ref` topological position" is **rejected** — spatial (grid) order is *not* dependency order (a table at A1 can depend on a table at Z99), and the engine has no topo sort (R2). **DECISION:** (a) detect at compute/authoring time whether a table's governing cone reads **another table's body range**; if so, either reject/warn (authoring) or (b) iterate the whole data-table pass to a **bounded fixpoint** (N passes, N = table count, stop when no body value changes). Default to (a)-reject for v1 simplicity; document (b) as the general fallback. Drop all "topological by ref" language.

**Re-entrancy (calc-correctness O3 — accepted):** the single `Option<HashMap>` field cannot nest. If a governing cone itself triggered `compute_data_tables` (table-feeds-table), it would clobber the override map. Because we **reject** table-feeds-table (above), nesting is forbidden by construction in v1. If nesting is ever allowed, `data_table_overrides` must become a **stack** (LibreOffice `m_TableOpList`, R6 §2). Documented, not built.

### 3.6 Volatile (`ca`) handling

No volatile infrastructure (R5). `calculate_always` is parsed and round-tripped. Under the v1 explicit-recalc trigger, every table recomputes whole, so `ca` is a no-op. `ca` only acquires teeth when an auto-on-edit trigger is added — see §3.1's revised trigger policy, which makes `ca` tables recompute unconditionally and reserves cone-gating for non-`ca` tables.

---

## 4. Iterative calculation interaction

VEEV is `iterate="1"` and its governing cone is genuinely circular (R1 §4, R2 §8, glm: 32 `#CIRC!` cells). **Without iterative calc, every governing eval returns `#CIRC!` and the body fills with errors** (R3 bug #1, R4 §6).

**DECISION:** iterative calc is a **separate workstream (P3)**, but — corrected per calc-correctness C3 — **the *shape* of the compute primitive (`solve_governing`, §3.3) is a prerequisite, designed from the start to delegate the per-substitution solve to either a single pass or a fixpoint.** P2 must NOT hardcode a single-pass primitive that P3 rips out. The *convergence implementation* is parallel; the *interface* is not.

- **Decouple acceptance:** the data-table feature's own gate (P0–P2) is a **synthetic non-circular fixture** (`=row_input * col_input` over a 3×3, plus the C3-mandated range/2nd-level cases). VEEV-correct values are explicitly a **joint 001+002 milestone**. "All-`#CIRC!` body" is **expected, not a failure** until P3.
- **Composition seam:** `solve_governing` keeps the override map active across **all** iteration passes of one body-cell solve; the seed lives in the `Cell`/memo and is **retained across passes** (cleared only between body cells). This is the exact gap the draft left open (C3) — now closed.
- **Iterative-calc scope (P3, from R5 plan 002 seed + R2 §8):**
  - Read `<calcPr iterate iterateCount iterateDelta calcId calcMode>` on import; emit them on export (currently dropped — `xlsx/src/export/workbook.rs:93` hardcodes empty `<calcPr/>`). **`calcPr` round-trip is iterative-calc's responsibility (P3), not data-tables'** (R5 plan 002 ownership boundary). Defaults: `iterateCount=100`, `iterateDelta=0.001`.
  - Relax the `CellState::Evaluating → #CIRC!` guard in `evaluate_cell` (verified verbatim) **only when iteration is enabled**: a re-entered `Evaluating` cell returns its *previous-pass cached value* (seed = 0/empty on first pass).
  - Wrap Phase 2 of `evaluate_workbook_cells` in a fixpoint loop seeding each formula cell's `FormulaValue` from the prior pass; stop when max |Δ| < `iterateDelta`.
  - **Header cells in the circular cone (calc-correctness C7):** because the data-table pass runs after the workbook solve converges, post-convergence headers (P74, P87) are correct; pre-P3 they are `#CIRC!`.
  - Open: SCC-scoped vs whole-workbook iteration; seeding (0 vs prior value); non-convergence + volatile (`TODAY()`). Carry to the P3 brainstorm (§15).

**Verdict:** ship P0–P2 without iteration; iteration is a prerequisite **only for VEEV-correct values** and lands as P3. Do **not** block the feature on it — but P2's primitive interface must already be the iteration-ready `solve_governing`.

---

## 5. Import (`xlsx/src/import/worksheets.rs`)

Three changes (R3/R4 — adopt codex's hardened version):

1. **Volatile-hint guard fix (required).** The branch that turns empty `<f ca="1"/>` into a volatile hint must also require `t.is_none()`, else VEEV's `<f t="dataTable" … ca="1"/>` is misclassified before reaching the dataTable arm:
   ```rust
   formula_node.attribute("t").is_none()
       && formula_node.attribute("ca") == Some("1")
       && formula_node.text().is_none()
       && !formula_node.children().any(|n| n.is_element())
   ```
2. **`"dataTable"` arm** (was `Err(NotImplemented(...))`, which aborted the whole VEEV import). Parse, **enforce invariants**, and push. **The arm records the descriptor only and FALLS THROUGH to normal cached-value handling** (ooxml H4 — do not early-return; the anchor's `<v>` must still be read):
   ```rust
   "dataTable" => {
       let raw_range = get_attribute(&formula_node, "ref")?.to_string();
       let range = normalize_to_canonical_range(&raw_range); // 1×1 → "X:X" (ooxml H5)
       let two_dimensional = formula_node.attribute("dt2D") == Some("1");
       let row_oriented    = formula_node.attribute("dtr")  == Some("1");
       let r1 = get_attribute(&formula_node, "r1")?.to_string();
       // Invariant: r2/del2 meaningful only in 2-D (ooxml M9/O6)
       let (r2, input2_deleted) = if two_dimensional {
           (formula_node.attribute("r2").map(str::to_string),
            formula_node.attribute("del2") == Some("1"))
       } else { (None, false) };
       let input1_deleted   = formula_node.attribute("del1") == Some("1");
       let calculate_always = formula_node.attribute("ca")   == Some("1");
       data_tables.push(DataTable { range, two_dimensional, row_oriented,
           r1, r2, input1_deleted, input2_deleted, calculate_always });
       // NB: do NOT return/continue — fall through so the anchor's cached <v> is read.
   }
   ```
3. **Robustness filters (codex):** `has_tag_name("row")` / `has_tag_name("c")` on row/cell iteration.

**Unparsed `<f>` attributes (ooxml O2):** `aca`/`bx`/`si` are schema-valid on a `dataTable` `<f>` but absent in VEEV. We do **not** capture them; document the drop (low risk).

**Cached body preservation:** the anchor (`formula_index=-1`) and the non-anchor body cells are plain `<v>` and flow through `get_cell_from_excel` as ordinary value cells — VEEV's string caches `"$197 / 9%"` import as shared/inline strings. **A freshly imported VEEV shows Excel's last-saved grid with no recompute** (R3/R4). This is the minimum viable bar and it works without iteration.

`calcPr` ingestion is **P3's** job (§4).

---

## 6. Export (`xlsx/src/export/worksheets.rs`)

**Adopt codex's anchor-only HashMap injection** (R4 §5 — the unique deliverable of all prior art; first attempt has none and *destroys* the table on save). LibreOffice itself cannot do this on the `.xlsx` path (R6 §4 `// OOXTODO`), so a correct round-trip is genuinely net-new value.

```rust
fn get_data_table_anchor(table: &DataTable) -> Option<(i32,i32)>  // range.split(':').next()
fn get_data_table_formula(table: &DataTable) -> String            // rebuilds <f .../>
```

`get_worksheet_xml` builds `HashMap<(row,col), &DataTable>` keyed by the **anchor** and injects the formula into the anchor `<c>` **before** the `<v>`, **in every `<c>`-emitting variant arm (~6: EmptyCell/Boolean/Number/Error/SharedString/`str`)** — ooxml/architecture-fit corrected: there is **no single reattach site**. The draft's "model on the array-formula reattach at `worksheets.rs:353-355`" is **rejected on two grounds:**
- `:353-355` is only the SharedString/`str` variant; array `<f>` injection is spread across ~6 variants. Patching one drops the `<f>` for the other five cell types (e.g. a numeric anchor like W75≈0.0809). Inject in **every** arm, as codex already does. (architecture-fit M2)
- A data-table body is **NOT an array formula.** Routing it through array-formula block tagging risks emitting `t="array"`/`ref`/`si` on body cells, which Excel rejects/reinterprets. Keep codex's anchor-only keying; body cells emit **plain `<v>` with zero formula metadata**. (ooxml H7 — the draft's "hardening" suggestion was a regression risk.)

```
<f t="dataTable" ref="{range}" dt2D="{0|1}" dtr="{0|1}" r1="{r1}"{ r2="..."}{ del1="1"}{ del2="1"}{ ca="1"}/>
```
Self-closing, empty body (assert no paired `…></f>` form — ooxml O4).

**Attribute order & emission (honest about uncertainty — ooxml C2/M9/M10):**
- For the three VEEV tables (no `del`), the emit order `ref dt2D dtr r1 [r2] ca` must be diffed against the **raw bytes** of VEEV `sheet3.xml` for Q75/W75/Q86 (not against codex's own expected-string constant, which is circular). This is a P0 verification task.
- Emit `r2` **iff `two_dimensional`** (now equivalent to `r2.is_some()` given the import invariant, §1.1).
- Emit `del1`/`del2` from the new fields **when present**. Their **positional order relative to `ca` is inferred from R7, not observed** (R7 could not see `del` in any real file). We document it as best-effort and do **not** claim byte-faithful `del` round-trip until a real Excel file with `del1` is obtained.
- `dt2D="0"`/`dtr="0"` are always emitted (codex). This is schema-valid and matches VEEV (which emits them explicitly) but cannot match an arbitrary Excel file that omitted defaults — which is fine, because we **do not claim byte-faithful file round-trip** (below).

**Honest round-trip bar (ooxml C3/O1 — the central scope correction):** IronCalc re-serializes the whole sheet (cell ordering, whitespace, `t="str"`↔`t="s"` string promotion, style renumbering, omitted defaults, `<dimension>`/`<cols>`). A real VEEV file will differ in thousands of bytes outside the `<f>`. The achievable, asserted bar is:
1. **Value-faithful:** every descriptor field equal after re-import; every body cached value equal **as a string**, regardless of `t="str"` vs `t="s"` representation.
2. **Stable-output:** IronCalc's own output is deterministic.
3. **`<f>`-substring-equal:** the exact `<f t="dataTable" …/>` string appears in the re-emitted XML.

**DECISION on string body cells (ooxml O1):** preserve `t="str"` for data-table body caches where IronCalc's storage allows, to minimize diff and match Excel — but value-faithfulness (same string) is the contractual bar; a `t="str"`↔`t="s"` flip is acceptable and tested-around.

**Governing-formula `ca` (ooxml H8 — flagged, out of scope):** P74/W74/X74 are ordinary formula cells carrying `ca="1"`. Whether IronCalc preserves `ca` on a *normal* `<f ca="1">…</f>` is a **pre-existing fidelity gap** owned by neither data-tables nor (cleanly) iterative-calc. Audit it; if `ca` is dropped on normal formulas, **document that the file is not byte-faithful for those cells** and that they lose volatility on Excel reopen. Do not silently absorb this into data-tables scope.

**Spill/data-table anchor collision (architecture-fit L3):** a data-table anchor must not also be a dynamic-array spill anchor (`cm="1"`). Add an import guard rejecting the pathological overlap.

---

## 7. Authoring API + undo (`user_model/`, `actions.rs`)

Expose create/edit/delete through **`UserModel`** (the undoable, diff-emitting wrapper used by wasm/python/web, `user_model/common.rs`). None of the prior art wired this (R3 bug #9); it's net-new.

**New `Diff` variant (architecture-fit C1 — accepted; was entirely omitted):** the `Diff` enum has **no descriptor variant**. Add, **at the END of the enum** (bitcode positional, §1.2):
```rust
SetDataTable {
    sheet: u32,
    anchor_row: i32,
    anchor_column: i32,
    old: Box<Option<DataTable>>,
    new: Box<Option<DataTable>>,
},
```
with matching arms in `apply_diff_list` and undo/redo (`history.rs` + `common.rs`). Without this, create/delete cannot restore `Worksheet.data_tables` on undo and would leave an orphaned/missing descriptor — a correctness hole, not polish.

**Recompute is NOT diffed (architecture-fit C2 — accepted; draft was self-contradictory):** the draft said body recompute must "route through the diff machinery" (§7) yet also "write plain values via `set_cell_*`" (§3.5). **Resolution: diff the *descriptor* (`SetDataTable`) and the *user-entered* header/input edits only.** Body recompute is a **derived, non-undoable recalc consequence**, written in place, **never diffed** — exactly how formula results already work in IronCalc (`set_cells_with_result` mutates `FormulaValue` without a diff, R2 §2). R3 bug #9 was about the *authoring* path bypassing undo, not the *recompute* path; diffing 25 body cells on every `ca` recalc would flood the undo log and resync the whole body on each F9. (Excel does not make data-table *recompute* an undoable action.)

**API surface (R5 KTD7):**
- `set_data_table(sheet, range, row_input: Option<&str>, col_input: Option<&str>)` — derives `dt2D`/`dtr` from which inputs are supplied (both ⇒ 2-var; only col ⇒ 1-var column `dtr=0`; only row ⇒ 1-var row `dtr=1`), validates geometry (header room, governing-cell present, **inputs on-sheet — reject off-sheet, matching Excel**), pushes a `DataTable` via `SetDataTable` diff, marks the body **array-locked**, and computes it.
- `delete_data_table(sheet, anchor)` — removes the descriptor (via `SetDataTable{new:None}`) and clears the body as a unit.
- `get_data_table(sheet, cell) -> Option<DataTable>` — resolves an **interior** cell to its table by scanning `data_tables` ranges for containment (architecture-fit L1). Used for UI display and the `{=TABLE(r1,r2)}` formula-bar string.

**Containment check (architecture-fit L1/L2 — budget honestly):** both the array-lock and `get_data_table(cell)` need a `cell_in_data_table(sheet,row,col) -> Option<&DataTable>` containment scan. There is **no reusable extracted array-formula lock predicate** to borrow (verified: `set_user_array_formula` at `common.rs:1949` stores array cells but exposes no `cell_in_locked_array` check). So budget for **writing the containment check from scratch**; define it once and share it between the lock and the formula bar.

**Array-lock semantics (Excel "You cannot change part of a data table", R1 §6):** `set_user_input` / clear / paste paths must call `cell_in_data_table` and **reject single-interior-cell edits**, allowing only whole-table replace/delete or edits to inputs/headers (which trigger recompute).

**Structural edits (`actions.rs`):** insert/delete row/col must shift a table's `range`/`r1`/`r2` (formula displacement lives at `actions.rs:291`). v1 (P0–P2) stores A1 strings **without** shifting (acceptable, R2 §4); P4 adds displacement so authored tables survive structural edits.

---

## 8. Bindings + UI

**Bindings (wasm/python/nodejs, `bindings/`):** expose `evaluate_with_data_tables`, `set_data_table`, `delete_data_table`, `get_data_table` through `UserModel`. **Do NOT auto-wire `evaluate_with_data_tables` into the keystroke recalc path** (architecture-fit M4 — accepted; the draft's "auto-call when any sheet has data_tables" was the rejected codex default-wiring re-entering through the back door, and would not even protect VEEV since its `calcMode` is absent=`auto`). v1: explicit method only. Auto-on-edit is deferred to a post-P5 phase gated on cone-amortization (§3.5) and obeys the revised `ca`/`calcMode` trigger policy (§3.1).

**Webapp UX (mirror Excel exactly, R1 §6):**
- New **Data menu** ("Data ▸ What-If Analysis ▸ Data Table…") in `FileBar.tsx`.
- A **`DataTableDialog`** on the existing Modal/`Prompt` kit with two reference fields — **Row input cell** and **Column input cell** — each with a selection-capture button (reuse the `EditNamedRange` pattern). Fill one for 1-var, both for 2-var.
- The selected block is array-entered; the body renders **array-region-styled** and protected against single-cell edits, surfacing Excel's "You cannot change part of a data table" message on attempts.
- Formula bar shows `{=TABLE(r1,r2)}` for body cells (R7 §5; resolved via `get_data_table(cell)`): 2-var `{=TABLE(F8,G10)}`; 1-var column `{=TABLE(,G10)}`; 1-var row `{=TABLE(F8,)}`.
- Edit-mode resizing of an existing table is deferred (acceptable UX hole for v1).

---

## 9. Edge cases & errors

| Case | Decision (sources: R6 §5, R7 §3–4, critiques) |
|------|----------------------------------------------|
| **Deleted input cell (`del1`/`del2`)** | Round-trip the flag verbatim; **preserve the cached body, refuse to recompute** the table. **Do NOT stamp `#REF!` over the cache** (ooxml C1 — reversed from draft). Synthetic `del1="1"` fixture required (none in VEEV). |
| **Off-sheet input cells** | Excel **forbids** input cells on another sheet. **Authoring API rejects** off-sheet inputs (§7); import/compute *tolerates* sheet-qualified `r1`/`r2` (codex `split_sheet_reference`) for robustness. |
| **Single-cell (1×1) `ref`** | Normalize `range` to `"X:X"` at import so compute and export agree (ooxml H5). Add to test matrix. |
| **Error/#REF in governing formula** | Per-cell: that body cell gets the error (re-origined to the body cell, §3.4); other cells unaffected. |
| **Circular refs in governing cone** | Without iteration ⇒ `#CIRC!` in body (expected pre-P3). With iteration ⇒ solve to convergence per substitution via `solve_governing` (§3.3/§4). |
| **Range straddling an input cell** | Per-cell substitution is now correct (override in `evaluate_cell`), but to match Excel we reject the table with `#VALUE!` when the governing cone reads the input **through a range** (`IsTableOpInRange`, R6). Single-cell refs fine. |
| **Spill/dynamic-array in governing cone** | `recompute_cells` does not re-run spill Phase 1 ⇒ **detect & reject with `#VALUE!`** in v1 (calc-correctness C5). Documented limitation. |
| **Empty/partial body, table at sheet edge** | Skip (need header row/col outside body) — non-fatal. |
| **Empty header cell** | Substitute as `CalcResult::Number(0.0)` (Excel: empty→0; calc-correctness O1). |
| **Header cell is itself a formula/error** | Substitute its **computed value** (read post-convergence); error header ⇒ error body (add test; ooxml M11). |
| **Body/anchor style after recompute** | **Preserve the imported per-cell style; do NOT re-derive from `CalcResult`** (R3 bug #10 resolved toward preserve; ooxml O5, architecture-fit M5). |
| **Tables-feeding-tables / nested** | Detect "governing cone reads another table's body" and **reject** (v1); general fallback is a bounded fixpoint pass (calc-correctness C8, architecture-fit M3). **Drop "topological by ref".** |
| **Re-entrant override** | Forbidden by the reject-nesting rule; single `Option<HashMap>` never nests. Stack only if nesting is ever allowed (calc-correctness O3). |
| **Array/Lambda governing result** | Coerce to `#VALUE!` (Excel disallows). |
| **Anchor also a spill anchor** | Reject at import (architecture-fit L3). |
| **`INDIRECT`/`OFFSET` landing on input cell** | Redirected correctly because it resolves to `(sheet,row,col)` then funnels through `evaluate_cell`; a string-built `INDIRECT` the override can't influence is a known divergence — document (calc-correctness O2). |

---

## 10. Test strategy

**Unit / engine tests (`base/src/test/test_data_table.rs`):** assert **computed** body values (not echoed cache):
- 1-var **column** (`=B1*2` → 2,4,6); 1-var **row** (the under-tested orientation, R3); 2-var (`A1+B1` → 11,21,12,22) — each asserting **inputs are untouched** (trivially true with redirection).
- **C1/C3-mandated:** a governing formula reading the input **through `SUM(range)`**; a **second-level** dependency; an **override on a value-typed input cell** AND on a **formula-typed input cell** (both honor the override — calc-correctness C6).
- **Memo-invariance (architecture-fit M1):** assert `self.cells`/`self.support` are empty/clean after a table pass; assert `data_table_overrides == None` on the hot path (zero-overhead claim).
- Error-header propagation (re-origined to body); range-straddling-input rejection; spill-in-cone rejection; deleted-input cache-preservation (no `#REF!` stamp); empty-header=0; string results (`TEXT(...)`).
- **Multi-table single-sheet ordering** (3 tables on one sheet, real Model_SENS geometry).

**xlsx round-trip (`xlsx/tests/test_data_table.rs`):**
- import → evaluate → export → re-read XML: **assert ALL THREE** `<f>` strings (ooxml M12): Q75 (`dt2D=1 dtr=1 r1=F8 r2=G10`), W75 (`dt2D=0 dtr=0 r1=G10`, **no r2**), Q86 (`dt2D=1 dtr=1 r1=F8 r2=H9`). The W75 case guards against emitting `r2` for a 1-var table.
- **Raw-byte diff** of codex's emitted `<f>` against VEEV `sheet3.xml` bytes for Q75/W75/Q86 (not against codex's own constant — ooxml C2).
- import → export → **re-import**: assert `DataTable` struct value-equal (codex gap; closes re-import leg).
- **Anchor cached value survives** (ooxml H4): `Q75 == "$197 / 9%"` (string) after import — wire as a **P0 gate**.
- **Body cell has no `<f>`** (ooxml H7): assert R75/S75 contain no `<f` substring; assert no paired `…></f>` (O4).
- **Body value survives regardless of `t="str"`/`t="s"`** (ooxml C3/O1).
- **Synthetic `del1="1"` fixture** (hand-edit a VEEV copy): import→export, assert `del1` round-trips and **body strings survive** (ooxml C1, architecture-fit L4).
- **Formula-valued header** round-trip (P87=`=+H9`): body uses computed value; re-import cache is the value, not the formula text (ooxml M11).
- **`flush_send_queue → apply_external_diffs`** carrying a `SetDataTable` diff (architecture-fit M6).

**VEEV-derived fixture:** carve `Model_SENS` (or a reduced copy) into `xlsx/tests/` exercising all three table shapes; assert cached values (`Q75=="$197 / 9%"`, `W75≈0.08088…`) at P0 (glm's regression fixture — "would have caught the first attempt's breakage").

**Iterative-calc cases (P3):** the synthetic circular ramp (`F9↔G9`) + a data table over it; assert convergence to Excel's cached values within `iterateDelta`; assert non-convergence behavior. glm's **32-`#CIRC!`-cell** target is the sharp convergence test.

---

## 11. Phased rollout

Each phase = one reviewable PR (or a small stack). Built on a **new branch off `main`** (the first-attempt branch no longer compiles against main per glm: missing `Color` import in `xlsx/src/import/worksheets.rs` after the theme-color refactor). Reuse prior art by cherry-pick, not by branching off it.

| Phase | Deliverable | Files | Acceptance criteria | Upstream vs Baba |
|-------|-------------|-------|---------------------|------------------|
| **P0 — Model + import + descriptor-preserving export** | `DataTable` struct (+ invariants, range-normalize) + `Worksheet.data_tables`; import parse (fall-through to value, volatile guard fix, robustness filters); **anchor-only** export injection (all ~6 variants); `new_empty`/serde. **No compute; body left as imported.** Dirty-table guard (below). | `types.rs`, `new_empty.rs`, `xlsx/src/import/worksheets.rs`, `xlsx/src/export/worksheets.rs` | VEEV opens without corruption; all 3 `<f>` strings round-trip (substring + raw-byte diff); `Q75=="$197 / 9%"`; re-import struct value-equal; body cells have no `<f>`; `del1` synthetic fixture round-trips with cache intact. **Value-faithful, NOT byte-faithful.** | **Upstream.** Pure fidelity. |
| **P1 — Compute spike + `solve_governing` seam** | `recompute_cells` (exists in prior art); `solve_governing` primitive; override field; **PoC spike** of single-choke-point redirection. | `model.rs`, new `base/src/data_table.rs`, `lib.rs` | Spike proves redirection correct on (a) `=row*col`, (b) input read **through `SUM(range)`**, (c) **2nd-level dependency**, and (d) the `solve_governing` signature can host a fixpoint over a circular cone. Memo/`support` clean after pass; override `None` on hot path. | Upstream. |
| **P2 — Compute on explicit recalc** | `compute_data_tables` (all 3 layouts) via `evaluate_with_data_tables`; collect-then-write-then-repropagate sequencing; array-lock read path; reject spill-in-cone / range-straddle / nested. | `data_table.rs`, `model.rs`, `user_model/common.rs` | Synthetic non-circular fixtures compute correct values; inputs untouched; default `evaluate()` unchanged; outputs plain values with preserved style; downstream consumers fresh; 3-table ordering test passes. **VEEV body = `#CIRC!` is acceptable here.** | Upstream. |
| **P3 — Iterative calc (joint)** | `calcPr` ingest/emit; `Evaluating→prev-value` under iteration; fixpoint loop in `evaluate_workbook_cells`; `solve_governing_iterative`. | `xlsx/src/{import,export}/workbook.rs`, `model.rs`, `data_table.rs` | Synthetic circular ramp converges; **VEEV's 3 tables reproduce Excel's cached values within `iterateDelta`**; 32-`#CIRC!` target resolves. | Upstream (iterative calc broadly wanted); VEEV correctness is the Baba-driving milestone. |
| **P4 — Authoring API + `SetDataTable` diff + structural edits** | `SetDataTable` `Diff` (appended); `set/delete/get_data_table`; array-lock write enforcement; containment check; row/col displacement of `range`/`r1`/`r2`. Body recompute non-diffed. | `user_model/{common,history,undo_redo}.rs`, `actions.rs` | Create/edit/delete undoable; single-interior-cell edits rejected; tables survive row/col insert/delete; `flush→apply` diff round-trips; **Baba can emit a VEEV-style block over non-circular inputs** (full circular requires P3). | Authoring is **Baba/l123-driving** but upstreamable. |
| **P5 — Bindings + Web UI + perf** | wasm/python/nodejs exposure (explicit method, no auto-wire); `DataMenu` + `DataTableDialog`; cone-amortization per table (`aLastTableOpParams`). | `bindings/*`, `webapp/*`, `data_table.rs` | Excel-parity dialog; array-region display + edit protection; `{=TABLE()}` formula bar; large-table perf acceptable. | Upstream UI; Baba uses bindings. |

**Dirty-table guard (P0, architecture-fit C4 — new):** P0 re-emits the `<f>` with the body **exactly as imported**. To avoid shipping a silent-wrong-answer path once cells are editable, add a guard: if any header/input/governing-cone cell of a table is edited in a P0-only build (no compute), either (a) block the edit with "data table recompute not yet supported" or (b) mark the table descriptor `stale` and surface a warning on save. (a) is simplest for P0; (b) becomes moot at P2 when compute lands. **The export-of-stale-cache standalone phase is thus made safe**, addressing C4 without delaying fidelity value.

**Minimum shippable to upstream:** P0 alone (fidelity, value-faithful) is independently valuable — ship first. P2 computes the common (non-circular) case. P3 unblocks VEEV. P4/P5 are authoring + parity.

**Baba-payoff ordering note (architecture-fit C5):** Baba's value (emit a VEEV-style block that *also* sits over the circular ramp) is gated on **P3 then P4**. If Baba is the sponsor, add an explicit **P3.5 "Baba authoring over non-circular tables"** milestone so Baba sees a programmatic-emit win before the full iterative+authoring stack lands. The authoring API lives in **OSS `UserModel`** (the right call for reuse) — so Baba's "driving" feature is itself an upstream contribution; make that boundary explicit in the PR description.

---

## 12. Open questions / risks for the maintainer

Superseded by the consolidated §15. Retained heading for the draft's 12-area structure.

---

## 13. Resolved review issues

What changed because of the three critiques. **[A]=accepted, [R]=rejected with reason.**

**Calc-correctness:**
- **R-CC1 [A] (CRITICAL):** Single override choke point at the **top of `evaluate_cell`**, before the `SpillCell` branch. **Removed** the draft's `ReferenceKind`-arm injection (ranges/spills bypass it → silent stale reads). §3.2.
- **R-CC2 [A]:** Memo must be **cleared per substitution** via `recompute_cells`; **dropped** the "snapshot/restore that preserves the memo" framing (stale-or-equivalent-to-clear). §3.2/§3.3.
- **R-CC3 [A] (CRITICAL):** Explicit collect → write-all → clear → re-propagate sequencing; no live-memo snapshot; `support` entries under override are never trusted (stated invariant). §3.5.
- **R-CC4 [A] (CRITICAL):** Iteration is a **prerequisite for the primitive's *interface***. Introduced `solve_governing` (single-pass or fixpoint) so P2 can't hardcode a single pass P3 rips out; override stays active across iteration passes; seed retained across passes, cleared only between body cells. §3.3/§4.
- **R-CC5 [A]:** `ca` auto-trigger contradiction resolved — `ca` tables recompute unconditionally; cone-gating reserved for non-`ca`; v1 is explicit-recalc-only. §3.1/§3.6.
- **R-CC6 [A]:** `recompute_cells` doesn't re-run spill Phase 1 → **detect & reject spill-in-governing-cone** (`#VALUE!`), v1. §3.1/§9.
- **R-CC7 [A]:** Override gate is the **first** statement (before `SpillCell`), so value-typed and formula-typed input cells both honor it; test both. §3.2/§10.
- **R-CC8 [A]:** Header reads occur post-convergence; pre-P3 `#CIRC!` headers→error body is expected and documented. §3.3/§4.
- **R-CC9 [A]:** **Dropped "topological by ref"**; reject table-feeds-table (v1) or bounded fixpoint (general). §3.5/§9.
- **R-CC10 [A]:** Empty header → `Number(0.0)`; error re-origined to body cell; nesting forbidden (no stack in v1). §3.4/§9.

**OOXML round-trip:**
- **R-OX1 [A] (CRITICAL):** Deleted axis **preserves cached body, refuses recompute** — **does NOT stamp `#REF!`** (reversed from draft). Synthetic `del1` fixture added. §1.1/§3.4/§9/§10.
- **R-OX2 [A]:** `del1`/`del2` positional order is **inferred, not observed**; documented best-effort, no byte-faithful `del` claim. Raw-byte diff of the three VEEV `<f>` strings added. §6/§10.
- **R-OX3 [A] (CRITICAL):** Round-trip bar restated as **value-faithful + stable-output + `<f>`-substring**; **byte-faithful file equality dropped** everywhere. §0/§6/§10.
- **R-OX4 [A]:** Import arm **falls through** to read the anchor's cached `<v>`; `Q75=="$197 / 9%"` is a P0 gate. §2/§5/§10.
- **R-OX5 [A]:** `range` normalized to canonical `start:end` (1×1 case). §1.1/§5.
- **R-OX6 [A]:** 1-var-**row** geometry marked **unverified** until an Excel-built fixture exists. §3.4/§10.
- **R-OX7 [R→corrected]:** Draft's "model export on array-formula reattach" **rejected** — body cells must carry zero formula metadata; keep codex's anchor-only injection across all ~6 variants. §6.
- **R-OX8 [A]:** `r2`/`del2` invariants enforced at import (None when 1-var); makes export rule equivalent. §1.1/§5.
- **R-OX9 [A]:** Assert **all three** VEEV `<f>` strings, not just the 2-var. §10.
- **R-OX10 [A, flagged out-of-scope]:** Governing-formula `ca` (P74/W74/X74) preservation is a pre-existing gap; audit + document, not absorbed into data-tables. §6.

**Architecture-fit:**
- **R-AF1 [A] (CRITICAL):** Added `Diff::SetDataTable` variant (appended at enum end, bitcode-positional). §1.2/§7.
- **R-AF2 [A] (CRITICAL):** Body recompute is **non-diffed** (recalc consequence); only descriptor + user input edits are diffed. Resolves §7↔§3.5 contradiction. §7.
- **R-AF3 [A] (CRITICAL):** §3.2/§3.3 converged on **one** primitive (`recompute_cells` clear + override map as only carried state); snapshot/restore-vs-clear contradiction removed. §3.2/§3.3.
- **R-AF4 [A] (CRITICAL):** P0 export-of-stale-cache made safe via a **dirty-table guard** (block edit or mark stale). §11.
- **R-AF5 [A]:** P3 made a hard predecessor of the Baba milestone; added optional **P3.5** for non-circular Baba authoring; authoring API confirmed in OSS `UserModel`. §11.
- **R-AF6 [A]:** Export injection is **per-variant (~6 sites)**, not one line; rejected the single-line reattach hint. §6.
- **R-AF7 [A]:** Style **preserved**, not re-derived from `CalcResult`. §3.4/§9.
- **R-AF8 [A]:** Bindings do **not** auto-wire `evaluate_with_data_tables`; explicit method only (v1). §8.
- **R-AF9 [A]:** Added memo-invariance / override-`None` / 3-table-ordering / structural-edit tests. §10.
- **R-AF10 [A]:** `Diff` bitcode positional hazard documented; append-only + `flush→apply` test. §1.2/§7/§10.
- **R-AF11 [A]:** Containment check budgeted as **new code** (no reusable array-lock predicate); shared by lock + formula bar. §7.

**Nothing material from the three critiques was rejected outright.** The two "rejections" (R-OX7, and the draft's own topological-ordering/snapshot claims) are rejections of the *draft's* positions in favour of the critics'. The critics were uniformly correct on the load-bearing issues (override choke point, memo clear, diff variant, byte-vs-value fidelity, deleted-axis preservation).

---

## 14. Recommended path (decision-ready)

**Ship in five phases off a fresh branch from `main`; cherry-pick prior art, never branch off `data_table_first_attempt` (it no longer compiles).**

1. **P0 — Fidelity (upstream, low-risk, ship first).** Descriptor + invariants + canonical-range; import (fall-through, guard fix); anchor-only export across all variants; serde default; dirty-table guard.
   - **Accept when:** VEEV imports without corruption; all three `<f>` strings present (substring + raw-byte diff); `Q75=="$197 / 9%"`; re-import struct value-equal; body cells have no `<f>`; synthetic `del1` round-trips with cache intact. **Value-faithful, not byte-faithful.**

2. **P1 — Compute seam spike.** `solve_governing` + single-choke-point override + `recompute_cells`.
   - **Accept when:** redirection is correct for `=row*col`, input-through-`SUM(range)`, and a 2nd-level dependency; the primitive's signature provably hosts a fixpoint over a circular cone; memo/`support` clean after the pass; override `None` on the hot path.

3. **P2 — Compute on explicit recalc (upstream).** All three layouts via `evaluate_with_data_tables`; collect→write→repropagate; array-lock read path; reject spill-in-cone / range-straddle / nested.
   - **Accept when:** synthetic non-circular fixtures (1-var col, 1-var row, 2-var) compute correct values with inputs untouched and styles preserved; default `evaluate()` unchanged; 3-table single-sheet ordering test passes; **VEEV body `#CIRC!` is acceptable.**

4. **P3 — Iterative calc (joint, upstream; unblocks VEEV).** `calcPr` round-trip; relaxed `Evaluating` guard under iteration; fixpoint loop; `solve_governing_iterative`.
   - **Accept when:** synthetic circular ramp converges; **VEEV's three tables reproduce Excel's cached values within `iterateDelta`**; the 32-`#CIRC!` target resolves.

5. **P4 — Authoring + undo + structural edits (Baba-driving, upstreamable).** `SetDataTable` diff (appended); `set/delete/get_data_table`; array-lock write enforcement; containment check; row/col displacement. *(Optional P3.5 before P4 for non-circular Baba authoring.)*
   - **Accept when:** create/edit/delete undoable; single-interior-cell edits rejected; tables survive structural edits; `flush→apply` carries the diff; Baba can emit a block programmatically (full circular needs P3).

6. **P5 — Bindings + Web UI + perf.** Explicit binding methods; `DataMenu`/`DataTableDialog`; cone-amortization.
   - **Accept when:** Excel-parity dialog; array-region display + edit protection; `{=TABLE()}` formula bar; acceptable large-table perf.

---

## 15. Open questions for the maintainer

1. **Compute primitive default if the P1 spike is marginal.** Single-choke-point redirection + `recompute_cells`-clear is the design. If the spike shows it's too costly at scale, do you want mutate-restore (proven, but the upstream-rejected pattern) as the fallback, or block on cone-amortization first?
2. **Cone-amortization timing.** Per-body-cell memo clear is O(cells × bodycells) — fine at 5×5, needs `aLastTableOpParams`-style amortization (memoize only cone cells that don't transitively depend on an input) for large tables. Defer to P5, or design it into P2?
3. **Iterative-calc ownership & scope (P3).** SCC-scoped vs whole-workbook iteration; seed (0 vs prior value); non-convergence + volatile (`TODAY()`); `calcMode` precedence. R5 plan 002 is a *seed*, not a contract — needs its own brainstorm. Confirm `calcPr` round-trip belongs to P3.
4. **Auto-on-edit trigger policy (post-P5).** Confirm: `ca` tables recompute unconditionally; non-`ca` cone-gated from real reads; `calcMode=autoNoTable` suppresses; no auto-wire in v1 bindings. Is "recompute all tables on the sheet" an acceptable simple default at the perf phase?
5. **Governing-formula `ca` (H8).** Does IronCalc preserve `ca="1"` on *normal* `<f ca="1">…</f>` cells today? If not, VEEV exposes a pre-existing volatility-fidelity gap (P74/W74/X74). Owned by whom — iterative-calc, or a standalone fidelity fix?
6. **`t="str"` vs `t="s"` for body caches.** Preserve `t="str"` to minimize diff (recommended), or accept promotion to shared strings? Value-faithfulness holds either way; this is a diff-minimization call.
7. **Deleted-axis & 1-var-row fixtures.** No real Excel file with `del1`/`del2` or a 1-var-row table exists in our corpus. Can you (or Baba) produce Excel-authored fixtures so geometry/order are validated against ground truth rather than inferred?
8. **Baba boundary.** The authoring API lands in OSS `UserModel` (reuse-correct), so Baba's driving feature is an upstream contribution. Confirm that's the intended boundary, and whether P3.5 (non-circular Baba authoring) is worth carving out for an earlier Baba win.
9. **Tables-feeding-tables.** Reject at authoring/compute (v1 simplicity) vs bounded-fixpoint (general)? Rare in practice; confirm the v1 reject is acceptable.
