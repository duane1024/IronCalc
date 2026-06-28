---
title: Iterative Calculation - Brainstorm Seed
type: feat
date: 2026-06-27
topic: iterative-calculation
kind: brainstorm-seed
status_note: NOT a Product Contract. This is a pre-brainstorm seed — run /ce-brainstorm against it in a fresh session to produce the requirements-only plan.
related_plan: docs/plans/2026-06-27-001-feat-data-tables-plan.md
---

# Iterative Calculation - Brainstorm Seed

> **What this is.** A grounding seed for a *separate* brainstorm/planning session on adding workbook-level **iterative calculation** to IronCalc. It is intentionally not a finished plan — it captures the verified problem, the facts a fresh session needs, and the open questions, so that session can start at Phase 2 instead of re-discovering everything. It was spun out of the data-tables brainstorm (`docs/plans/2026-06-27-001-feat-data-tables-plan.md`), which discovered the dependency.

## Why this exists

The VEEV template (`/Users/ddmoore/Downloads/VEEV_Template_v9.xlsx`, used in Baba) cannot compute in IronCalc — independent of data tables — because it relies on **iterative calculation**, which IronCalc does not support. Its 2029 EBITDA margin ramp is written as a mutually circular pair:

- `Model_SENS!F9 = +AVERAGE(E9,G9)`
- `Model_SENS!G9 = +AVERAGE(F9,H9)`

`F9` depends on `G9` and `G9` depends on `F9`. The workbook ships with `<calcPr calcId="191029" iterate="1"/>`, and Excel converges this cycle via iterative calculation; the stored values are the converged solution. Every VEEV data-table output (share price, IRR) sits downstream of this ramp, so without iterative calc those outputs are `#CIRC!` even with a perfect data-table engine.

This makes iterative calculation a **prerequisite** for the data-tables work to deliver end-to-end value on VEEV, but a fully independent engine feature worth contributing upstream on its own.

## Verified facts (with file:line)

- **No workbook-level iterative calc exists.** Circular references produce `#CIRC!` at `base/src/model.rs:1449` (circular detection via a `CellState::Evaluating` re-entry guard). The only iteration in the engine is *inside* individual financial functions (e.g. IRR/YIELD solvers in `base/src/functions/financial/financial_util.rs`), not a fixed-point recalc loop over the workbook.
- **`calcPr` is not read on import** and is written empty on export (`xlsx/src/export/workbook.rs:93` emits `<calcPr/>`). So `iterate`, `iterateCount`, and `iterateDelta` are silently dropped today.
- **Recalc has no dependency graph / topological sort.** `Model::evaluate()` (`base/src/model.rs:3030`) is a two-phase pass (spill anchors, then all cells, then conditional formatting); general dependencies resolve by lazy recursion in `evaluate_cell` (`base/src/model.rs:1412`). An iterative-calc design must decide how fixed-point iteration interacts with this recursive, cache-clearing evaluator and its `Evaluating`→`CIRC` guard.

## Relationship to the data-tables plan

- Data tables compute by **reference redirection** (see related plan). When a swept input feeds a circular subgraph, each redirected re-evaluation must itself converge — so iterative calc and data-table redirection must compose. Sequencing: iterative calc should land first or in parallel; the data-tables plan is validated without it on non-circular fixtures.
- **`calcPr` round-trip belongs to this plan**, not the data-tables plan (which owns only the per-cell `<f t="dataTable">` descriptor).

## Open questions for the brainstorm

- Convergence controls: honor Excel's `iterate`, `iterateCount` (default 100), `iterateDelta` (default 0.001); expose them through `UserModel`/bindings and the webapp calc-options UI.
- Algorithm: how to run fixed-point iteration over the existing recursive evaluator — relax the `Evaluating`→`CIRC` guard only when iteration is enabled, seed cells with prior/zero values, iterate to convergence or max count.
- Scope of iteration: whole workbook vs only the strongly-connected component(s) containing cycles; how to detect cycles without a dependency graph.
- Non-convergence behavior and interaction with volatile functions (e.g. VEEV's `TODAY()` in the IRR chain).
- Interaction with data-table redirection and with conditional formatting evaluation.
- `.xlsx` round-trip of `calcPr` (`iterate`/`iterateCount`/`iterateDelta`) on both import and export.

## Suggested kickoff for the new session

Run `/ce-brainstorm` (or `/ce-plan` if you want to go straight to implementation planning) with a prompt like: *"Add workbook-level iterative calculation to IronCalc so circular references converge instead of returning #CIRC!. Use this seed: docs/plans/2026-06-27-002-feat-iterative-calculation-seed.md. It's a prerequisite for the VEEV model and the data-tables plan."*

## Sources

- VEEV: `Model_SENS` circular ramp `F9`/`G9`; `xl/workbook.xml` `<calcPr ... iterate="1"/>`.
- Engine: `base/src/model.rs:1449` (`CIRC`), `:3030` (`evaluate`), `:1412` (`evaluate_cell`); `xlsx/src/export/workbook.rs:93` (empty `calcPr`).
- LibreOffice reference for circular/iterative handling lives in its Calc core (`sc/`) — worth a scan in the new session if a reference algorithm is wanted.
