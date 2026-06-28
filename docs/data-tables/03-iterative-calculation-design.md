# PART 3 — Workbook-Level Iterative Calculation for IronCalc: Implementation Design & Phased Plan

Status: **FINAL (review-incorporated).** Companion to `docs/data-tables/02-implementation-design.md`
(this is that plan's **P3**). Synthesizes IC1 (engine internals), IC2 (Excel semantics), IC3
(LibreOffice reference), IC4 (algorithm options), IC5 (composition contract), and two adversarial
critiques (`ic_critique_convergence.md`, `ic_critique_excel-compat-perf.md`).

> **Reading note.** §0 (scope + the branch-reality correction), §3 (the algorithm), §4 (composition),
> §12 (Resolved review issues), §13 (Recommended path), §14 (Open questions) are the decision-ready
> core. **DECISION** flags a chosen option; **[A]/[R]** in §12 mark accepted/rejected critique points.
> Where a critic was wrong, the rejection is stated with a reason.

> **A note on the third critique.** The task referenced three critiques
> (`convergence`, `evaluator-integration`, `excel-compat-perf`). Only two exist on disk:
> `ic_critique_convergence.md` and `ic_critique_excel-compat-perf.md`. The evaluator-integration
> concerns are in fact **covered** by `excel-compat-perf` findings A–J (Phase-1/Phase-2 inseparability,
> the override-seam-doesn't-exist gap, volatile-cache infra, bitcode forward-compat, delta-hook
> placement) — these are evaluator-integration issues. This document incorporates both existing
> critiques in full; no third file was synthesized into thin air.

---

## 0. Scope, headline decision, and the branch-reality correction

### 0.1 What this builds

Add a **workbook-level fixed-point iteration loop** so circular references converge to a stable value
(like Excel/LibreOffice under `<calcPr iterate="1"/>`) instead of returning `Error::CIRC`. This is an
**independently-upstreamable engine feature** AND the hard prerequisite for VEEV `Model_SENS`
data-table correctness — without it, 32 `#CIRC!` cells cascade into `G58→""`, `G61→#VALUE!`,
`P74→#VALUE!`, and all three Model_SENS data-table outputs are wrong.

**DECISION (the algorithm): whole-pass Gauss-Seidel/Jacobi-hybrid fixed-point iteration, seeded from
each cell's surviving `FormulaValue`** (IC4 option A). Wrap the Phase-2 cell sweep of `evaluate()` in
an outer loop of up to `iterateCount` passes; relax the `CellState::Evaluating → CIRC` guard
(`model.rs:1447`) *only when iteration is enabled* to return a seed (the cell's stored value, or 0 if
unevaluated); stop when `max|Δ| < iterateDelta` over scalar numeric formula cells, or at the
`iterateCount` cap (keep last values, **no error** — Excel semantics).

**Why not cycle-scoped / SCC / Tarjan (IC3 option C, IC4 option B):** IronCalc has **no dependency
graph and no readable recursion stack** — dependencies resolve by lazy recursion + memoization inside
`evaluate_cell` (verified `model.rs:1442`). `self.support` is an evaluation-side-effect reverse-dep map
that the data-table contract declares *untrusted under override* (IC1 §6, IC5 I6). LibreOffice's
elegance comes from `ScRecursionHelper` (`recursionhelper.cxx`), an explicit recursion-stack object
IronCalc lacks; building one — or a real SCC graph — is a large net-new subsystem and a philosophical
departure from "lazy recursion, no graph." Whole-pass needs **one guard branch + one outer loop + one
delta accumulator** and reuses the seed store that already exists. At VEEV scale (≈32 circular cells)
it is trivially fast. The converged *values* are identical for a contraction map regardless of scope
(IC2 §2.1); scope is a speed optimization, deferred (§7).

### 0.2 The branch-reality correction (the single most load-bearing fix, excel-compat A — [A])

**Verified on this worktree** (branch `claude/xenodochial-varahamihira-e55eb2`, based on `main`,
`git log`: `81386ad7`):

- There is **no** `evaluate_workbook_cells`, **no** `recompute_cells`, **no**
  `evaluate_with_data_tables`, **no** `compute_data_tables`, and **no** `base/src/data_table.rs`.
  `grep -rn "solve_governing\|data_table_overrides" base/src/` returns **nothing**.
- `Model::evaluate()` is a **single function** (`model.rs:3030`) containing Phase 1 (the spill
  restart loop) and Phase 2 (the `get_all_cells()` sweep) **inline**, ending with
  `evaluate_conditional_formatting()`. There is no separable "Phase-2 method" to wrap.

The companion data-tables design (`02-implementation-design.md`) and IC5 verified their line numbers
against the **`data_table_first_attempt`** branch, which *does* contain that substrate. **The draft
inherited those names as if they existed on `main`. They do not.** This has three consequences this
final design must honor:

1. **The iterative-calc engine feature (P0/P1 here) stands entirely on `main`** and must be written
   against `evaluate()` as it actually is — wrap the inline Phase-2 sweep, relax the guard at 1447.
   It has **zero dependency** on the data-table substrate.
2. **The data-table composition (§4) is a *contract against a future branch*, not against code that
   exists here.** It is correct as a contract (IC5 proved the seam sound) but the implementer must
   either (a) land iterative-calc on `main` first and let the data-tables branch rebase onto it, or
   (b) build both on the same integration branch. §4 states this explicitly and §13 sequences it.
3. **The draft's "wrap Phase 2 of `evaluate_workbook_cells`" must be re-pointed to "wrap the Phase-2
   `get_all_cells()` sweep inside `evaluate()` (`model.rs:3073-3082`)."** All edit-site line numbers
   in §3.5 are re-verified against this worktree's `main`.

### 0.3 Decoupling restated

Like the data-tables plan, we **decouple**: the iterative-calc engine feature ships and is
acceptance-gated on **synthetic circular fixtures** (the F9/G9 ramp, self-ref, oscillation). VEEV-
correct *data-table* values are the **joint milestone** (data-tables P3 = this doc's IC-P2). "All
`#CIRC!` when iteration is OFF" is the regression-lock, not a failure.

---

## 1. calcPr data model

**DECISION:** store the iteration-relevant settings on `Workbook.settings`
(`base/src/types.rs:106`, the existing `WorkbookSettings { tz, locale }`) as a nested
`CalcProperties` value. This is the workbook-singular settings home; it round-trips through bitcode
and is reachable from `Model` via `self.workbook.settings`.

```rust
// base/src/types.rs — new, near WorkbookSettings (~:105)
#[derive(Encode, Decode, Debug, PartialEq, Clone)]
pub struct CalcProperties {
    /// calcPr@iterate — master switch. Default false.
    #[serde(default)]
    pub iterate: bool,
    /// calcPr@iterateCount — max passes. Excel default 100.
    #[serde(default = "default_iterate_count")]
    pub iterate_count: u32,
    /// calcPr@iterateDelta — convergence threshold ("Maximum Change"). Excel default 0.001.
    #[serde(default = "default_iterate_delta")]
    pub iterate_delta: f64,
    /// calcPr@calcMode — auto | autoNoTable | manual. Round-tripped; orthogonal to iteration.
    #[serde(default)]
    pub calc_mode: CalcMode,
    /// calcPr@calcId — opaque engine build stamp (e.g. 191029). Preserve VERBATIM, never interpret.
    #[serde(default)]
    pub calc_id: Option<u32>,
    /// calcPr@fullCalcOnLoad — round-tripped for fidelity (excel-compat G1). Engine ignores it.
    #[serde(default)]
    pub full_calc_on_load: bool,
    /// calcPr@refMode — "A1" | "R1C1". Round-tripped for fidelity (excel-compat G2). Engine ignores it.
    #[serde(default)]
    pub ref_mode: Option<String>,
}
fn default_iterate_count() -> u32 { 100 }
fn default_iterate_delta() -> f64 { 0.001 }

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone, Default)]
pub enum CalcMode { #[default] Auto, AutoNoTable, Manual }

impl Default for CalcProperties {
    fn default() -> Self {
        CalcProperties {
            iterate: false, iterate_count: 100, iterate_delta: 0.001,
            calc_mode: CalcMode::Auto, calc_id: None,
            full_calc_on_load: false, ref_mode: None,
        }
    }
}
```

Then extend `WorkbookSettings`:

```rust
#[derive(Encode, Decode, Debug, PartialEq, Clone)]   // NOTE: Eq DROPPED (f64 in CalcProperties)
pub struct WorkbookSettings {
    pub tz: String,
    pub locale: String,
    #[serde(default)]
    pub calc_properties: CalcProperties,              // <- new trailing field
}
```

**Fidelity-field additions vs the draft (excel-compat G — [A]):** the draft stored only
`iterate/iterateCount/iterateDelta/calcMode/calcId` and *dropped* `fullCalcOnLoad`/`refMode`. Both are
now **round-tripped** (engine ignores them) so that a re-exported VEEV does not silently change reopen
behavior. If a workbook had `fullCalcOnLoad="1"`, dropping it would change whether Excel force-recalcs
on open. Storing it is cheap and removes a fidelity gap.

**`Eq` removal (convergence/excel-compat H — [A], with the grep promoted from assertion to task):**
`WorkbookSettings` currently derives `Eq` (verified `types.rs:105`). `iterate_delta: f64` is not `Eq`,
so we **drop `Eq`** (keep `PartialEq`). **This is a verification task, not an assertion:** run
`grep -rn "WorkbookSettings" base/ xlsx/ bindings/` and confirm no `Eq`-dependent use (HashMap key,
`assert_eq` on the struct in a context requiring `Eq`, `BTreeSet`, etc.). The draft asserted "none
expected"; the critique correctly demands the grep be run. (Expected outcome: safe — `WorkbookSettings`
is not a map key — but it must be confirmed, not assumed.)

**Bitcode forward-compat (excel-compat H — [A], escalated from "trivial" to "must-test"):** the draft
claimed appending a trailing field to `WorkbookSettings` is "bitcode positionally forward-safe" for old
`.ic` files. **`bitcode` is a tight positional codec with no documented protobuf-style trailing-field
guarantee.** `#[serde(default)]` helps **JSON only**, not the bitcode wire format. Decoding an old
buffer that lacks `calc_properties` may hit EOF. **Required test (new):** serialize a `WorkbookSettings`
with the *old* two-field shape (hand-built buffer or a pinned fixture from before the change), decode
with the *new* struct, and assert it either defaults gracefully or errors cleanly. **If bitcode cannot
decode the short buffer, a version tag / migration is required — a materially bigger task than "trailing
field," and it must be discovered in IC-P0, not in production.** This is an **open risk** (§14 Q6), not
a settled claim.

**Alternative considered & rejected:** a flat `Model.iterative: Option<IterativeOptions>` derived at
load (IC1 §7a). Rejected — settings must survive `.ic` round-trip and authoring edits; `Workbook` is the
durable home. `Model` gets *accessors* (§8), not storage.

---

## 2. Import / Export round-trip

### 2.1 Import (`xlsx/src/import/workbook.rs`)

`calcPr` is a child of `xl/workbook.xml`, **not read today** (silently dropped — verified). Add a parse
mapping attributes to `CalcProperties` with Excel defaults for omitted attributes:

```rust
// in the workbook.xml parse, after sheets/definedNames
let calc_properties = match root.descendants().find(|n| n.has_tag_name("calcPr")) {
    Some(n) => CalcProperties {
        iterate:       n.attribute("iterate").map_or(false, |v| v == "1" || v == "true"),
        iterate_count: n.attribute("iterateCount").and_then(|v| v.parse().ok()).unwrap_or(100),
        iterate_delta: n.attribute("iterateDelta").and_then(|v| v.parse().ok()).unwrap_or(0.001),
        calc_mode:     match n.attribute("calcMode") {
                           Some("manual") => CalcMode::Manual,
                           Some("autoNoTable") => CalcMode::AutoNoTable,
                           _ => CalcMode::Auto,
                       },
        calc_id:           n.attribute("calcId").and_then(|v| v.parse().ok()),
        full_calc_on_load: n.attribute("fullCalcOnLoad").map_or(false, |v| v == "1" || v == "true"),
        ref_mode:          n.attribute("refMode").map(str::to_string),
    },
    None => CalcProperties::default(),
};
```

Notes (IC2 §1): Excel omits attributes at their default, so VEEV's `<calcPr calcId="191029"
iterate="1"/>` ⇒ `iterate=true, iterate_count=100, iterate_delta=0.001`. **Preserve `calcId` verbatim**
— opaque, never bumped.

**`calcId` parse-failure hazard (excel-compat G4 — [A]):** `.parse::<u32>().ok()` silently maps a
malformed/overflowing `calcId` to `None`, which then **omits** it on export → triggers the very Excel
full-recalc-on-open we are trying to avoid (§14 Q4). ECMA `calcId` is `xsd:unsignedInt` (≤4294967295),
so `u32` *fits* a well-formed value; the residual risk is a malformed file. **DECISION:** accept the
risk for v1 (well-formed Excel files always fit `u32`) but **document it**, and log a warning on parse
failure rather than silently dropping. (Preserving the raw string would be the byte-faithful fix;
deferred — not worth a `String` field for an edge case no real Excel file hits.)

### 2.2 Export (`xlsx/src/export/workbook.rs:93`)

Replace the hardcoded `<calcPr/>` (verified at line 93) with a populated, attribute-omitting emitter:

```rust
fn calc_pr_xml(cp: &CalcProperties) -> String {
    let mut attrs = String::new();
    if let Some(id) = cp.calc_id { attrs.push_str(&format!(" calcId=\"{id}\"")); }
    match cp.calc_mode {
        CalcMode::Manual => attrs.push_str(" calcMode=\"manual\""),
        CalcMode::AutoNoTable => attrs.push_str(" calcMode=\"autoNoTable\""),
        CalcMode::Auto => {} // default, omit
    }
    if cp.full_calc_on_load { attrs.push_str(" fullCalcOnLoad=\"1\""); }
    if let Some(rm) = &cp.ref_mode { if rm != "A1" { attrs.push_str(&format!(" refMode=\"{rm}\"")); } }
    if cp.iterate { attrs.push_str(" iterate=\"1\""); }
    if cp.iterate_count != 100 { attrs.push_str(&format!(" iterateCount=\"{}\"", cp.iterate_count)); }
    if !near_default_delta(cp.iterate_delta) {
        attrs.push_str(&format!(" iterateDelta=\"{}\"", cp.iterate_delta));
    }
    format!("<calcPr{attrs}/>")
}

// excel-compat G3: f64::EPSILON (~2.2e-16) is FAR too tight — a parsed "0.001" can round to
// 0.001000000000001 and re-emit spuriously. Use a meaningful tolerance.
fn near_default_delta(d: f64) -> bool { (d - 0.001).abs() < 1e-9 }
```

**`iterateDelta` tolerance fix (excel-compat G3 — [A]):** the draft's `(d - 0.001).abs() > f64::EPSILON`
re-emits the attribute spuriously for any delta that round-trips through string parsing with a
sub-ULP error. Use `1e-9` (or, more conservatively, **track import-presence** and re-emit iff present
— deferred; the tolerance is sufficient for value-faithfulness).

**Round-trip bar (mirror data-tables R-OX3): value-faithful + stable-output, NOT byte-faithful.**
`iterate/iterateCount/iterateDelta/calcMode/calcId/fullCalcOnLoad/refMode` equal after re-import; exact
byte order of `<calcPr>`'s attributes is not contractual. VEEV's `<calcPr calcId="191029" iterate="1"/>`
re-emits substring-equal (modulo attribute order, which we do not contract).

---

## 3. THE ALGORITHM (core)

### 3.1 Two stores — the seed already exists (IC1 §3, IC4 §3)

Two separate stores, and their separation is what makes option A cheap:

1. `self.cells: HashMap<(u32,i32,i32), CellState>` (verified `model.rs:97` enum, two-variant
   `Evaluating | Evaluated`) — the **memo / re-entry guard only**, value-less. Cleared every pass.
2. The cell's value lives in **`sheet_data`** as a `FormulaValue` (types.rs), written by
   `set_cells_with_result` (`model.rs:891`) and read by `get_cell_value` (`model.rs:1256`). **Not**
   cleared by `self.cells.clear()`.

So the prior-pass value of every formula cell **survives a memo clear**. That stored `FormulaValue` IS
the per-cell iteration seed. No new value-snapshot store is needed for *seeding* (a separate snapshot
*is* needed for the basin-preserving first-pass seed and for delta tracking — see §3.4/§3.6, the
biggest correction from the convergence critique). `CellState` stays two-variant.

### 3.2 Relax the circular guard (model.rs:1447) — the one hot edit

Current guard (verified verbatim, `model.rs:1447-1453`, inside `match original_cell.get_formula()
=> Some(f) =>` after the memo lookup):

```rust
CellState::Evaluating => {
    return CalcResult::new_error(
        Error::CIRC, cell_reference, "Circular reference detected".to_string());
}
```

Relaxed:

```rust
CellState::Evaluating => {
    if self.iteration_enabled() {
        // Relaxation step: return the seed for this back-edge.
        return self.get_seed_value(&original_cell, cell_reference);
    }
    return CalcResult::new_error(
        Error::CIRC, cell_reference, "Circular reference detected".to_string());
}
```

**`get_seed_value` — corrected per convergence C12 + excel-compat J3 ([A]):** the draft said
"if stored `FormulaValue` is `Unevaluated` **or an `Error`**, return `Number(0.0)`." That is **too
broad and masks real errors.** `get_cell_value` maps `FormulaValue::Unevaluated` →
`Error::ERROR "Unevaluated formula"` (verified `model.rs:1278`). The guard must distinguish *that
specific* unevaluated-sentinel from a genuine `#DIV/0!`/`#REF!` that a precedent legitimately holds.

```rust
fn get_seed_value(&self, cell: &Cell, cell_ref: CellReferenceIndex) -> CalcResult {
    match cell {
        // Genuinely not-yet-evaluated this run → Excel seeds an unknown circular precedent as 0.
        Cell::CellFormula { v: FormulaValue::Unevaluated, .. }
        | Cell::ArrayFormula { v: FormulaValue::Unevaluated, .. } => CalcResult::Number(0.0),
        // Anything else (Number/Text/Boolean/Error) → return the prior-pass stored value verbatim.
        // A real #DIV/0! precedent must propagate, NOT be swallowed to 0 (convergence C12).
        _ => self.get_cell_value(cell, cell_ref),
    }
}
```

**Seed source caveat (convergence C1 — [A], honestly stated, not "fixed"):** `original_cell` is the
clone taken at the top of *this* `evaluate_cell` invocation (`model.rs:1444`,
`Some(c) => c.clone()`), captured **before** the formula re-evaluates this pass. For a back-edge into a
cell currently `Evaluating` (on the recursion stack), `get_seed_value` returns that cell's
**prior-pass** value (it has not yet written its new value — `set_cells_with_result` runs after the
recursive descent returns). For a back-edge into a cell already `Evaluated` earlier in the sweep, the
read is this-pass-fresh. **Therefore the scheme is Gauss-Seidel across the sweep, Jacobi across active
back-edges within one recursive descent — NOT pure Excel Gauss-Seidel.** See §3.7 for why this is
accepted and what it costs.

**Ordering constraint (IC5 I7, load-bearing for data tables):** when the override choke point exists
(future data-tables branch), the override check must be the **first** statement of `evaluate_cell`,
before the `SpillCell` branch and well before this `Evaluating` guard at 1447. The SpillCell branch is
already before the guard (verified `model.rs:1450` vs `:1447`... — SpillCell at ~1450 is *after* the
fetch but the guard is inside the `get_formula()=>Some` arm at 1447; **the spill branch precedes the
formula match, so an override placed before the spill branch precedes the guard by construction**). A
pinned input is thus a constant that never re-enters its own formula and is cut from the SCC.

### 3.3 The fixpoint loop — wrap the Phase-2 sweep inside `evaluate()` (model.rs:3073-3082)

**Re-pointed from the draft (excel-compat C + branch-reality §0.2 — [A]).** On `main`, `evaluate()` is
one function: Phase 1 (the `while retry` spill restart loop, `model.rs:3039-3070`) then Phase 2 (the
`get_all_cells()` sweep, `model.rs:3073-3082`) then CF. There is no `evaluate_workbook_cells` to wrap.
We wrap **only the Phase-2 sweep**, leaving Phase 1 and CF outside:

```rust
// model.rs, replacing the Phase-2 block at :3073-3082 inside evaluate()
// (Phase 1 spill restart loop above is UNCHANGED and runs once.)

let all_cells = self.get_all_cells();   // verified DETERMINISTIC: sorts rows then cols (model.rs:2906)

if !self.iteration_enabled() {
    // today's behavior: single Phase-2 sweep, CIRC guard intact
    for cell in &all_cells {
        self.evaluate_cell(CellReferenceIndex { sheet: cell.index, row: cell.row, column: cell.column });
    }
} else {
    let max_iter = self.iterate_count();   // calc_properties.iterate_count (default 100)
    let eps      = self.iterate_delta();   // calc_properties.iterate_delta (default 0.001)

    // (A) Basin-preserving first-pass seed snapshot (convergence C4 — see §3.6).
    let seed = self.snapshot_numeric_cone();          // immutable map read by the relaxed guard on pass 0

    let mut prev = self.snapshot_numeric_cone();      // (B) delta baseline (convergence C2 — see §3.4)
    for pass in 0..max_iter {
        // MEMO ONLY — FormulaValue seeds in sheet_data survive. Phase-1 spill marks are dropped;
        // spill anchors re-evaluate via lazy recursion each pass (see §6 spill row + excel-compat C).
        self.cells.clear();
        self.support.clear();
        self.clear_variable_stack();
        self.clear_lambdas();
        self.iteration_seed = if pass == 0 { Some(&seed) } else { None };   // pass-0 Jacobi seed
        for cell in &all_cells {
            self.evaluate_cell(CellReferenceIndex { sheet: cell.index, row: cell.row, column: cell.column });
        }
        // (C) delta computed in the LOOP against `prev`, restricted to scalar numeric cells (§3.4).
        let now = self.snapshot_numeric_cone();
        let max_delta = max_abs_change(&prev, &now);
        prev = now;
        // (D) i44115 guard: require >=2 full recomputes before declaring convergence (§3.5).
        if pass >= 1 && max_delta < eps { break; }
    }
    self.iteration_seed = None;
}
```

`snapshot_numeric_cone()` returns a `HashMap<(u32,i32,i32), f64>` of scalar-numeric formula-cell
values. For VEEV it is ≈ the whole formula set (small). For large books it can be restricted to a
detected cycle-cone in a later phase (§7); v1 snapshots all numeric formula cells (correct, just more
memory). This snapshot is **the corrected mechanism** that replaces the draft's two broken economies
(the in-write delta fold and the implicit cache seed) — see §3.4 and §3.6.

**Phase-1 / Phase-2 inseparability (excel-compat C — [A]):** because the per-pass `self.cells.clear()`
drops Phase-1 spill `Evaluated` marks, **spill anchors re-evaluate via lazy recursion every pass.** The
draft's "Phase 1 runs once" is therefore **false under iteration** and is corrected here: Phase 1's
*restart-reordering* runs once (before the loop), but spill *evaluation* recurs each pass. This is
functionally fine **only because spill geometry is frozen** (no `evaluate_cell` inside the loop may
change spill area — guaranteed by the §6 out-of-scope rule for spill-in/sized-by cycle). If a spill's
size depends on a circular cell, the frozen `all_cells` list desyncs — explicitly **out of scope**,
§6/C10.

### 3.4 Convergence test and delta tracking — moved OUT of `set_cells_with_result` (convergence C2 — [A])

**The draft's in-write delta fold is rejected. It does not work.** Verified at `set_cells_with_result`
(`model.rs:891`): the function early-returns for non-formula cells (`get_formula()==None => Ok(())`),
writes `CalcResult::Array` results via a **separate early path**, and **does not read the cell's prior
value as a number anywhere.** The draft's `(prior_numeric_value, new_numeric_value)` pair does not
exist; the `_ => ∞` catch-all would fire **every pass for every dynamic-array formula in the book**
(number-in-cell vs `Array`-result, or `Unevaluated`-clone vs value), forcing iteration to burn all
`iterateCount` passes and **never converge early** — a correctness-adjacent performance cliff that
contradicts §7's "<15 passes."

**DECISION:** compute the delta in the **fixpoint loop** by diffing two numeric snapshots
(`prev` vs `now` in §3.3), restricted to **scalar numeric formula cells**:

```rust
fn max_abs_change(prev: &HashMap<K, f64>, now: &HashMap<K, f64>) -> f64 {
    let mut m = 0.0_f64;
    for (k, &nv) in now {
        match prev.get(k) {
            Some(&ov) if nv.is_finite() && ov.is_finite() => m = m.max((nv - ov).abs()),
            // appeared this pass, or NaN/Inf this pass → not converged
            _ => return f64::INFINITY,
        }
    }
    m
}
```

Convergence semantics (IC2 §2.4, corrected per convergence C6/C11):
- **Metric:** `max |new − old|` over scalar numeric formula cells in the snapshot, **absolute**
  (Excel's "Maximum Change" is absolute). [IC2 §2.4]
- **Type-aware, not blanket-∞ (convergence C6 — [A]):** number-vs-number → `|Δ|`; **a stable error
  (error-vs-same-error) counts as 0 — a steady-state error IS converged**; a genuine type flip
  (number↔error, number↔string, number↔empty across passes) → not converged this pass. The draft's
  blanket `_ => ∞` wrongly treated number↔empty and the normal pass-0 `#DIV/0!→number` healing
  transient as permanent non-convergence. Implementation: the snapshot only holds *scalar numerics*;
  a cell absent from `now` but present in `prev` (went non-numeric) returns `∞` *for that pass only*,
  letting it heal next pass.
- **NaN/Inf** → non-converged (`∞`); rely on `iterateCount` to terminate a diverging cycle (Excel-
  faithful, no divergence detection). [IC2 §4]
- **Delta-set restriction is REQUIRED, not optional (convergence C11 — [A], escalated):** the draft
  framed "count all formula cells" as "rarely matters." It can matter badly: a single large-magnitude
  **acyclic** cell (e.g. a 1e9 revenue cell whose 4th significant digit genuinely wobbles by >0.001
  absolute, e.g. a volatile-ish or slowly-settling aggregate) holds iteration **hostage to the cap**
  even though the cycle converged. v1 snapshots all numeric formula cells for simplicity but **the
  design must restrict the delta set to the cycle-cone** as soon as cone detection exists (§7), and
  the acceptance tests (§9) include a large-acyclic-cell case to prove it does not falsely burn 100
  passes. **Interim mitigation for v1:** exclude any cell whose value was identical (bit-equal) in the
  previous pass from the *next* pass's delta (a cell that stopped moving cannot un-converge a
  contraction); cheap and prevents the settled-acyclic-cell trap without full cone detection.

### 3.5 Off-by-one and the i44115 premature-convergence guard (convergence C5 — [A])

The draft conflated two different "prior" notions (the per-call clone vs the prior **pass**) and its
`pass >= 1` gate was inconsistent with the (now-rejected) in-write fold. With delta computed in the
loop against an explicit `prev` snapshot (§3.4), the rule is clean:

- `prev` is initialized to the pre-loop snapshot (the imported cache / current values). So **pass 0's
  delta is meaningful** (pass-0 result vs the seed), unlike the draft.
- **i44115 guard (from IC3 / LibreOffice `formulacell.cxx`, bug i44115):** LO requires
  `nSeenInIteration > 1` — at least **two** full recomputes before a cell may be declared converged,
  to prevent a pass-0 fluke (the seed happening to reproduce within `eps`) from stopping a still-moving
  system. We honor it with `pass >= 1` **as the gate** (i.e. allow a stop only after pass index 1 = the
  second pass), but — crucially — pass 0's delta is still *computed and carried* so a genuinely
  cache-seeded already-converged workbook stops after exactly 2 passes (one to confirm), matching
  Excel's "converges in ~1 pass from cache" within one confirmation pass. The draft omitted i44115
  entirely; this is the correct, documented guard.
- **The cap counts passes, not comparisons** (IC2 §2.4): the loop runs `0..max_iter`; on exhaustion,
  retain last values, no `#CIRC!` stamp.

### 3.6 First-pass seed: basin-preserving snapshot (convergence C4 / C3 — [A])

**The draft's "seed from the imported `<v>` cache ⇒ converges in ~1 pass, avoids wrong basin" claim is
NOT delivered by the draft's mechanism** and is corrected here. Trace the draft: `set_cells_with_result`
**overwrites** each cell's `FormulaValue` on every pass. On pass 0, the sweep reaches non-cycle
precedents first and overwrites their imported cache with recomputed values; when it reaches the
*second-visited* cycle member, its partner has already been overwritten this pass with a
partially-seeded value. So "seed from cache" held only for the *first-touched* cycle member — the rest
were seeded from pass-0 transients. For a contraction this still converges, but it **defeats the stated
"~1 pass, exact basin" property** the design used to argue VEEV correctness and to argue against
honoring `calcChain.xml`. For a **non-linear** loop (VEEV's debt/cash/interest, rows ~31,38,55-56,
66-70), seed-from-0 "risks a *different* basin of attraction" (IC2 §2.3).

**DECISION:** add an **explicit immutable seed snapshot** read by the relaxed guard for the **whole of
pass 0** (true Jacobi seed):

- `snapshot_numeric_cone()` is captured **once before the loop** into `seed` (§3.3 step A). On pass 0,
  `self.iteration_seed = Some(&seed)`; `get_seed_value` consults `iteration_seed` **first** (returning
  the immutable pre-pass value for a back-edge), so **every** cycle member is seeded from the cache /
  pre-pass value, not from a pass-0 transient. From pass 1 on, `iteration_seed = None` and the guard
  reads the live (last-pass-written) `FormulaValue` as before — which is correct, because by then a
  full pass of fresh values exists.
- This makes the "~1 pass from cache" property **real**: an imported, already-converged VEEV with
  trusted `<v>` caches seeds all cycle members from the fixed point on pass 0, reproduces it, and the
  i44115 guard stops after 2 passes.
- For seed-from-0 (fresh formulas, no cache), `seed` simply holds 0 for unevaluated cells; the basin is
  whatever a 0-seed contraction reaches. **The non-linear-basin risk is carried to §14 Q1** and a
  **non-linear-loop basin test is added** (§9), not just the linear ramp.

`get_seed_value` revised to consult the immutable seed first:

```rust
fn get_seed_value(&self, cell: &Cell, cell_ref: CellReferenceIndex) -> CalcResult {
    let key = (cell_ref.sheet, cell_ref.row, cell_ref.column);
    if let Some(seed) = self.iteration_seed {            // pass 0 only
        if let Some(&v) = seed.get(&key) { return CalcResult::Number(v); }
    }
    match cell {
        Cell::CellFormula { v: FormulaValue::Unevaluated, .. }
        | Cell::ArrayFormula { v: FormulaValue::Unevaluated, .. } => CalcResult::Number(0.0),
        _ => self.get_cell_value(cell, cell_ref),
    }
}
```

### 3.7 Gauss-Seidel honesty (convergence C1 — [A], accepted-as-stated, not over-claimed)

The draft asserted "this is exactly what Excel does (Gauss-Seidel)" and used it to claim last-digit
Excel fidelity. **That over-claims.** As established in §3.2, back-edges into cells *currently on the
recursion stack* read prior-pass values (Jacobi-like), while back-edges into already-`Evaluated` cells
read this-pass values (Gauss-Seidel). The scheme is a **sweep-order-dependent hybrid.**

**DECISION:** accept the hybrid (true Gauss-Seidel on a cycle would require flattening the cycle into an
ordered list à la LO `aRecursionFormulas` / option C, which we defer), and **DROP the claim of
last-digit Excel reproduction.** The honest contract is:
- For a **linear** cycle (the F9/G9 ramp), the fixed point is **unique and order-independent** (IC2
  §2.1) — converged values match Excel **to within `iterateDelta`** regardless of hybrid vs pure
  Gauss-Seidel; only pass count differs. The ramp test asserts to-delta, not last-digit.
- For a **non-linear** cycle, the converged value **can** depend on within-pass order. We **validate
  numerically against VEEV's caches** rather than promise last-digit equality. If VEEV disagrees, the
  divergence is almost certainly within-pass order; honoring `xl/calcChain.xml` order becomes the next
  lever (§14 Q1) — **not built in v1, flagged.**

### 3.8 Edit-site summary (re-verified against THIS worktree's `main`)

| Site | Edit |
|---|---|
| `model.rs:1447` (`CellState::Evaluating` arm, inside `get_formula()=>Some`) | relax → `get_seed_value` when `iteration_enabled()` |
| `model.rs:3073-3082` (Phase-2 sweep inside `evaluate()`) | wrap in N-pass fixpoint loop with snapshot-based delta + i44115 guard |
| `model.rs` new fns | `snapshot_numeric_cone()`, `max_abs_change()`, `get_seed_value()` |
| `model.rs` (Model struct) | add `iteration_seed: Option<&HashMap<..,f64>>` (or an owned-with-lifetime-safe shape; see note), `iteration_enabled/iterate_count/iterate_delta` accessors, volatile-cache fields (§6) |
| `types.rs:105` | `CalcProperties` (+`full_calc_on_load`/`ref_mode`), `CalcMode`; extend `WorkbookSettings`; **drop `Eq`** |
| `xlsx/src/import/workbook.rs` | parse `<calcPr>` |
| `xlsx/src/export/workbook.rs:93` | emit populated `<calcPr>` (replace `<calcPr/>`) |
| `base/src/functions/date_and_time.rs:1135,1175` (`fn_today`/`fn_now`) | consult volatile snapshot (§6) |
| `base/src/functions/math_and_trigonometry/random.rs:20` (`fn_rand`) | volatile policy (§6) |

**Implementation note on `iteration_seed` lifetime:** a borrowed `Option<&HashMap>` field on `Model`
fights the borrow checker (the loop mutably borrows `self`). Implement as an owned
`Option<HashMap<(u32,i32,i32), f64>>` field set/cleared around the loop, or thread the seed as a
parameter through a private `evaluate_phase2(&mut self, seed: Option<&HashMap<..>>)` helper. The latter
is cleaner; either is acceptable. **Flagged so the implementer does not copy the pseudo-code's borrow
literally.**

`CellState` enum (`model.rs:97`) stays two-variant — no new value-carrying variant needed.

---

## 4. Composition with data tables (IC5 contract) — a contract against a FUTURE branch

**Branch-reality preface (excel-compat A — [A]).** Everything in §4 is a **contract**, not a patch
against this worktree. On `main` there is no `solve_governing`, no `data_table_overrides`, no
`recompute_cells`. The data-tables feature (companion doc) builds those on its own branch. This section
specifies **how iterative-calc must compose with that seam when both land on one integration branch**
(§13 sequences this). It does **not** describe code that exists here. The draft presented §4 as
"honoring an existing contract"; the honest framing is "this is the joint contract the two branches
must satisfy when merged."

`solve_governing` (data-tables §3.3) is the single seam. Under iteration it dispatches to
`solve_governing_iterative`:

```rust
fn solve_governing(&mut self, targets: &[CellReferenceIndex],
                   overrides: HashMap<(u32,i32,i32), CalcResult>) -> Vec<CalcResult> {
    self.data_table_overrides = Some(overrides);          // (1) pin inputs (read interception)
    let result = if self.iteration_enabled() {
        self.solve_governing_iterative(targets)           // P3 fixpoint over the cone
    } else {
        self.recompute_cells(targets)                     // P2 single demand-driven pass
    };
    self.data_table_overrides = None;                     // (5) unpin
    result
}

fn solve_governing_iterative(&mut self, targets: &[CellReferenceIndex]) -> Vec<CalcResult> {
    // overrides already Some; cone scope = the targets' transitive precedents (NOT whole-Phase-2).
    self.cells.clear(); self.support.clear();
    self.clear_variable_stack(); self.clear_lambdas();
    let max_iter = self.iterate_count();
    let eps = self.iterate_delta();
    let seed = self.snapshot_numeric_cone();              // warm-start source (see below)
    let mut prev = seed.clone();
    let mut result = vec![];
    for pass in 0..max_iter {
        self.cells.clear(); self.support.clear();
        self.clear_variable_stack(); self.clear_lambdas();
        self.iteration_seed = if pass == 0 { Some(seed.clone()) } else { None };
        result = targets.iter().map(|&t| self.evaluate_cell(t)).collect();  // demand-driven cone
        let now = self.snapshot_numeric_cone();
        let max_delta = max_abs_change(&prev, &now); prev = now;
        if pass >= 1 && max_delta < eps { break; }        // i44115 guard
    }
    self.iteration_seed = None;
    result   // last iterate on non-convergence (no #CIRC! — IC5 I8)
}
```

### 4.1 Scope reconciliation: initial solve is whole-Phase-2; data-table body solve is cone-scoped (excel-compat E — [A])

The draft used "whole-Phase-2" (§0/§3) and "demand-driven cone" (§4) **interchangeably** — they are
different scopes. **DECISION, made explicit:**
- **Initial workbook solve** (`evaluate()`): **whole-Phase-2 fixpoint** — re-evaluate `get_all_cells()`
  every pass. Simple, correct, O(all_formula_cells × passes). Viable for VEEV because VEEV is *small*,
  **not** because whole-Phase-2 is cheap (the honest statement excel-compat E demands).
- **Data-table body solve** (`solve_governing_iterative`): **cone-scoped fixpoint** —
  `targets.map(evaluate_cell)` pulls only the governing cone via lazy recursion. This is what keeps the
  25-substitutions × 3-tables multiplier tractable.

These are **two solvers** sharing the guard relaxation, the seed snapshot, and the delta machinery —
not one. §7's cost model is rewritten to reflect this.

### 4.2 Invariants honored (IC5 I1–I11)

- **Override held across ALL passes of one body-cell solve.** `data_table_overrides` is `Some` for the
  whole `solve_governing`, *outside* the iteration loop (I2). A pinned input that is itself a cycle
  member (VEEV F8/G10) becomes a boundary condition cut from the SCC (I3, §2.1).
- **Override check precedes the `Evaluating→seed` check** (I7, §3.2): a pinned input always wins.
- **Memo cleared BETWEEN body cells, never between iteration passes** (I4): the data-table loop owns the
  inter-substitution clear; the iteration loop owns the per-pass memo clear AND seed retention.
- **`support` under override never trusted** (I6) — cleared with the memo each pass.
- **Header values read post-convergence on the unswept true model**, before any override (I9): the
  data-table pass runs after `evaluate()`'s fixpoint converges (IC5 §2.3). A circular-cone header (P74,
  P87) is sampled at its fixed-point value.
- **Volatile freeze shared across the whole data-table pass** (§6): the snapshot taken at the top of the
  outermost recalc entry is the SAME `TODAY()` for the unswept solve AND all 25 substitutions (IC5
  §7.3), so headers sampled pre-sweep match body re-convergence.

### 4.3 Warm-start — corrected "cold" definition (convergence C9 — [A])

**The draft's v1 "cold" seed is mislabeled.** The draft said v1 ships "cold (the cell's current
`FormulaValue`, which after step (5) of a prior solve is whatever it last held)." **That is NOT cold —
it is accidental previous-substitution warm-start**, the very thing IC5 I11 says to gate behind a flag
(default OFF) because it introduces grid-traversal-order dependence. So the draft's two-run determinism
test could pass while per-substitution results silently depend on which body cell was solved first.

**DECISION:** define v1 "cold" precisely as **unswept-converged warm-start** (IC5 I11 option 1, the
correctness-safe one):
- The data-table pass snapshots the converged unswept cone **once** (after the initial workbook solve,
  before any override). Each substitution's pass-0 seed (`iteration_seed`) is **that immutable unswept
  snapshot** — identical for every body cell, independent of grid order. For a contraction map the
  fixed point is seed-independent (I11), so the converged answer is unchanged and the per-substitution
  result is **order-independent and deterministic**.
- **Previous-substitution warm-start** (seed substitution k from k−1's converged cone) is the
  optimization with ordering dependence — **gated behind a flag, default OFF in v1.**
- This makes the determinism test (§9) *actually* test seed-independence, not accidental order-stability.

---

## 5. Determinism & correctness

`get_all_cells` (verified `model.rs:2906`) sorts rows then columns via `sort_unstable` → it **is
deterministic** (sheet-index, row, col). **The draft's worry that it "may be HashMap-backed" is
unfounded for this path (convergence C3 / excel-compat F — [R, with reason]); drop the "verify it is
stable / sort keys" hedge** — it is already verified stable. Determinism is required for test stability
and reproducing Excel's cached values within `iterateDelta`.

- **Matching Excel's converged values:** for linear cycles, exact-to-delta regardless of order (§3.7);
  reproduce the **stop rule** (stop one confirmation pass after the delta band, i44115) not the analytic
  fixed point. **Seed from the imported `<v>` cache** when present via the explicit pass-0 seed snapshot
  (§3.6) — converges in ~1+1 passes, avoids a wrong basin for non-linear loops.
- **Run-to-run determinism is NOT byte-reproducibility across days (excel-compat F — [A]):** "two runs
  produce byte-identical values" holds **only within one recalc clock/seed** — `TODAY()` differs across
  days, so the §9 determinism test is worded "two runs **within the same frozen volatile snapshot**
  produce identical values." Do not conflate run-to-run determinism with Excel-cache reproduction.
- **Open risk (§14 Q1):** non-linear debt-loop order mismatch ⇒ `xl/calcChain.xml` ordering may become
  necessary. Not built in v1; flagged.

---

## 6. Edge cases

| Case | Decision |
|---|---|
| **Non-convergence (hit `iterateCount`)** | Keep last computed values, **NO error**. Match Excel (IC2 §4). Do NOT emit LibreOffice's `Err522`/`#CIRC!`. Optionally surface a non-fatal "did not converge" diagnostic. |
| **Divergence / overflow** | Values grow to `Inf`/`NaN`; delta tracked as `∞` so never converges; loop stops at cap with last values (possibly `#NUM!`). No special handling (Excel-faithful, IC2 §4). |
| **Oscillation (`A1=10−A1`)** | Never satisfies `|Δ|<eps`; runs to cap, stores last pass's side (parity-dependent). Acceptable (IC2 §4). |
| **Self-reference** | `A1=A1+1` diverges +1/pass → cap, last value. `A1=(A1+10)/2` → converges to 10 (IC2 §6). **`A1=A1` and `A1=A1*2` → "converge" to the seed (0)** after 2 passes — a degenerate seed-dependent fixed point Excel also lands on; **document and test** that the seed, not a true solve, determines the value (convergence C8 — [A]). |
| **Volatile `TODAY`/`NOW` across passes** | **DECISION (faithful freeze, IC2 §3):** snapshot `today/now` ONCE per outermost recalc, frozen across all passes AND all data-table substitutions. **Implementation is a real sub-task, not a one-liner (excel-compat D — [A]):** `fn_today`/`fn_now` (`date_and_time.rs:1135,1175`) call `excel_serial_for_now(tz)` **live** each invocation; `TODAY(tz)` takes a tz argument, so the cache must be **keyed by tz** (a `HashMap<String, i64>` on `Model`, populated lazily per distinct tz at recalc start), not a scalar. **Lifetime (convergence C7 — [A]):** set the snapshot at the top of the OUTERMOST recalc entry (`evaluate()` / future `evaluate_with_data_tables`), clear it at that entry's end — one snapshot shared by the workbook solve and all sub-solves; never reused across separate recalc events. |
| **Volatile `RAND`/`RANDBETWEEN`** | **Split out from TODAY/NOW (excel-compat D — [A]):** freezing `RAND` to force convergence is a **deliberate non-Excel deviation** (Excel re-rolls each pass → may never converge, IC2 §3). **DECISION:** leave `RAND` **live**; document that `RAND`+iteration is inherently non-convergent (runs to cap), matching Excel. No VEEV concern. Do NOT frame this as "Excel-faithful." |
| **`INDIRECT`/`OFFSET` inside a cycle (convergence C7 — [A], NEW)** | These are volatile **reference** functions that change dependency edges per pass; an `INDIRECT("A"&B1)` with `B1` in the cycle re-targets each pass, so cycle membership itself changes and the frozen sweep list + any convergence guarantee evaporate. **DECISION: a volatile *reference* inside a cycle is OUT OF SCOPE / non-convergent in v1** — document explicitly (the design claims to be upstreamable, so it must state this, not be silently wrong). VEEV does not use them in the cycle. |
| **Spill / dynamic arrays inside a cycle** | Out of scope for v1. Phase-1 *reordering* runs once; spill *evaluation* recurs each pass (§3.3). A cycle crossing a spill cone yields `#VALUE!` (mirrors data-tables). |
| **Spill SIZED BY a cycle (convergence C10 — [A], NEW)** | `=SEQUENCE(A1)` where A1 is circular: Phase 1 sizes the spill from A1's **pre-convergence seed**, then freezes it while A1 converges to a different value — the spill is the wrong size and never resized (order is inverted: spill is Phase 1, the cycle is Phase 2). **DECISION: any spill anchor whose size/shape transitively depends on a circular cell is UNSUPPORTED** — document as a known limitation; do not imply `#VALUE!` falls out automatically (it does not). Detect via a support-edge into a known cycle member if cone detection is added (§7). |
| **Conditional formatting timing** | CF runs as a **separate pass AFTER** the workbook solve (verified: `evaluate_conditional_formatting()` is the last line of `evaluate()`, `model.rs:3083`). Iteration does NOT touch CF; CF reads converged values once at the end. Must NOT be pulled inside the loop. (excel-compat J5 confirms this is correct as-is.) |
| **Cells entering/leaving a cycle across passes** | Handled naturally: the relaxed guard fires only on actual re-entry; a cell that stops being circular evaluates normally and contributes its `|Δ|`. No membership bookkeeping (a benefit of whole-Phase-2). |
| **Real error inside a cycle (convergence C12 / excel-compat J3 — [A])** | A genuine `#REF!`/`#DIV/0!` precedent must **propagate**, not be seeded to 0. `get_seed_value` returns 0 **only** for `FormulaValue::Unevaluated` (the sentinel `get_cell_value` maps to `Error::ERROR "Unevaluated formula"`); any other stored value (including a real error) is returned verbatim (§3.2/§3.6). Test: a `#DIV/0!`-in-cycle is not silently zeroed. |
| **Error masking via the seed (convergence C12 — [A])** | The relaxed guard fires only on **re-entry** (back-edge), never on forward eval, so genuine errors computed by forward evaluation still propagate normally; only the narrow back-edge-into-unevaluated case seeds 0. |

---

## 7. Performance

**Two solvers, two cost models (excel-compat E — [A], rewritten honestly):**

- **Initial workbook solve (whole-Phase-2):** cost = `passes × all_formula_cells`. Every pass
  re-evaluates the **entire** workbook, including the acyclic majority that converged after pass 1.
  Bounded by `iterateCount`. **Honest statement:** this is O(all_formula_cells × passes) on *every*
  solve; it is viable for VEEV because VEEV is small (≈32 circular cells in a small model), **NOT
  because whole-Phase-2 is cheap.** A real thousands-of-cells financial model pays thousands × ~15
  passes on the initial solve alone. The "acyclic majority is negligible" claim holds **only when the
  acyclic majority is small** — stated, not hand-waved.
- **Data-table body solve (cone-scoped):** cost = `Σ over substitutions of passes-to-converge × cone`.
  VEEV 2-D tables are 5×5 ⇒ 25 substitutions; worst case 25 × `iterateCount` × cone evaluations per
  table, ~7500 across 3 tables. In practice a well-conditioned averaging ramp converges in <15 passes
  (geometric contraction), so far below worst case (IC5 §2.4). **Cone-scoping (not whole-Phase-2) is
  what keeps this tractable** — §4.1.

- **Warm-start** (§4.3, unswept-converged seed) attacks the `passes` factor and is the single
  highest-leverage optimization for the data-table inner loop. **DECISION (excel-compat E — [A]): turn
  unswept-converged warm-start ON by default for the data-table inner loop** (it is correctness-safe,
  IC5 I11) — it is the difference between ~2 passes and 8–15 per substitution. Previous-substitution
  warm-start stays OFF (ordering dependence).
- **Guardrails:** `iterateCount` hard-caps total work; `iteration_enabled()` is `false`-fast on the
  non-iterating hot path (the delta snapshots and guard relaxation are entirely skipped when
  `iterate=false` — zero overhead).
- **Delta-set restriction** (§3.4/C11) prevents a large-magnitude acyclic cell from holding iteration
  to the cap.
- **Future (deferred, §7-defer):** **cycle-scoped iteration** (IC4 option C / IC3 LibreOffice-style)
  iterates only cycle members — design after option A ships, reusing the guard relaxation and
  convergence criteria; it also gives true Gauss-Seidel on a flattened cycle (§3.7) and enables the
  cone-restricted delta set (§3.4). **SCC/Tarjan (option B)** is deferred indefinitely (needs a real
  dependency graph). **Cone-amortization** for data tables (only re-evaluate cone cells that depend on
  the swept input) is orthogonal and lives in the data-tables perf phase.

---

## 8. API / bindings / UI

- **`Model` accessors** (read `self.workbook.settings.calc_properties`):
  `iteration_enabled() -> bool`, `iterate_count() -> u32`, `iterate_delta() -> f64`.
- **`UserModel` methods** (undoable, diff-emitting wrapper for wasm/python/web):
  `get_iterative_calculation() -> (bool, u32, f64)` and
  `set_iterative_calculation(enabled: bool, max_iterations: u32, max_change: f64)`. This is a
  workbook-settings edit; emit a **`Diff::SetCalcProperties` variant appended at the END of the `Diff`
  enum** (bitcode positional hazard — mirrors data-tables §1.2 `SetDataTable`). **Recalc-on-toggle
  contract (excel-compat J6 — [A]):** setting `enabled=true` triggers a **full recompute**; setting
  `enabled=false` likewise (cells that were converged must revert to `#CIRC!`); **undo/redo of the
  toggle must also recompute**, because the converged values are not themselves diffed (they are a
  recalc consequence, exactly like formula results). State this in the diff's apply/undo arms.
- **Bindings (`bindings/` wasm/python/nodejs):** expose `get/set_iterative_calculation` through
  `UserModel`. No auto-wiring beyond the existing recalc path.
- **Webapp calc-options UI:** a "Calculation options" panel (Excel: File ▸ Options ▸ Formulas ▸
  Calculation options) with **Enable iterative calculation** (checkbox), **Maximum iterations**
  (number, default 100, range 1–32767), **Maximum change** (number, default 0.001). On change, call
  `set_iterative_calculation` and recompute. Surface a non-fatal "did not converge" toast when a solve
  hits the cap (optional).
- **`support` post-solve (excel-compat J4 — [A]):** the iterative loop rebuilds and discards `support`
  each pass; the **final** pass leaves a valid `support` for the converged graph. **Verify nothing
  downstream relies on `support` reflecting a single-pass graph** (incremental recalc, dependency
  queries). Expected safe (each pass builds it fresh from the converged reads on the last pass), but
  confirm — flagged as a test (§9).

---

## 9. Test strategy

Engine tests (`base/src/test/`):
- **Convergent linear cycle, closed-form:** `A1=(A1+10)/2` ⇒ converges to ≈9.99939 within
  `iterateDelta=0.001` in ~14 passes (IC2 §6); assert within delta, NO `#CIRC!`, **assert pass count
  ≤ a small bound** (J1 — cap counts passes).
- **VEEV F9/G9 ramp (closed-form):** `E9=1.0, H9=2.0, F9=+AVERAGE(E9,G9), G9=+AVERAGE(F9,H9)` ⇒
  analytic `F9=(2E9+H9)/3=1.333…`, `G9=(E9+2H9)/3=1.666…` (verified by the convergence critique). Assert
  within `iterateDelta`; convergence in ≪100 passes.
- **Non-linear-loop basin test (convergence C3/C4 — NEW):** a small non-linear cycle (e.g. a
  net-interest/debt micro-loop with a division) where seed-from-0 and seed-from-cache reach the SAME
  fixed point; assert the explicit pass-0 seed snapshot (§3.6) makes a cache-seeded run converge in ≤2
  passes (i44115), and that a 0-seeded run also converges (possibly more passes). Proves the basin
  argument is delivered by the mechanism, not just claimed.
- **Cache-seed pass-count test (excel-compat J2 — NEW):** import a workbook whose `<v>` caches ARE the
  fixed point; recompute with iteration; **assert ≤2 passes** (the single most load-bearing VEEV
  behavior).
- **Self-ref / divergence:** `A1=A1+1` ⇒ stops at cap, NO `#CIRC!`, value finite (last iterate).
- **Degenerate seed fixed point (convergence C8 — NEW):** `A1=A1` ⇒ 0 (seed), not error; `A1=A1*2` ⇒ 0.
- **Oscillation:** `A1=10−A1` ⇒ stops at cap, no error, last side.
- **Stable-error steady state (convergence C6 — NEW):** a cycle that holds a stable `#DIV/0!` at the
  fixed point ⇒ converges (error-vs-same-error counts 0), does NOT burn 100 passes.
- **Real-error-in-cycle not zeroed (convergence C12 — NEW):** a `#DIV/0!` precedent inside a cycle
  propagates; assert it is not silently seeded to 0.
- **Large-acyclic-cell hostage (convergence C11 — NEW):** a 1e9-magnitude acyclic cell that wobbles
  >0.001 absolute must NOT force the converged cycle to burn 100 passes (proves the delta-set
  restriction / settled-cell exclusion).
- **Determinism:** two runs **within one frozen volatile snapshot** produce identical converged values
  (excel-compat F — reworded).
- **Volatile freeze:** `TODAY()` returns the same serial for every pass of one recalc; `TODAY(tz)` with
  two distinct tz args caches both (excel-compat D).
- **`support` post-solve sanity (excel-compat J4 — NEW):** after an iterative solve, a dependency query
  / incremental edit behaves correctly.
- **Bitcode forward-compat (excel-compat H — NEW, REQUIRED):** decode an old two-field
  `WorkbookSettings` buffer with the new struct; assert graceful default or clean error (gates whether a
  migration is needed — §1/§14 Q6).

Data-table composition tests (land with the data-tables branch, IC5 fixtures):
- **Fixture B — data table OVER a cycle:** F9/G9 ramp + a 1-var data table whose input is `E9` and
  governing reads `G9`. Sweep `E9 ∈ {0.5,1.0,1.5,2.0,2.5}`; assert each body = `E9 + 2(H9−E9)/3` within
  `iterateDelta`. Assert `E9` untouched, `data_table_overrides==None`, memo clean after. **Assert
  per-substitution results are independent of grid-traversal order** (proves the corrected "cold" =
  unswept-converged seed, convergence C9).
- **Fixture C — VEEV sharp test:** import `Model_SENS`, `evaluate_with_data_tables` with iteration on ⇒
  the 32 previously-`#CIRC!` cells hold finite values; `G58/G61/P74` finite; the 3 tables reproduce
  Excel's cached values within `iterateDelta` (`Q75=="$197 / 9%"` modulo TEXT rounding, `W75≈0.08088…`).
  Iteration-OFF ⇒ all-`#CIRC!` body (regression-lock the decoupling).

xlsx round-trip (`xlsx/tests/`):
- **calcPr round-trip:** import `<calcPr calcId="191029" iterate="1"/>`, assert `calc_properties` parsed
  (iterate=true, count=100, delta=0.001); export, assert `<calcPr … iterate="1"/>` substring present and
  re-import value-equal. Test explicit `iterateCount="50" iterateDelta="0.0001"`. **Test
  `fullCalcOnLoad`/`refMode` round-trip** (excel-compat G1/G2).

---

## 10. (reserved — area kept for the draft's 11-area structure; superseded by §12/§13/§14)

The draft's area 10 was "Phased rollout" and area 11 "Open questions." To keep the draft's 11-area
shape while adding the required **Resolved review issues** / **Recommended path** / **Open questions**
sections, the phased rollout is §13 (Recommended path) and open questions are §14. This heading is
retained intentionally; no content is lost.

---

## 11. (reserved — see §14 Open questions for the maintainer)

Retained heading for structural parity with the draft's 11 areas. Content lives in §14.

---

## 12. Resolved review issues

What changed because of the two critiques (and the worktree verification). **[A]=accepted,
[R]=rejected with reason.** The most load-bearing fixes are marked **(CRITICAL)**.

**Branch reality / evaluator integration (excel-compat A, C, I):**
- **R-IC-A [A] (CRITICAL):** This worktree is on `main`, NOT `data_table_first_attempt`. There is no
  `evaluate_workbook_cells`/`recompute_cells`/`solve_governing`/`data_table_overrides`/`data_table.rs`.
  Re-pointed the fixpoint wrap to the inline Phase-2 sweep in `evaluate()` (`model.rs:3073-3082`).
  Re-framed §4 as a **contract against a future merged branch**, not existing code. §0.2/§3.3/§4.
- **R-IC-C [A] (CRITICAL):** Phase 1 and Phase 2 are not separable methods on `main`; the per-pass
  `cells.clear()` drops spill marks so spill anchors **re-evaluate every pass**. "Phase 1 runs once" is
  false under iteration and is corrected (Phase-1 *reordering* once, spill *eval* per pass). §3.3/§6.
- **R-IC-I [A]:** IC-P1 cannot be cleanly decoupled from data tables on the integration branch (the
  restore-and-re-evaluate path would iterate). Phase plan reorders so the data-table compose decision
  precedes turning iteration on in that path. §13.

**Convergence / numerical correctness (convergence C1–C12):**
- **R-CV1 [A] (CRITICAL):** Delta tracking moved OUT of `set_cells_with_result` (which has no prior
  numeric value and would fire `∞` every pass for every dynamic-array formula, never converging early)
  into the fixpoint loop via two numeric snapshots, restricted to scalar numerics. §3.4. (convergence C2)
- **R-CV2 [A] (CRITICAL):** Explicit immutable pass-0 seed snapshot so ALL cycle members (not just the
  first-touched) seed from cache → real "~1 pass from cache" + correct basin. §3.6. (convergence C4)
- **R-CV3 [A]:** Gauss-Seidel over-claim dropped; scheme is a sweep-order-dependent Gauss-Seidel/Jacobi
  hybrid; last-digit Excel fidelity NOT promised — validate numerically. §3.7. (convergence C1)
- **R-CV4 [A]:** i44115 premature-convergence guard added (≥2 full recomputes before declaring
  converged); off-by-one reconciled with the in-loop delta. §3.5. (convergence C5)
- **R-CV5 [A]:** Type-aware delta — stable error counts 0, number↔empty/error transient does not
  permanently force `∞`; blanket `_ => ∞` rejected. §3.4. (convergence C6)
- **R-CV6 [A]:** `INDIRECT`/`OFFSET`-in-cycle scoped OUT (volatile references change edges per pass).
  §6. (convergence C7)
- **R-CV7 [A]:** Degenerate seed fixed points (`A1=A1`, `A1=A1*2` → seed) documented + tested. §6/§9.
  (convergence C8)
- **R-CV8 [A] (CRITICAL):** v1 "cold" redefined as **unswept-converged warm-start** (order-independent),
  not the accidental previous-substitution warm-start the draft described. §4.3. (convergence C9)
- **R-CV9 [A]:** Spill-sized-by-cycle declared unsupported (Phase-1/Phase-2 order inversion). §6.
  (convergence C10)
- **R-CV10 [A]:** Delta-set restriction promoted from optional to required (large acyclic cell can hold
  iteration hostage); interim settled-cell exclusion for v1. §3.4. (convergence C11)
- **R-CV11 [A]:** `get_seed_value` seeds 0 **only** for `Unevaluated`, propagating real errors (the
  draft's "Unevaluated OR Error ⇒ 0" masked `#REF!`/`#DIV/0!`). §3.2/§3.6. (convergence C12)

**Excel-compat / performance / scope (excel-compat B, D, E, F, G, H, J):**
- **R-EC1 [A]:** Dropped the `pass >= 1`-vs-`∞` redundancy; clean gate with explicit `prev`. §3.5.
  (excel-compat B)
- **R-EC2 [A] (CRITICAL):** Volatile freeze is a real sub-task — `today/now` cached **per-tz** on
  `Model`, set/cleared at the outermost recalc entry; `RAND` split out as a non-Excel deviation (left
  live). Edit-site rows added for `date_and_time.rs`/`random.rs`. §6/§3.8. (excel-compat D)
- **R-EC3 [A] (CRITICAL):** Two solvers distinguished — initial solve = whole-Phase-2; data-table body
  solve = cone-scoped; warm-start ON by default for the inner loop. §4.1/§7. (excel-compat E)
- **R-EC4 [A]:** Determinism claim reworded to "within one frozen volatile snapshot"; dropped the
  unfounded `get_all_cells` HashMap hedge (it IS deterministic). §5/§9. (excel-compat F)
- **R-EC5 [A]:** `fullCalcOnLoad`/`refMode` round-tripped; `iterateDelta` export tolerance `1e-9` not
  `f64::EPSILON`; `calcId` parse-failure documented. §1/§2. (excel-compat G)
- **R-EC6 [A] (CRITICAL):** Bitcode trailing-field forward-compat is UNVERIFIED — required test added;
  if it fails, a migration is needed (bigger than "trailing field"). `Eq`-removal grep promoted to a
  task. §1/§9/§14 Q6. (excel-compat H)
- **R-EC7 [A]:** Recalc-on-toggle/undo contract for `Diff::SetCalcProperties`; `support` post-solve
  sanity verified by test; cap-counts-passes assertion added. §8/§9. (excel-compat J)

**Rejected (with reason):**
- **R-EC-REJ1 [R]:** The draft's worry that `get_all_cells` "may be HashMap-backed" — **rejected**:
  verified it sorts rows then columns (`model.rs:2906`), so it is deterministic. Dropped the hedge.
- **R-CV-REJ1 [R]:** The draft's in-write delta fold "no whole-workbook snapshot buffer" economy —
  **rejected** as a false economy (it cannot read the prior numeric value and breaks on Array results);
  replaced with the in-loop snapshot diff. (This is a rejection of the *draft's* position in favor of
  the critic's, per convergence C2.)

**Nothing material from either critique was rejected outright.** The two "rejections" above are
rejections of the *draft's own* positions in favor of the critics'. Both critics were correct on the
load-bearing issues (delta location, basin seed, branch reality, volatile infra, Gauss-Seidel honesty,
bitcode forward-compat).

---

## 13. Recommended path (decision-ready, phased, mapped to data-tables P3)

**Ship on a fresh branch from `main`.** The iterative-calc engine feature (IC-P0/IC-P1) is independent
of the data-tables branch and upstreamable on its own. The composition (IC-P2) lands when both features
are on one integration branch (the data-tables branch rebases onto, or merges with, the iterative-calc
work — §0.2). Each phase ≈ one reviewable PR.

| Phase | Deliverable | Files | Acceptance criteria | Maps to |
|---|---|---|---|---|
| **IC-P0 — calcPr round-trip + bitcode-safety** | `CalcProperties`/`CalcMode` (+`full_calc_on_load`/`ref_mode`) on `WorkbookSettings`; **drop `Eq`** (after grep); `new_empty` defaults; import parse; export emit (replace `<calcPr/>`). **NO engine behavior change.** Plus the **bitcode forward-compat test** (gates whether a migration is needed). | `types.rs`, `new_empty.rs`, `xlsx/src/{import,export}/workbook.rs` | A workbook with `<calcPr iterate="1" .../>` imports to correct `calc_properties`; export substring-equal; re-import value-equal; `fullCalcOnLoad`/`refMode` round-trip; defaults (false/100/0.001) for omitted attrs; **old two-field `.ic` decodes gracefully (or a migration is scoped).** | **Independently upstreamable, low-risk.** |
| **IC-P1 — core fixpoint + relaxed guard** | `iteration_enabled/iterate_count/iterate_delta` accessors; relax `Evaluating→get_seed_value` (`model.rs:1447`); wrap Phase-2 sweep in N-pass loop (`model.rs:3073-3082`); snapshot-based delta + i44115 guard; explicit pass-0 seed snapshot; type-aware delta + delta-set restriction; **volatile freeze (per-tz `today/now` cache; `RAND` left live)**. | `model.rs`, `date_and_time.rs`, `random.rs` | Linear ramp + `A1=(A1+10)/2` converge to closed-form within `iterateDelta` in ≪100 (and ≤bound) passes; **cache-seeded workbook converges in ≤2 passes**; non-linear-basin test passes; self-ref/oscillation/divergence/stable-error/real-error-in-cycle behave per §6; large-acyclic-cell does not burn 100 passes; determinism (within one volatile snapshot); default (`iterate=false`) path byte-unchanged + zero-overhead. **Non-data-table cycles converge.** | Upstreamable engine feature. |
| **IC-P2 — compose with data tables (= data-tables P3)** | On the integration branch: `solve_governing_iterative` (cone-scoped); override held across passes; memo cleared between body cells only; headers read post-convergence; **unswept-converged warm-start ON by default**; shared volatile snapshot across the whole data-table pass. | `data_table.rs`, `model.rs` (integration branch) | Joint **Fixture B** (data table over the ramp; per-substitution results order-independent) passes; **VEEV Fixture C** — 32 `#CIRC!` resolve, 3 tables reproduce Excel caches within `iterateDelta`, `G58/G61/P74` finite; iteration-OFF ⇒ all-`#CIRC!` (regression-lock). **Unblocks VEEV.** **This IS data-tables P3.** | **Joint milestone with `02-implementation-design.md` §11 P3 / §13 P3.** |
| **IC-P3 — perf + UI** | `get/set_iterative_calculation` on `UserModel` + `Diff::SetCalcProperties` (appended, recalc-on-toggle/undo); bindings; webapp calc-options UI; (future) cycle-scoped iteration behind profiling; cone-restricted delta set. | `user_model/*`, `bindings/*`, `webapp/*`, `model.rs` | UI toggles iteration and recomputes (and undo recomputes); large-book guardrails verified; `support` post-solve sanity holds; (if pursued) cycle-scoped iteration matches whole-Phase-2 values on the VEEV fixture. | Upstream UI; perf. |

**Mapping to data-tables P3 (explicit):** `02-implementation-design.md` §11 P3 ("Iterative calc
(joint)") and §13/§14 P3 deliverable ("`calcPr` ingest/emit; `Evaluating→prev-value` under iteration;
fixpoint loop; `solve_governing_iterative`") is satisfied by **IC-P0 + IC-P1 + IC-P2** here. IC-P0
delivers `calcPr` ingest/emit (which the data-tables doc §4 explicitly assigns to iterative-calc, not
data-tables); IC-P1 delivers the guard relaxation + fixpoint loop; IC-P2 delivers
`solve_governing_iterative` and the joint VEEV acceptance. The data-tables plan's P0/P1/P2 (descriptor,
import/export, non-circular compute) are unblocked **without** any of this and ship first.

---

## 14. Open questions for the maintainer

1. **Non-linear cycle order-sensitivity (primary correctness risk).** VEEV's debt/cash/interest loop
   (rows ~31,38,55-56,66-70) may be mildly non-linear; the converged value can depend on within-pass
   order, and our scheme is a sweep-order-dependent Gauss-Seidel/Jacobi hybrid (§3.7), not pure
   Gauss-Seidel. Mitigation: deterministic order + explicit cache-seed snapshot. **If VEEV still
   disagrees with Excel after cache-seeding, importing/honoring `xl/calcChain.xml` order becomes
   necessary — NOT built in v1. Validate against VEEV's caches before locking IC-P2 acceptance.**
2. **Volatile pinning for VEEV + the live-recompute caveat.** `today/now` are frozen per recalc, cached
   per-tz (§6). A *live* recompute moves `TODAY()` with the calendar away from VEEV's saved cache.
   Confirm the accepted position is "trust imported caches for display; accept calendar drift on live
   recompute" (IC2 §3, IC5 §7.3). Confirm `RAND` left live (non-convergent with iteration) is acceptable
   rather than frozen.
3. **Whole-Phase-2 vs cycle-scoped for the initial solve.** v1 is whole-Phase-2 (simple, correct,
   O(all_formula_cells × passes)). Confirm this is acceptable for the initial workbook solve on
   realistically-sized books, or whether cycle-scoped iteration (the deferred perf path, §7) is needed
   before shipping. The data-table inner loop is already cone-scoped (§4.1).
4. **`calcId` reopen behavior.** If IronCalc writes a lower/zero/absent `calcId` than the consuming
   Excel, Excel may force a full recalc on open. We preserve the imported `calcId` verbatim except on
   parse failure (rare, documented §2.1). Verify by reopening an IronCalc-exported VEEV in Excel.
5. **Which cells count toward `max|Δ|`.** v1 counts all scalar-numeric formula cells with a settled-cell
   exclusion (§3.4); the cone-restricted set is deferred to IC-P3 (§7). Confirm acceptable, or require
   cone-restriction before IC-P2.
6. **Bitcode forward-compat (gating IC-P0).** Appending `calc_properties` to `WorkbookSettings` assumes
   `bitcode` can decode an old short buffer. **This is unverified** (§1, excel-compat H) — `bitcode` is a
   tight positional codec with no documented trailing-field guarantee. The IC-P0 test decides this; if
   it fails, IC-P0 must add a version tag / migration (materially larger). **Confirm the appetite for a
   migration if needed, or an alternative settings-versioning approach.**
7. **`Eq` removal blast radius.** Dropping `Eq` from `WorkbookSettings` (for `iterate_delta: f64`) —
   confirm the grep finds no `Eq`-dependent use (expected safe; must be run, not assumed).
8. **Composition seam ownership.** §4 is a contract against a future merged branch, since this worktree
   (`main`) has no data-table substrate. Confirm the intended integration: land iterative-calc on `main`
   first and rebase the data-tables branch onto it, or build both on one integration branch? IC-P2's
   acceptance (VEEV Fixture C) is only runnable once both are present.
