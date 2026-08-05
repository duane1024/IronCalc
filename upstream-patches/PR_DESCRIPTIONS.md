# PR descriptions for the three upstream fixes

---

## PR 1 — title: `fix: YEARFRAC swaps reversed dates before day-count adjustments`

### Problem

`YEARFRAC` computes the day count in argument order and takes the absolute
value of the result at the end. Excel instead **swaps start/end before**
applying the 30/360 day-of-month adjustments, and the two are not
equivalent because the adjustment rules are asymmetric: only the *start*
day collapses 31 → 30 unconditionally (the end day collapses only when the
start day is 30 or 31, per US 30/360 / NASD).

Concrete divergence (basis 0):

```
=YEARFRAC(DATE(2025,12,31), DATE(2026,7,10))   → 190/360 = 0.52777…  (correct)
=YEARFRAC(DATE(2026,7,10), DATE(2025,12,31))   → 189/360 = 0.525     (Excel: 190/360)
```

With dates in reverse order, Dec 31 is treated as the *end* date, so its
day-31 never collapses to 30 and the count comes out one day short. Any
formula like `=YEARFRAC(TODAY(), maturity_date)` silently drifts from Excel
once `TODAY()` passes the second argument. I hit this in a real financial
workbook where accrual factors stopped matching Excel mid-year.

The swap also protects basis 1 (actual/actual), whose year-range arithmetic
assumes ordered dates — reversed multi-year inputs could iterate over an
empty year range.

### Fix

Swap the two serial numbers immediately after parsing, when
`start > end` — before any basis-specific adjustment. This mirrors what
Excel does; the existing `abs()` at the end is now redundant for ordering
but harmless, so the change stays minimal.

### Testing

New regression test `test_yearfrac_basis_0_swaps_reversed_dates` in
`base/src/test/test_yearfrac_basis.rs` asserts both argument orders return
190/360 for the Dec 31 → Jul 10 pair. Existing YEARFRAC tests unchanged and
passing. Verified against Excel and LibreOffice.

---

## PR 2 — title: `fix: quote sheet names by allow-list, not block-list`

*(Companion to the import-side fix in <link to PR 3>; the two are
independent but fix the parse and stringify sides of the same underlying
assumption.)*

### Problem

When the engine re-stringifies a parsed formula (which happens for every
imported formula, since formulas are stored via parse → stringify),
`name_needs_quoting` decides whether a sheet name needs quotes using a
block-list of "bad" characters: space and `()'$,;-+{}`. But Excel forbids
only `\ / ? * [ ] :` in sheet names — everything else is legal, including
`!`, `&`, `=`, `%`, …

So a workbook with a sheet legally named `SUMM_P&L` or `!CAP` round-trips
its references *unquoted*:

- `='SUMM_P&L'!H28` re-stringifies as `=SUMM_P&L!H28` → `#NAME?`
- `='!CAP'!E5` re-stringifies as `=!CAP!E5` → unparseable → `#ERROR!`

This came from a real analyst workbook with both of those sheet names: 122
cells corrupted on the first recompute. It's invisible in viewer-style use
because the cached imported values still render — the damage only shows
once something triggers re-evaluation, which makes it nasty.

### Fix

Invert the logic to an **allow-list**: a name may skip quoting only if it
matches the unquoted-identifier grammar (letter or `_` first, then
letters/digits/`_`/`.`). Everything else gets quoted. Quoting is always
legal in a formula, so over-quoting is safe; under-quoting is what breaks.
The existing checks that force quoting for A1/R1C1-lookalike names (a sheet
named `A1`) are kept.

### Testing

- New `quote_name` unit cases: `!CAP`, `SUMM_P&L`, `A=B`, `P%L` (all now
  quoted) and `SUMM_P.L` (correctly stays unquoted).
- New end-to-end test `quoted_sheet_names_with_special_chars` in
  `base/src/test/test_general.rs`: creates sheets `!CAP` and `SUMM_P&L`,
  references them from formulas, evaluates, and asserts the values come
  through.
- All existing quoting tests pass unchanged (the allow-list is a strict
  superset of the old block-list's quoting behavior).

---

## PR 3 — title: `fix(xlsx): split cell references at the last '!', not the first`

*(Companion to the stringify-side fix in <link to PR 2>.)*

### Problem

The importer's `parse_reference` (used for shared-formula cell contexts in
`xlsx/src/import/worksheets.rs`) scans left-to-right and treats the *first*
`!` as the sheet/cell separator. Excel forbids only `\ / ? * [ ] :` in
sheet names, so `!` is legal — a sheet named `!CAP` (real name from a
production workbook) yields contexts like `!CAP!B7`, which the old
state-machine parses as sheet `""`, and then tries to read `CAP!B7` as a
cell reference, shredding the import.

The function even carried a `// FIXME: This is buggy` comment.

### Fix

Split at the **last** `!` via `rfind` (a cell reference itself can never
contain `!`, so the last one is unambiguous), then scan the remainder into
column letters and row digits as before. Sheets with no `!` behave as
before (empty sheet context). This also handles multiple `!`s:
`A!B!C!AS23` → sheet `A!B!C`, cell `AS23`.

### Testing

New unit test `parse_reference_sheet_name_containing_bang` covering
`!CAP!B7` and `A!B!C!AS23`. The existing test for a sheet name with
non-ASCII characters (`📈 Overview`) still passes.

Note: there is a similarly named reference parser in `ironcalc_base` (the
existing comment mentions the two should be fixed together). The base-side
equivalent of this problem is in the *stringifier* and is addressed in
<link to PR 2>; I kept this PR import-side-only to stay minimal, but happy
to consolidate if you'd prefer.
